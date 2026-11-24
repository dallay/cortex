//! Opt-in buffered fire-and-forget memory writer (`BOLT-08-04`, `SDD-2`).
//!
//! Mirrors `neo4j_agent_memory.memory.buffered.BufferedWriter` while preserving
//! hexagonal boundaries and two distinct write surfaces:
//!
//! 1. **Domain-validated buffered surface** (`submit_domain_write`) for typed
//!    application writes that already passed domain/scope checks.
//! 2. **Bolt-parity raw buffered surface** (`submit` / `submit_idempotent`),
//!    explicitly gated on write-enabled clients.
//!
//! Flush semantics use an **explicit ordered barrier message** in the FIFO
//! channel (`WriterCommand::Flush`), avoiding racy queue-length polling.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use muninn_core::config::WriteMode;
use muninn_core::ports::{BufferedWriteError, BufferedWriteJob, WriteExecutor};
use muninn_core::{MemoryError, MemoryResult};
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};
use tokio::task::JoinHandle;

/// Default bounded channel capacity (matches upstream `max_pending = 200`).
pub const DEFAULT_MAX_PENDING: usize = 200;

/// Default maximum retained background errors to bound memory usage.
pub const DEFAULT_MAX_RETAINED_ERRORS: usize = 1_000;

/// Default timeout bounding any user-supplied async error callback.
pub const DEFAULT_CALLBACK_TIMEOUT: Duration = Duration::from_secs(5);

/// Optional async callback invoked when a background buffered write fails.
pub type ErrorCallback =
    Arc<dyn Fn(BufferedWriteError) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// Configuration for [`BufferedWriter`].
#[derive(Clone)]
pub struct BufferedWriterConfig {
    pub write_mode: WriteMode,
    pub max_pending: usize,
    pub max_retained_errors: usize,
    pub callback_timeout: Duration,
    pub max_idempotent_retries: usize,
    pub on_error: Option<ErrorCallback>,
}

impl Default for BufferedWriterConfig {
    fn default() -> Self {
        Self {
            write_mode: WriteMode::Sync,
            max_pending: DEFAULT_MAX_PENDING,
            max_retained_errors: DEFAULT_MAX_RETAINED_ERRORS,
            callback_timeout: DEFAULT_CALLBACK_TIMEOUT,
            max_idempotent_retries: 1,
            on_error: None,
        }
    }
}

enum WriterCommand {
    Write(BufferedWriteJob),
    Flush(oneshot::Sender<()>),
    Shutdown,
}

struct ErrorLog {
    entries: VecDeque<BufferedWriteError>,
    max_entries: usize,
    dropped: usize,
}

impl ErrorLog {
    fn new(max_entries: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            max_entries: max_entries.max(1),
            dropped: 0,
        }
    }

    fn push(&mut self, error: BufferedWriteError) {
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.entries.push_back(error);
    }

    fn snapshot(&self) -> Vec<BufferedWriteError> {
        self.entries.iter().cloned().collect()
    }
}

struct WriterLifecycle {
    stopped: bool,
    sender: Option<mpsc::Sender<WriterCommand>>,
    drainer: Option<JoinHandle<()>>,
}

/// Bounded, barrier-coordinated buffered writer (`client.buffered`).
pub struct BufferedWriter {
    executor: Arc<dyn WriteExecutor>,
    config: BufferedWriterConfig,
    lifecycle: AsyncMutex<WriterLifecycle>,
    pending: Arc<AtomicUsize>,
    errors: Arc<Mutex<ErrorLog>>,
}

