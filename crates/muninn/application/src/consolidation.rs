//! Memory consolidation primitives — dry-runnable hygiene jobs.
//!
//! Equivalent to `neo4j_agent_memory.memory.consolidation`.
//!
//! Four primitives, all idempotent and dry-runnable:
//!
//! * `dedupe_entities` — find pairs of entities with high embedding
//!   similarity and either flag them with `:SAME_AS` or report them.
//! * `summarize_long_traces` — find traces with many steps that
//!   haven't been summarized yet.
//! * `detect_superseded_preferences` / `detect_superseded_preferences_for_owner` —
//!   find preferences with near-duplicate content suggesting one supersedes the other.
//! * `archive_expired_conversations` — archive old conversations based on an explicit TTL.
//!
//! Single transactional audit ownership (`SDD-4`, `F-05`):
//! * The store atomically writes `(:ConsolidationRun)` in the same transaction
//!   when and only when `!dry_run && mutations_applied > 0`.
//! * This orchestrator validates `report.validate_invariants()?` and never
//!   attempts a secondary repair audit write outside the store transaction.

use std::sync::Arc;

use muninn_core::ports::{
    ConsolidationReport, ConsolidationStore, LongTermStore, ReadAuditRecord, ReasoningStore,
    ShortTermStore,
};
use muninn_core::{MemoryError, MemoryResult};

/// Default similarity threshold for entity deduplication (upstream `0.95`).
pub const DEFAULT_DEDUPE_THRESHOLD: f64 = 0.95;

/// Default maximum candidate pairs for entity deduplication (upstream `10_000`).
pub const DEFAULT_DEDUPE_LIMIT: i64 = 10_000;

/// Default similarity threshold for preference supersession (upstream `0.92`).
pub const DEFAULT_SUPERSEDE_THRESHOLD: f64 = 0.92;

/// Minimum steps to consider a trace "long" (upstream `20`).
pub const DEFAULT_MIN_TRACE_STEPS: i64 = 20;

/// Default maximum traces for summarization flagging (upstream `1_000`).
pub const DEFAULT_TRACE_LIMIT: i64 = 1_000;

/// Legacy fallback constant retained for callers constructing explicit TTLs.
pub const DEFAULT_CONVERSATION_TTL_DAYS: i64 = 90;

/// Default limit for consolidation candidates where upstream has no dedicated cap.
pub const DEFAULT_CONSOLIDATION_LIMIT: i64 = 100;

/// High-level consolidation orchestrator.
///
/// Equivalent to `neo4j_agent_memory.memory.consolidation.ConsolidationMemory`.
pub struct ConsolidationMemory {
    #[allow(dead_code)]
    long_term: Option<Box<dyn LongTermStore>>,
    #[allow(dead_code)]
    reasoning: Option<Box<dyn ReasoningStore>>,
    #[allow(dead_code)]
    short_term: Option<Box<dyn ShortTermStore>>,
    consolidation: Arc<dyn ConsolidationStore>,
}

impl ConsolidationMemory {
    /// Create a new consolidation orchestrator.
    pub fn new(
        long_term: Box<dyn LongTermStore>,
        reasoning: Box<dyn ReasoningStore>,
        short_term: Box<dyn ShortTermStore>,
        consolidation: Box<dyn ConsolidationStore>,
    ) -> Self {
        Self {
            long_term: Some(long_term),
            reasoning: Some(reasoning),
            short_term: Some(short_term),
            consolidation: Arc::from(consolidation),
        }
    }

    /// Create a consolidation orchestrator with only a consolidation store.
    pub fn from_store(consolidation: Box<dyn ConsolidationStore>) -> Self {
        Self::from_arc(Arc::from(consolidation))
    }

    /// Create a consolidation orchestrator backed by a shared store handle.
    pub fn from_arc(consolidation: Arc<dyn ConsolidationStore>) -> Self {
        Self {
            long_term: None,
            reasoning: None,
            short_term: None,
            consolidation,
        }
    }

