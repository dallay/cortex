use agent_core::{
    AgentError, AgentLoop, ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink,
    ModelDelta, ModelProvider, ModelRequest, Result, Role, Session, SessionInfo, SessionStore,
};
use agent_runtime::{
    loop_engine::{LoopConfig, StandardLoop},
    sessions::SqliteSessions,
    tools::Registry,
};
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use std::sync::{Arc, Mutex};

struct CountingStore {
    inner: SqliteSessions,
    snapshots: Mutex<Vec<Session>>,
}
#[async_trait]
impl SessionStore for CountingStore {
    async fn save(&self, session: &Session) -> Result<()> {
        self.inner.save(session).await?;
        self.snapshots.lock().unwrap().push(session.clone());
        Ok(())
    }
    async fn load(&self, id: &str) -> Result<Session> {
        self.inner.load(id).await
    }
    async fn list(&self) -> Result<Vec<SessionInfo>> {
        self.inner.list().await
    }
}

#[derive(Clone, Copy)]
enum Ending {
    Finished,
    Incomplete,
    Error,
    Cancel,
    LifecycleCancel,
}
struct Model {
    chunks: usize,
    ending: Ending,
    requests: Mutex<Vec<ModelRequest>>,
}
#[async_trait]
impl ModelProvider for Model {
    async fn stream(
        &self,
        request: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        self.requests.lock().unwrap().push(request);
        let mut deltas: Vec<_> = (0..self.chunks)
            .map(|_| Ok(ModelDelta::Text("é".into())))
            .collect();
        match self.ending {
            Ending::Finished => {
                deltas.push(Ok(ModelDelta::Usage {
                    input_tokens: 10,
                    output_tokens: 20,
                }));
                deltas.push(Ok(ModelDelta::Finished));
            }
            Ending::Error => deltas.push(Err(AgentError::Model("stream failed".into()))),
            Ending::Incomplete | Ending::Cancel | Ending::LifecycleCancel => {}
        }
        let stream = futures::stream::iter(deltas);
        if matches!(self.ending, Ending::Cancel | Ending::LifecycleCancel) {
            // Keep the provider open so cancellation cannot race with stream exhaustion.
            Ok(stream.chain(futures::stream::pending()).boxed())
        } else {
            Ok(stream.boxed())
        }
    }
}
struct NoApproval;
#[async_trait]
impl ApprovalPolicy for NoApproval {
    async fn approve(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        panic!("text-only turns must not request approval");
    }
}
struct Sink {
    events: Mutex<Vec<Event>>,
    saves_at_text: Mutex<Vec<usize>>,
    store: Arc<CountingStore>,
    cancel_on_text: Option<CancellationToken>,
}
impl EventSink for Sink {
    fn emit(&self, event: Event) {
        if matches!(event, Event::Text { .. }) {
            self.saves_at_text
                .lock()
                .unwrap()
                .push(self.store.snapshots.lock().unwrap().len());
            if let Some(cancel) = &self.cancel_on_text {
                cancel.cancel();
            }
        }
        self.events.lock().unwrap().push(event);
    }
}