impl BufferedWriter {
    pub fn new(
        executor: Arc<dyn WriteExecutor>,
        config: BufferedWriterConfig,
    ) -> MemoryResult<Self> {
        if config.max_pending == 0 {
            return Err(MemoryError::invalid("max_pending must be at least 1"));
        }
        if config.max_retained_errors == 0 {
            return Err(MemoryError::invalid(
                "max_retained_errors must be at least 1",
            ));
        }
        Ok(Self {
            executor,
            errors: Arc::new(Mutex::new(ErrorLog::new(config.max_retained_errors))),
            config,
            lifecycle: AsyncMutex::new(WriterLifecycle {
                stopped: false,
                sender: None,
                drainer: None,
            }),
            pending: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Indicates configured write mode (`client.buffered.is_buffered`).
    pub const fn is_buffered(&self) -> bool {
        matches!(self.config.write_mode, WriteMode::Buffered)
    }

    /// Number of writes currently queued waiting for the drainer (`client.buffered.pending`).
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }

    /// Snapshot of background write errors recorded since startup, newest last (`client.buffered.errors`).
    pub fn errors(&self) -> Vec<BufferedWriteError> {
        self.errors
            .lock()
            .expect("buffered error log lock")
            .snapshot()
    }

    /// Number of oldest errors evicted when the bounded error ring reached capacity.
    pub fn dropped_error_count(&self) -> usize {
        self.errors.lock().expect("buffered error log lock").dropped
    }

    /// Submit a Bolt-parity raw write (`client.buffered.submit`).
    ///
    /// In `Sync` mode, executes inline and propagates errors immediately.
    /// In `Buffered` mode, enqueues with bounded backpressure and returns once
    /// accepted into the queue (acceptance does not guarantee persistence).
    pub async fn submit(
        &self,
        query: impl Into<String>,
        params: Map<String, Value>,
    ) -> MemoryResult<()> {
        self.submit_domain_write(BufferedWriteJob::new(query, params))
            .await
    }

    /// Submit an explicitly idempotent write eligible for bounded retry on transient error.
    pub async fn submit_idempotent(
        &self,
        query: impl Into<String>,
        params: Map<String, Value>,
    ) -> MemoryResult<()> {
        self.submit_domain_write(BufferedWriteJob::idempotent(query, params))
            .await
    }

    /// Typed domain-facing buffered write surface.
    pub async fn submit_domain_write(&self, job: BufferedWriteJob) -> MemoryResult<()> {
        if job.query.trim().is_empty() {
            return Err(MemoryError::invalid(
                "buffered write query must not be blank",
            ));
        }

        let sender = {
            let mut state = self.lifecycle.lock().await;
            if state.stopped {
                return Err(MemoryError::invalid(
                    "BufferedWriter has been stopped (client closed)",
                ));
            }
            if !self.is_buffered() {
                None
            } else {
                Some(self.ensure_drainer_locked(&mut state))
            }
        };

        let Some(sender) = sender else {
            return self.executor.execute_write(&job.query, &job.params).await;
        };

        // Acquire bounded channel permit (backpressure when full), then record acceptance.
        let permit = sender
            .reserve()
            .await
            .map_err(|_| MemoryError::invalid("BufferedWriter has been stopped (client closed)"))?;
        self.pending.fetch_add(1, Ordering::SeqCst);
        permit.send(WriterCommand::Write(job));
        Ok(())
    }

    /// Wait until every write accepted *before* this barrier completes (`client.buffered.flush`).
    pub async fn flush(&self) -> MemoryResult<()> {
        if !self.is_buffered() {
            return Ok(());
        }
        let sender = {
            let state = self.lifecycle.lock().await;
            state.sender.clone()
        };
        let Some(sender) = sender else {
            return Ok(());
        };
        let (tx, rx) = oneshot::channel();
        if sender.send(WriterCommand::Flush(tx)).await.is_err() {
            return Ok(());
        }
        let _ = rx.await;
        Ok(())
    }

    /// Alias of [`Self::flush`] (`client.buffered.wait_for_pending`).
    pub async fn wait_for_pending(&self) -> MemoryResult<()> {
        self.flush().await
    }

    /// Reject further submissions, drain all accepted writes, and stop the background drainer (`client.buffered.stop`).
    pub async fn stop(&self) -> MemoryResult<()> {
        let (sender, drainer) = {
            let mut state = self.lifecycle.lock().await;
            if state.stopped {
                return Ok(());
            }
            state.stopped = true;
            (state.sender.take(), state.drainer.take())
        };
        if let Some(sender) = sender {
            let _ = sender.send(WriterCommand::Shutdown).await;
        }
        if let Some(handle) = drainer {
            let _ = handle.await;
        }
        Ok(())
    }

    fn ensure_drainer_locked(&self, state: &mut WriterLifecycle) -> mpsc::Sender<WriterCommand> {
        if let Some(sender) = &state.sender {
            return sender.clone();
        }
        let (tx, rx) = mpsc::channel(self.config.max_pending);
        let handle = tokio::spawn(drain_loop(
            rx,
            Arc::clone(&self.executor),
            Arc::clone(&self.pending),
            Arc::clone(&self.errors),
            self.config.max_idempotent_retries,
            self.config.callback_timeout,
            self.config.on_error.clone(),
        ));
        state.sender = Some(tx.clone());
        state.drainer = Some(handle);
        tx
    }
}

async fn drain_loop(
    mut rx: mpsc::Receiver<WriterCommand>,
    executor: Arc<dyn WriteExecutor>,
    pending: Arc<AtomicUsize>,
    errors: Arc<Mutex<ErrorLog>>,
    max_idempotent_retries: usize,
    callback_timeout: Duration,
    on_error: Option<ErrorCallback>,
) {
    while let Some(command) = rx.recv().await {
        match command {
            WriterCommand::Write(job) => {
                pending.fetch_sub(1, Ordering::SeqCst);
                let attempts = if job.idempotent {
                    1 + max_idempotent_retries
                } else {
                    1
                };
                let mut last_err = None;
                for _ in 0..attempts {
                    match executor.execute_write(&job.query, &job.params).await {
                        Ok(()) => {
                            last_err = None;
                            break;
                        }
                        Err(err) => {
                            last_err = Some(err);
                        }
                    }
                }
                if let Some(err) = last_err {
                    let record = BufferedWriteError {
                        query: job.query,
                        parameter_keys: job.params.keys().cloned().collect(),
                        error: err.to_string(),
                        when: Utc::now(),
                    };
                    errors
                        .lock()
                        .expect("buffered error log lock")
                        .push(record.clone());
                    if let Some(callback) = &on_error {
                        // Isolate and bound callback execution so a slow hook never stalls draining.
                        let _ = tokio::time::timeout(callback_timeout, callback(record)).await;
                    }
                }
            }
            WriterCommand::Flush(done) => {
                let _ = done.send(());
            }
            WriterCommand::Shutdown => {
                // FIFO ordering guarantees all previously accepted Write commands
                // have already been processed before Shutdown is dequeued.
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use tokio::sync::Notify;

    struct FakeExecutor {
        writes: Mutex<Vec<String>>,
        fail_prefix: Option<String>,
        fail_first_n: AtomicUsize,
        gate: Option<Arc<Notify>>,
    }

    impl FakeExecutor {
        fn ok() -> Arc<Self> {
            Arc::new(Self {
                writes: Mutex::new(Vec::new()),
                fail_prefix: None,
                fail_first_n: AtomicUsize::new(0),
                gate: None,
            })
        }

        fn failing_on(prefix: &str) -> Arc<Self> {
            Arc::new(Self {
                writes: Mutex::new(Vec::new()),
                fail_prefix: Some(prefix.to_owned()),
                fail_first_n: AtomicUsize::new(0),
                gate: None,
            })
        }

        fn failing_first(n: usize) -> Arc<Self> {
            Arc::new(Self {
                writes: Mutex::new(Vec::new()),
                fail_prefix: None,
                fail_first_n: AtomicUsize::new(n),
                gate: None,
            })
        }

        fn gated(gate: Arc<Notify>) -> Arc<Self> {
            Arc::new(Self {
                writes: Mutex::new(Vec::new()),
                fail_prefix: None,
                fail_first_n: AtomicUsize::new(0),
                gate: Some(gate),
            })
        }

        fn recorded(&self) -> Vec<String> {
            self.writes.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl WriteExecutor for FakeExecutor {
        async fn execute_write(
            &self,
            query: &str,
            _params: &Map<String, Value>,
        ) -> MemoryResult<()> {
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            if let Some(prefix) = &self.fail_prefix {
                if query.starts_with(prefix) {
                    return Err(MemoryError::storage("injected write failure"));
                }
            }
            let remaining = self.fail_first_n.load(Ordering::SeqCst);
            if remaining > 0
                && self
                    .fail_first_n
                    .compare_exchange(remaining, remaining - 1, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            {
                return Err(MemoryError::storage("transient failure"));
            }
            self.writes.lock().unwrap().push(query.to_owned());
            Ok(())
        }
    }

    #[tokio::test]
    async fn gm09_b01_sync_mode_executes_inline_and_propagates_errors() {
        let exec = FakeExecutor::failing_on("BAD");
        let writer = BufferedWriter::new(exec.clone(), BufferedWriterConfig::default()).unwrap();
        assert!(!writer.is_buffered());
        assert_eq!(writer.pending(), 0);

        writer.submit("CREATE (:Ok)", Map::new()).await.unwrap();
        assert_eq!(exec.recorded(), vec!["CREATE (:Ok)"]);

        let err = writer.submit("BAD QUERY", Map::new()).await.unwrap_err();
        assert!(matches!(err, MemoryError::Storage(_)));
        assert!(writer.errors().is_empty());
        writer.flush().await.unwrap();
        writer.stop().await.unwrap();
    }

    #[tokio::test]
    async fn gm09_b02_and_b04_buffered_mode_enqueues_and_flushes_via_ordered_barrier() {
        let exec = FakeExecutor::ok();
        let writer = BufferedWriter::new(
            exec.clone(),
            BufferedWriterConfig {
                write_mode: WriteMode::Buffered,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(writer.is_buffered());

        for i in 0..10 {
            writer
                .submit(format!("CREATE (:N {{i: {i}}})"), Map::new())
                .await
                .unwrap();
        }
        writer.wait_for_pending().await.unwrap();
        assert_eq!(writer.pending(), 0);
        assert_eq!(exec.recorded().len(), 10);
        writer.stop().await.unwrap();
    }

    #[tokio::test]
    async fn gm09_b03_bounded_queue_applies_backpressure() {
        let gate = Arc::new(Notify::new());
        let exec = FakeExecutor::gated(gate.clone());
        let writer = Arc::new(
            BufferedWriter::new(
                exec.clone(),
                BufferedWriterConfig {
                    write_mode: WriteMode::Buffered,
                    max_pending: 1,
                    ..Default::default()
                },
            )
            .unwrap(),
        );

        // First item is dequeued by drainer and blocks on `gate`; second fills capacity 1.
        writer.submit("Q1", Map::new()).await.unwrap();
        tokio::task::yield_now().await;
        writer.submit("Q2", Map::new()).await.unwrap();

        let writer_clone = Arc::clone(&writer);
        let third = tokio::spawn(async move { writer_clone.submit("Q3", Map::new()).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !third.is_finished(),
            "third submit must block under backpressure when queue is full"
        );

        // Release all 3 gated writes.
        for _ in 0..3 {
            gate.notify_one();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        third.await.unwrap().unwrap();
        writer.stop().await.unwrap();
        assert_eq!(exec.recorded(), vec!["Q1", "Q2", "Q3"]);
    }

    #[tokio::test]
    async fn gm09_b05_background_errors_captured_with_bounded_callback_without_poisoning() {
        let exec = FakeExecutor::failing_on("FAIL");
        let callback_hits = Arc::new(AtomicUsize::new(0));
        let hits = Arc::clone(&callback_hits);
        let on_error: ErrorCallback = Arc::new(move |_err| {
            let hits = Arc::clone(&hits);
            Box::pin(async move {
                hits.fetch_add(1, Ordering::SeqCst);
                // Exceed callback_timeout to prove drainer is not stalled indefinitely.
                tokio::time::sleep(Duration::from_millis(200)).await;
            })
        });

        let writer = BufferedWriter::new(
            exec.clone(),
            BufferedWriterConfig {
                write_mode: WriteMode::Buffered,
                max_retained_errors: 2,
                callback_timeout: Duration::from_millis(15),
                on_error: Some(on_error),
                ..Default::default()
            },
        )
        .unwrap();

        writer.submit("FAIL-1", Map::new()).await.unwrap();
        writer.submit("OK-1", Map::new()).await.unwrap();
        writer.submit("FAIL-2", Map::new()).await.unwrap();
        writer.submit("FAIL-3", Map::new()).await.unwrap();
        writer.submit("OK-2", Map::new()).await.unwrap();

        writer.flush().await.unwrap();
        assert_eq!(exec.recorded(), vec!["OK-1", "OK-2"]);
        assert_eq!(writer.errors().len(), 2);
        assert_eq!(writer.errors()[0].query, "FAIL-2");
        assert_eq!(writer.errors()[1].query, "FAIL-3");
        assert_eq!(writer.dropped_error_count(), 1);
        assert_eq!(callback_hits.load(Ordering::SeqCst), 3);
        writer.stop().await.unwrap();
    }

    #[tokio::test]
    async fn gm09_b06_and_b07_graceful_stop_drains_and_rejects_subsequent_submits() {
        let exec = FakeExecutor::ok();
        let writer = Arc::new(
            BufferedWriter::new(
                exec.clone(),
                BufferedWriterConfig {
                    write_mode: WriteMode::Buffered,
                    max_pending: 32,
                    ..Default::default()
                },
            )
            .unwrap(),
        );

        let mut handles = Vec::new();
        for i in 0..20 {
            let w = Arc::clone(&writer);
            handles.push(tokio::spawn(async move {
                w.submit(format!("W-{i}"), Map::new()).await
            }));
        }
        let mut accepted = 0usize;
        for handle in handles {
            if handle.await.unwrap().is_ok() {
                accepted += 1;
            }
        }
        writer.stop().await.unwrap();
        // Idempotent second stop.
        writer.stop().await.unwrap();
        assert_eq!(exec.recorded().len(), accepted);

        let err = writer.submit("AFTER-STOP", Map::new()).await.unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn gm09_b08_retries_only_explicitly_idempotent_writes() {
        // Non-idempotent write fails once and is NOT retried.
        let exec_non_idem = FakeExecutor::failing_first(1);
        let writer_non_idem = BufferedWriter::new(
            exec_non_idem.clone(),
            BufferedWriterConfig {
                write_mode: WriteMode::Buffered,
                max_idempotent_retries: 2,
                ..Default::default()
            },
        )
        .unwrap();
        writer_non_idem
            .submit("CREATE (:Once)", Map::new())
            .await
            .unwrap();
        writer_non_idem.flush().await.unwrap();
        assert!(exec_non_idem.recorded().is_empty());
        assert_eq!(writer_non_idem.errors().len(), 1);
        writer_non_idem.stop().await.unwrap();

        // Explicitly idempotent write retries transient error and succeeds.
        let exec_idem = FakeExecutor::failing_first(1);
        let writer_idem = BufferedWriter::new(
            exec_idem.clone(),
            BufferedWriterConfig {
                write_mode: WriteMode::Buffered,
                max_idempotent_retries: 2,
                ..Default::default()
            },
        )
        .unwrap();
        writer_idem
            .submit_idempotent("MERGE (:Idem {id: 1})", Map::new())
            .await
            .unwrap();
        writer_idem.flush().await.unwrap();
        assert_eq!(exec_idem.recorded(), vec!["MERGE (:Idem {id: 1})"]);
        assert!(writer_idem.errors().is_empty());
        writer_idem.stop().await.unwrap();
    }
}