    /// Find pairs of entities with high embedding similarity (`BOLT-08-05`).
    pub async fn dedupe_entities(
        &self,
        threshold: Option<f64>,
        limit: Option<i64>,
        dry_run: Option<bool>,
    ) -> MemoryResult<ConsolidationReport> {
        let threshold = threshold.unwrap_or(DEFAULT_DEDUPE_THRESHOLD);
        let limit = limit.unwrap_or(DEFAULT_DEDUPE_LIMIT);
        let dry_run = dry_run.unwrap_or(true);

        let report = self
            .consolidation
            .dedupe_entities(threshold, limit, dry_run)
            .await?;
        report.validate_invariants()?;
        Ok(report)
    }

    /// Find traces with many steps that haven't been summarized (`BOLT-08-08`).
    pub async fn summarize_long_traces(
        &self,
        min_steps: Option<i64>,
        limit: Option<i64>,
        dry_run: Option<bool>,
    ) -> MemoryResult<ConsolidationReport> {
        let min_steps = min_steps.unwrap_or(DEFAULT_MIN_TRACE_STEPS);
        let limit = limit.unwrap_or(DEFAULT_TRACE_LIMIT);
        let dry_run = dry_run.unwrap_or(true);

        let report = self
            .consolidation
            .summarize_long_traces(min_steps, limit, dry_run)
            .await?;
        report.validate_invariants()?;
        Ok(report)
    }

    /// Find preferences with near-duplicate content (`BOLT-08-09`).
    pub async fn detect_superseded_preferences(
        &self,
        threshold: Option<f64>,
        limit: Option<i64>,
        dry_run: Option<bool>,
    ) -> MemoryResult<ConsolidationReport> {
        self.detect_superseded_preferences_for_owner(None, threshold, limit, dry_run)
            .await
    }

    /// Owner-filtered preference supersession (`BOLT-08-09`, `F-09`).
    pub async fn detect_superseded_preferences_for_owner(
        &self,
        user_identifier: Option<&str>,
        threshold: Option<f64>,
        limit: Option<i64>,
        dry_run: Option<bool>,
    ) -> MemoryResult<ConsolidationReport> {
        if let Some(owner) = user_identifier {
            if owner.trim().is_empty() {
                return Err(MemoryError::invalid("user_identifier must not be blank"));
            }
        }
        let threshold = threshold.unwrap_or(DEFAULT_SUPERSEDE_THRESHOLD);
        let limit = limit.unwrap_or(DEFAULT_CONSOLIDATION_LIMIT);
        let dry_run = dry_run.unwrap_or(true);

        let report = self
            .consolidation
            .detect_superseded_preferences_for_owner(user_identifier, threshold, limit, dry_run)
            .await?;
        report.validate_invariants()?;
        Ok(report)
    }

    /// Archive conversations older than an explicit TTL (`BOLT-08-10`, `F-06`).
    ///
    /// Matches upstream `archive_expired_conversations`: omitting `ttl_days`
    /// returns `MemoryError::InvalidInput` rather than silently applying a default.
    pub async fn archive_expired_conversations(
        &self,
        ttl_days: Option<i64>,
        limit: Option<i64>,
        dry_run: Option<bool>,
    ) -> MemoryResult<ConsolidationReport> {
        let ttl_days = ttl_days.ok_or_else(|| {
            MemoryError::invalid(
                "archive_expired_conversations requires an explicit ttl_days value",
            )
        })?;
        let limit = limit.unwrap_or(DEFAULT_CONSOLIDATION_LIMIT);
        let dry_run = dry_run.unwrap_or(true);

        let report = self
            .consolidation
            .archive_expired_conversations(ttl_days, limit, dry_run)
            .await?;
        report.validate_invariants()?;
        Ok(report)
    }