#[tokio::test]
async fn text_deltas_are_live_only_with_bounded_saves_and_durable_resume() {
    for chunks in [1, 128] {
        let workspace = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let db = data.path().join("sessions.db");
        let store = Arc::new(CountingStore {
            inner: SqliteSessions::open(&db).unwrap(),
            snapshots: Mutex::new(vec![]),
        });
        let model = Arc::new(Model {
            chunks,
            ending: Ending::Finished,
            requests: Mutex::new(vec![]),
        });
        let mut engine = StandardLoop {
            model: model.clone(),
            tools: Arc::new(Registry::native(2).unwrap()),
            sessions: store.clone(),
            config: LoopConfig::default(),
            lifecycle: CancellationToken::new(),
        };
        let sink = Sink {
            events: Mutex::new(vec![]),
            saves_at_text: Mutex::new(vec![]),
            store: store.clone(),
            cancel_on_text: None,
        };
        let mut session = Session::new(workspace.path().canonicalize().unwrap());
        engine
            .run(
                &mut session,
                "hello".into(),
                &NoApproval,
                &sink,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let live: Vec<_> = sink
            .events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| {
                if let Event::Text { text } = event {
                    Some(text.clone())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(live, vec!["é"; chunks]);
        assert_eq!(*sink.saves_at_text.lock().unwrap(), vec![2; chunks]);
        {
            let snapshots = store.snapshots.lock().unwrap();
            assert_eq!(
                snapshots.len(),
                5,
                "save count must not depend on text delta count"
            );
            assert!(snapshots
                .iter()
                .all(|s| s.events.iter().all(|e| !matches!(e, Event::Text { .. }))));
            drop(snapshots);
        }
        let reopened = Arc::new(SqliteSessions::open(&db).unwrap());
        let mut resumed = reopened.load(&session.id).await.unwrap();
        assert!(!resumed.interrupted);
        assert_eq!(resumed.messages.last().unwrap().content, "é".repeat(chunks));
        assert!(resumed
            .events
            .iter()
            .any(|e| matches!(e, Event::Usage { .. })));
        assert!(matches!(resumed.events.last(), Some(Event::TurnFinished)));
        engine.sessions = reopened;
        engine
            .run(
                &mut resumed,
                "continue".into(),
                &NoApproval,
                &sink,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(model.requests.lock().unwrap()[1]
            .messages
            .iter()
            .any(|m| { m.role == Role::Assistant && m.content == "é".repeat(chunks) }));
    }
}

#[tokio::test]
async fn partial_text_is_not_persisted_on_incomplete_error_or_cancellation() {
    for ending in [
        Ending::Incomplete,
        Ending::Error,
        Ending::Cancel,
        Ending::LifecycleCancel,
    ] {
        let workspace = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let db = data.path().join("sessions.db");
        let store = Arc::new(CountingStore {
            inner: SqliteSessions::open(&db).unwrap(),
            snapshots: Mutex::new(vec![]),
        });
        let cancel = CancellationToken::new();
        let lifecycle = CancellationToken::new();
        let sink = Sink {
            events: Mutex::new(vec![]),
            saves_at_text: Mutex::new(vec![]),
            store: store.clone(),
            cancel_on_text: match ending {
                Ending::Cancel => Some(cancel.clone()),
                Ending::LifecycleCancel => Some(lifecycle.clone()),
                _ => None,
            },
        };
        let engine = StandardLoop {
            model: Arc::new(Model {
                chunks: 1,
                ending,
                requests: Mutex::new(vec![]),
            }),
            tools: Arc::new(Registry::native(2).unwrap()),
            sessions: store.clone(),
            config: LoopConfig::default(),
            lifecycle,
        };
        let mut session = Session::new(workspace.path().canonicalize().unwrap());
        let error = engine
            .run(&mut session, "hello".into(), &NoApproval, &sink, cancel)
            .await
            .unwrap_err();
        match ending {
            Ending::Cancel | Ending::LifecycleCancel => {
                assert!(matches!(error, AgentError::Cancelled))
            }
            _ => assert!(matches!(error, AgentError::Model(_))),
        }
        assert!(sink
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, Event::Text { text } if text == "é")));
        assert_eq!(*sink.saves_at_text.lock().unwrap(), vec![2]);
        assert_eq!(store.snapshots.lock().unwrap().len(), 3);
        let reloaded = SqliteSessions::open(&db)
            .unwrap()
            .load(&session.id)
            .await
            .unwrap();
        assert!(reloaded.interrupted);
        assert_eq!(reloaded.messages.len(), 1);
        assert_eq!(reloaded.messages[0].role, Role::User);
        assert!(reloaded
            .events
            .iter()
            .all(|e| !matches!(e, Event::Text { .. } | Event::TurnFinished)));
        assert!(matches!(reloaded.events.last(), Some(Event::Error { .. })));
    }
}