    /// Run all four consolidation jobs in sequence with validated options.
    ///
    /// Validates the full configuration (including mandatory `conversation_ttl_days`)
    /// upfront before executing any job so partial mutations never occur on invalid config.
    pub async fn run_all_with_options(
        &self,
        options: &ConsolidationOptions,
    ) -> MemoryResult<Vec<ConsolidationReport>> {
        options.validate()?;
        let ttl_days = options.conversation_ttl_days.ok_or_else(|| {
            MemoryError::invalid(
                "run_all_with_options requires an explicit conversation_ttl_days value",
            )
        })?;
        let mut reports = Vec::with_capacity(4);
        reports.push(
            self.dedupe_entities(
                Some(options.dedupe_threshold),
                Some(options.dedupe_limit),
                Some(options.dry_run),
            )
            .await?,
        );
        reports.push(
            self.summarize_long_traces(
                Some(options.min_trace_steps),
                Some(options.trace_limit),
                Some(options.dry_run),
            )
            .await?,
        );
        reports.push(
            self.detect_superseded_preferences_for_owner(
                options.user_identifier.as_deref(),
                Some(options.supersede_threshold),
                Some(options.limit),
                Some(options.dry_run),
            )
            .await?,
        );
        reports.push(
            self.archive_expired_conversations(
                Some(ttl_days),
                Some(options.limit),
                Some(options.dry_run),
            )
            .await?,
        );
        Ok(reports)
    }

    /// Convenience helper running all four consolidation jobs with explicit `ttl_days`.
    pub async fn run_all(&self, dry_run: Option<bool>) -> MemoryResult<Vec<ConsolidationReport>> {
        let mut options = ConsolidationOptions {
            dry_run: dry_run.unwrap_or(true),
            ..ConsolidationOptions::default()
        };
        if options.conversation_ttl_days.is_none() {
            options.conversation_ttl_days = Some(DEFAULT_CONVERSATION_TTL_DAYS);
        }
        self.run_all_with_options(&options).await
    }

    /// Persist an explicit `:MemoryReadAudit` node (`BOLT-08-11`).
    pub async fn record_read_audit(&self, record: &ReadAuditRecord) -> MemoryResult<String> {
        self.consolidation.record_read_audit(record).await
    }
}

/// Configuration for consolidation jobs aligned with upstream defaults (`SDD-4`, `F-06`).
#[derive(Debug, Clone)]
pub struct ConsolidationOptions {
    pub dedupe_threshold: f64,
    pub dedupe_limit: i64,
    pub supersede_threshold: f64,
    pub min_trace_steps: i64,
    pub trace_limit: i64,
    /// Explicit TTL in days for conversation archiving (required by `run_all_with_options`).
    pub conversation_ttl_days: Option<i64>,
    /// Optional owner filter for `detect_superseded_preferences_for_owner`.
    pub user_identifier: Option<String>,
    pub limit: i64,
    pub dry_run: bool,
}

impl Default for ConsolidationOptions {
    fn default() -> Self {
        Self {
            dedupe_threshold: DEFAULT_DEDUPE_THRESHOLD,
            dedupe_limit: DEFAULT_DEDUPE_LIMIT,
            supersede_threshold: DEFAULT_SUPERSEDE_THRESHOLD,
            min_trace_steps: DEFAULT_MIN_TRACE_STEPS,
            trace_limit: DEFAULT_TRACE_LIMIT,
            conversation_ttl_days: None,
            user_identifier: None,
            limit: DEFAULT_CONSOLIDATION_LIMIT,
            dry_run: true,
        }
    }
}

impl ConsolidationOptions {
    /// Create options that perform actual mutations (caller must still supply `conversation_ttl_days`).
    pub fn execute() -> Self {
        Self {
            dry_run: false,
            ..Self::default()
        }
    }

    /// Validate all consolidation options upfront before executing any job.
    pub fn validate(&self) -> MemoryResult<()> {
        if !self.dedupe_threshold.is_finite() || !(0.0..=1.0).contains(&self.dedupe_threshold) {
            return Err(MemoryError::invalid(
                "dedupe_threshold must be in [0.0, 1.0]",
            ));
        }
        if !self.supersede_threshold.is_finite() || !(0.0..=1.0).contains(&self.supersede_threshold)
        {
            return Err(MemoryError::invalid(
                "supersede_threshold must be in [0.0, 1.0]",
            ));
        }
        if self.dedupe_limit < 1 || self.trace_limit < 1 || self.limit < 1 {
            return Err(MemoryError::invalid("consolidation limits must be >= 1"));
        }
        if self.min_trace_steps < 1 {
            return Err(MemoryError::invalid("min_trace_steps must be >= 1"));
        }
        if let Some(ttl) = self.conversation_ttl_days {
            if ttl < 0 {
                return Err(MemoryError::invalid("conversation_ttl_days must be >= 0"));
            }
        } else {
            return Err(MemoryError::invalid(
                "conversation_ttl_days must be provided explicitly",
            ));
        }
        if let Some(owner) = &self.user_identifier {
            if owner.trim().is_empty() {
                return Err(MemoryError::invalid("user_identifier must not be blank"));
            }
        }
        Ok(())
    }

    /// Create a `ConsolidationMemory` with these options.
    pub fn build(
        &self,
        long_term: Box<dyn LongTermStore>,
        reasoning: Box<dyn ReasoningStore>,
        short_term: Box<dyn ShortTermStore>,
        consolidation: Box<dyn ConsolidationStore>,
    ) -> ConsolidationMemory {
        ConsolidationMemory::new(long_term, reasoning, short_term, consolidation)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;
    use muninn_core::ports::ConsolidationCandidate;
    use uuid::Uuid;

    struct TestConsolidationStore {
        entity_count: usize,
        trace_count: usize,
        pref_count: usize,
        conversation_count: usize,
        omit_run_id_on_mutation: bool,
        zero_mutations_despite_candidates: bool,
        calls: Arc<AtomicUsize>,
        last_owner: Arc<Mutex<Option<String>>>,
        recorded_runs: Arc<Mutex<Vec<(String, String)>>>,
        recorded_read_audits: Arc<Mutex<Vec<ReadAuditRecord>>>,
    }

    impl TestConsolidationStore {
        fn new(
            entity_count: usize,
            trace_count: usize,
            pref_count: usize,
            conversation_count: usize,
        ) -> Self {
            Self {
                entity_count,
                trace_count,
                pref_count,
                conversation_count,
                omit_run_id_on_mutation: false,
                zero_mutations_despite_candidates: false,
                calls: Arc::new(AtomicUsize::new(0)),
                last_owner: Arc::new(Mutex::new(None)),
                recorded_runs: Arc::new(Mutex::new(Vec::new())),
                recorded_read_audits: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn build_report(
            &self,
            kind: &str,
            candidates: Vec<ConsolidationCandidate>,
            dry_run: bool,
        ) -> ConsolidationReport {
            let mutations_applied = if dry_run || self.zero_mutations_despite_candidates {
                0
            } else {
                candidates.len()
            };
            let run_id = if !dry_run && mutations_applied > 0 && !self.omit_run_id_on_mutation {
                let id = Uuid::new_v4().to_string();
                self.recorded_runs
                    .lock()
                    .unwrap()
                    .push((kind.to_owned(), id.clone()));
                Some(id)
            } else {
                None
            };
            ConsolidationReport {
                kind: kind.to_owned(),
                dry_run,
                candidates_found: candidates.len(),
                mutations_applied,
                candidates,
                run_id,
            }
        }
    }

    #[async_trait::async_trait]
    impl ConsolidationStore for TestConsolidationStore {
        async fn dedupe_entities(
            &self,
            threshold: f64,
            limit: i64,
            dry_run: bool,
        ) -> MemoryResult<ConsolidationReport> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let candidates: Vec<ConsolidationCandidate> =
                (0..std::cmp::min(self.entity_count, limit as usize))
                    .map(|i| ConsolidationCandidate {
                        id: format!("entity-{i}"),
                        kind: "merge".to_owned(),
                        target_id: Some(format!("entity-{}", i + 1)),
                        score: Some(threshold),
                        reason: format!("Similar entities (score: {threshold:.3})"),
                    })
                    .collect();
            Ok(self.build_report("dedupe_entities", candidates, dry_run))
        }

        async fn summarize_long_traces(
            &self,
            _min_steps: i64,
            limit: i64,
            dry_run: bool,
        ) -> MemoryResult<ConsolidationReport> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let candidates: Vec<ConsolidationCandidate> =
                (0..std::cmp::min(self.trace_count, limit as usize))
                    .map(|i| ConsolidationCandidate {
                        id: format!("trace-{i}"),
                        kind: "summarize".to_owned(),
                        target_id: None,
                        score: Some(20.0 + i as f64),
                        reason: "Long trace".to_owned(),
                    })
                    .collect();
            Ok(self.build_report("summarize_long_traces", candidates, dry_run))
        }

        async fn detect_superseded_preferences(
            &self,
            threshold: f64,
            limit: i64,
            dry_run: bool,
        ) -> MemoryResult<ConsolidationReport> {
            self.detect_superseded_preferences_for_owner(None, threshold, limit, dry_run)
                .await
        }

        async fn detect_superseded_preferences_for_owner(
            &self,
            user_identifier: Option<&str>,
            threshold: f64,
            limit: i64,
            dry_run: bool,
        ) -> MemoryResult<ConsolidationReport> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            *self.last_owner.lock().unwrap() = user_identifier.map(str::to_owned);
            let candidates: Vec<ConsolidationCandidate> =
                (0..std::cmp::min(self.pref_count, limit as usize))
                    .map(|i| ConsolidationCandidate {
                        id: format!("pref-old-{i}"),
                        kind: "supersede".to_owned(),
                        target_id: Some(format!("pref-new-{i}")),
                        score: Some(threshold),
                        reason: "Near-duplicate preference".to_owned(),
                    })
                    .collect();
            Ok(self.build_report("detect_superseded_preferences", candidates, dry_run))
        }

        async fn archive_expired_conversations(
            &self,
            _ttl_days: i64,
            limit: i64,
            dry_run: bool,
        ) -> MemoryResult<ConsolidationReport> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let candidates: Vec<ConsolidationCandidate> =
                (0..std::cmp::min(self.conversation_count, limit as usize))
                    .map(|i| ConsolidationCandidate {
                        id: format!("conv-{i}"),
                        kind: "archive".to_owned(),
                        target_id: None,
                        score: Some(90.0),
                        reason: "Expired conversation".to_owned(),
                    })
                    .collect();
            Ok(self.build_report("archive_expired_conversations", candidates, dry_run))
        }

        async fn record_run(&self, kind: &str, stats_json: &str) -> MemoryResult<String> {
            self.recorded_runs
                .lock()
                .unwrap()
                .push((kind.to_owned(), stats_json.to_owned()));
            Ok(Uuid::new_v4().to_string())
        }

        async fn record_read_audit(&self, record: &ReadAuditRecord) -> MemoryResult<String> {
            self.recorded_read_audits
                .lock()
                .unwrap()
                .push(record.clone());
            Ok(Uuid::new_v4().to_string())
        }
    }

    #[tokio::test]
    async fn f05_zero_mutations_writes_no_audit_and_missing_run_id_fails_invariants() {
        // Zero mutations despite candidates -> zero audit runs, valid report.
        let mut zero_store = TestConsolidationStore::new(3, 0, 0, 0);
        zero_store.zero_mutations_despite_candidates = true;
        let runs = zero_store.recorded_runs.clone();
        let memory = ConsolidationMemory::from_store(Box::new(zero_store));
        let report = memory
            .dedupe_entities(None, None, Some(false))
            .await
            .unwrap();
        assert_eq!(report.candidates_found, 3);
        assert_eq!(report.mutations_applied, 0);
        assert!(report.run_id.is_none());
        assert!(runs.lock().unwrap().is_empty());

        // Broken store that applies mutations without atomic run_id -> rejected fail-closed.
        let mut broken_store = TestConsolidationStore::new(2, 0, 0, 0);
        broken_store.omit_run_id_on_mutation = true;
        let broken_runs = broken_store.recorded_runs.clone();
        let broken_memory = ConsolidationMemory::from_store(Box::new(broken_store));
        let err = broken_memory
            .dedupe_entities(None, None, Some(false))
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::Storage(_)), "{err:?}");
        assert!(
            broken_runs.lock().unwrap().is_empty(),
            "orchestrator must never issue a secondary fallback record_run write"
        );
    }

    #[tokio::test]
    async fn f06_upstream_defaults_and_upfront_run_all_validation() {
        let defaults = ConsolidationOptions::default();
        assert!(defaults.dry_run);
        assert_eq!(defaults.dedupe_threshold, 0.95);
        assert_eq!(defaults.dedupe_limit, 10_000);
        assert_eq!(defaults.supersede_threshold, 0.92);
        assert_eq!(defaults.min_trace_steps, 20);
        assert_eq!(defaults.trace_limit, 1_000);
        assert_eq!(defaults.conversation_ttl_days, None);

        let store = TestConsolidationStore::new(2, 2, 2, 2);
        let calls = store.calls.clone();
        let memory = ConsolidationMemory::from_store(Box::new(store));

        // Omitting ttl_days in archive_expired_conversations fails closed.
        let err = memory
            .archive_expired_conversations(None, None, Some(true))
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));

        // run_all_with_options without ttl_days fails BEFORE executing any job!
        let err = memory
            .run_all_with_options(&ConsolidationOptions::execute())
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "upfront validation must prevent partial mutations"
        );
    }

    #[tokio::test]
    async fn f09_detect_superseded_preferences_forwards_owner_and_rejects_blank() {
        let store = TestConsolidationStore::new(0, 0, 3, 0);
        let last_owner = store.last_owner.clone();
        let memory = ConsolidationMemory::from_store(Box::new(store));

        let err = memory
            .detect_superseded_preferences_for_owner(Some("  "), None, None, Some(true))
            .await
            .unwrap_err();
        assert!(matches!(err, MemoryError::InvalidInput(_)));

        let report = memory
            .detect_superseded_preferences_for_owner(Some("alice"), None, None, Some(true))
            .await
            .unwrap();
        assert_eq!(report.candidates_found, 3);
        assert_eq!(last_owner.lock().unwrap().as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn run_all_and_read_audit_work_end_to_end() {
        let store = TestConsolidationStore::new(2, 2, 2, 2);
        let runs = store.recorded_runs.clone();
        let audits = store.recorded_read_audits.clone();
        let memory = ConsolidationMemory::from_store(Box::new(store));

        let reports = memory.run_all(Some(true)).await.unwrap();
        assert_eq!(reports.len(), 4);
        assert!(reports.iter().all(|r| r.dry_run && r.run_id.is_none()));
        assert!(runs.lock().unwrap().is_empty());

        let mut exec_opts = ConsolidationOptions::execute();
        exec_opts.conversation_ttl_days = Some(30);
        exec_opts.user_identifier = Some("alice".into());
        let exec_reports = memory.run_all_with_options(&exec_opts).await.unwrap();
        assert_eq!(exec_reports.len(), 4);
        assert!(exec_reports
            .iter()
            .all(|r| !r.dry_run && r.run_id.is_some()));
        assert_eq!(runs.lock().unwrap().len(), 4);

        let record = ReadAuditRecord::new("preference lookup");
        let audit_id = memory.record_read_audit(&record).await.unwrap();
        assert!(!audit_id.is_empty());
        assert_eq!(audits.lock().unwrap().len(), 1);
    }
}
