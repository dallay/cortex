#[path = "support/providers.rs"]
mod support;
use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::loop_engine::{LoopConfig, StandardLoop};
use huginn_core::{
    providers::*, AgentError, AgentLoop, ApprovalPolicy, ApprovalRequest, CancellationToken, Event,
    EventSink, InstructionSource, Message, ModelDelta, ModelProvider, ModelRequest, PreparedAction,
    Result, Role, Session, Tool, ToolCall, ToolContext, ToolDefinition, ToolRegistry,
};
use std::sync::{Arc, Mutex};
use support::*;

type Calls = Arc<Mutex<Vec<(String, bool)>>>;
struct RecordingAdapter {
    calls: Calls,
    summary_gate: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}
struct RecordingModel {
    label: String,
    calls: Calls,
    summary_gate: Option<(Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>,
}
#[async_trait]
impl ProviderAdapter for RecordingAdapter {
    fn descriptor(&self) -> ProviderDescriptor {
        Adapter {
            id: "recording",
            model: Arc::new(TextModel("unused")),
        }
        .descriptor()
    }
    async fn create_model(
        &self,
        selection: &ModelSelection,
        _: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>> {
        Ok(Arc::new(RecordingModel {
            label: selection.model.clone(),
            calls: self.calls.clone(),
            summary_gate: self.summary_gate.clone(),
        }))
    }
}
#[async_trait]
impl ModelProvider for RecordingModel {
    async fn stream(
        &self,
        request: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        let summary = request
            .messages
            .first()
            .is_some_and(|message| message.content.starts_with("Summarize repository work"));
        self.calls
            .lock()
            .unwrap()
            .push((self.label.clone(), summary));
        if summary {
            if let Some((entered, release)) = &self.summary_gate {
                entered.notify_one();
                signal(release).await;
            }
        }
        let tool = !summary
            && request.messages.last().is_some_and(|message| {
                message.role == Role::User && message.content == "tool-turn"
            });
        let delta = if tool {
            ModelDelta::ToolCall(ToolCall {
                id: "one".into(),
                name: "switch".into(),
                arguments: serde_json::json!({}),
            })
        } else {
            ModelDelta::Text(if summary {
                "short summary".into()
            } else {
                self.label.clone()
            })
        };
        Ok(futures::stream::iter([Ok(delta), Ok(ModelDelta::Finished)]).boxed())
    }
}
struct Instructions;
#[async_trait]
impl InstructionSource for Instructions {
    async fn load(&self, _: &std::path::Path) -> Result<String> {
        Ok(String::new())
    }
}
struct Policy;
#[async_trait]
impl ApprovalPolicy for Policy {
    async fn approve(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<bool> {
        panic!("no effect approval expected")
    }
}
struct Events;
impl EventSink for Events {
    fn emit(&self, _: Event) {}
}
struct Switch {
    management: Arc<ProvidersService>,
    connection: ConnectionIdentity,
    revoke: bool,
}
#[async_trait]
impl Tool for Switch {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "switch".into(),
            description: "test switch".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }
    }
    async fn prepare(&self, _: &ToolCall, _: &ToolContext) -> Result<PreparedAction> {
        Ok(PreparedAction {
            approval: None,
            payload: serde_json::json!({}),
        })
    }
    async fn execute(&self, _: PreparedAction, _: &ToolContext) -> Result<String> {
        if self.revoke {
            self.management
                .0
                .execute(
                    ProviderAction::Revoke(self.connection.clone()),
                    CancellationToken::new(),
                )
                .await?;
        } else {
            select(&self.management, &self.connection, "beta").await;
        }
        Ok("switch complete".into())
    }
}
struct Tools(Option<Arc<dyn Tool>>);
impl ToolRegistry for Tools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.0
            .as_ref()
            .map(|tool| vec![tool.definition()])
            .unwrap_or_default()
    }
    fn get(&self, _: &str) -> Option<Arc<dyn Tool>> {
        self.0.clone()
    }
}
fn engine(model: Arc<dyn ModelProvider>, tool: Option<Arc<dyn Tool>>) -> StandardLoop {
    StandardLoop {
        model,
        tools: Arc::new(Tools(tool)),
        sessions: Arc::new(Memory(Mutex::new(None))),
        instructions: Arc::new(Instructions),
        config: LoopConfig::default(),
        lifecycle: CancellationToken::new(),
    }
}
fn history() -> Session {
    let mut session = Session::new("/nonexistent/providers-test".into());
    for _ in 0..2 {
        session
            .messages
            .push(Message::text(Role::User, "u".repeat(1000)));
        session
            .messages
            .push(Message::text(Role::Assistant, "a".repeat(1000)));
    }
    session
}

#[tokio::test]
async fn tool_selection_applies_to_next_turn_and_revocation_stops_continuation() {
    for revoke in [false, true] {
        let (mut supervisor, management, catalog, router) = start().await;
        let calls = Arc::new(Mutex::new(vec![]));
        supervisor
            .register(Box::new(AdapterPlugin::new(
                "recording",
                Arc::new(RecordingAdapter {
                    calls: calls.clone(),
                    summary_gate: None,
                }),
            )))
            .unwrap();
        supervisor.start().await.unwrap();
        let connection = connect(&management, &catalog, "recording", "one").await;
        select(&management, &connection, "alpha").await;
        let engine = engine(
            router.0.clone(),
            Some(Arc::new(Switch {
                management: management.clone(),
                connection: connection.clone(),
                revoke,
            })),
        );
        let mut session = Session::new("/nonexistent/providers-test".into());
        let result = engine
            .run(
                &mut session,
                "tool-turn".into(),
                &Policy,
                &Events,
                CancellationToken::new(),
            )
            .await;
        if revoke {
            assert!(matches!(
                result,
                Err(AgentError::Provider(ProviderError::Revoked))
            ));
            assert_eq!(*calls.lock().unwrap(), vec![("alpha".into(), false)]);
            assert!(session.interrupted);
        } else {
            result.unwrap();
            assert_eq!(
                *calls.lock().unwrap(),
                vec![("alpha".into(), false), ("alpha".into(), false)]
            );
            engine
                .run(
                    &mut session,
                    "next".into(),
                    &Policy,
                    &Events,
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            assert_eq!(calls.lock().unwrap().last().unwrap().0, "beta");
        }
        supervisor.shutdown().await;
    }
}

#[tokio::test]
async fn automatic_and_manual_compaction_hold_the_original_selection() {
    for manual in [false, true] {
        let (mut supervisor, management, catalog, router) = start().await;
        let calls = Arc::new(Mutex::new(vec![]));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        supervisor
            .register(Box::new(AdapterPlugin::new(
                "recording",
                Arc::new(RecordingAdapter {
                    calls: calls.clone(),
                    summary_gate: Some((entered.clone(), release.clone())),
                }),
            )))
            .unwrap();
        supervisor.start().await.unwrap();
        let connection = connect(&management, &catalog, "recording", "one").await;
        select(&management, &connection, "alpha").await;
        let mut engine = engine(router.0.clone(), None);
        engine.config.context_tokens = if manual { 10000 } else { 5000 };
        engine.config.max_output_tokens = 100;
        let task = tokio::spawn(async move {
            let mut session = history();
            if manual {
                engine
                    .compact(&mut session, &Events, CancellationToken::new())
                    .await
                    .unwrap();
            } else {
                engine
                    .run(
                        &mut session,
                        "next".into(),
                        &Policy,
                        &Events,
                        CancellationToken::new(),
                    )
                    .await
                    .unwrap();
            }
            (engine, session)
        });
        signal(&entered).await;
        select(&management, &connection, "beta").await;
        release.notify_one();
        let (engine, mut session) = task.await.unwrap();
        assert!(session.summary.is_some());
        assert!(calls
            .lock()
            .unwrap()
            .iter()
            .all(|(model, _)| model == "alpha"));
        assert!(calls.lock().unwrap()[0].1);
        if !manual {
            assert_eq!(calls.lock().unwrap().len(), 2);
        }
        engine
            .run(
                &mut session,
                "later".into(),
                &Policy,
                &Events,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(calls.lock().unwrap().last().unwrap().0, "beta");
        supervisor.shutdown().await;
    }
}

struct GatedInstructions {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
#[async_trait]
impl InstructionSource for GatedInstructions {
    async fn load(&self, _: &std::path::Path) -> Result<String> {
        self.entered.notify_one();
        signal(&self.release).await;
        Ok(String::new())
    }
}
#[tokio::test]
async fn concurrent_turns_pin_before_instructions_io_and_do_not_share_selection() {
    let (mut supervisor, management, catalog, router) = start().await;
    let calls = Arc::new(Mutex::new(vec![]));
    supervisor
        .register(Box::new(AdapterPlugin::new(
            "recording",
            Arc::new(RecordingAdapter {
                calls: calls.clone(),
                summary_gate: None,
            }),
        )))
        .unwrap();
    supervisor.start().await.unwrap();
    let connection = connect(&management, &catalog, "recording", "one").await;
    select(&management, &connection, "alpha").await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let mut first = engine(router.0.clone(), None);
    first.instructions = Arc::new(GatedInstructions {
        entered: entered.clone(),
        release: release.clone(),
    });
    let task = tokio::spawn(async move {
        let mut session = Session::new("/nonexistent/first".into());
        first
            .run(
                &mut session,
                "first".into(),
                &Policy,
                &Events,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        session
    });
    signal(&entered).await;
    select(&management, &connection, "beta").await;
    let second = engine(router.0.clone(), None);
    let mut session = Session::new("/nonexistent/second".into());
    second
        .run(
            &mut session,
            "second".into(),
            &Policy,
            &Events,
            CancellationToken::new(),
        )
        .await
        .unwrap();
    release.notify_one();
    let first_session = task.await.unwrap();
    assert_eq!(session.messages.last().unwrap().content, "beta");
    assert_eq!(first_session.messages.last().unwrap().content, "alpha");
    assert_eq!(
        *calls.lock().unwrap(),
        vec![("beta".into(), false), ("alpha".into(), false)]
    );
    supervisor.shutdown().await;
}

#[tokio::test]
async fn unconfigured_turn_preserves_prompt_and_error_without_fake_output() {
    let (mut supervisor, _, _, router) = start().await;
    let engine = engine(router.0.clone(), None);
    let mut session = Session::new("/nonexistent/providers-test".into());
    assert!(engine
        .run(
            &mut session,
            "attempt".into(),
            &Policy,
            &Events,
            CancellationToken::new()
        )
        .await
        .is_err());
    assert_eq!(session.messages.len(), 1);
    assert_eq!(session.messages[0].content, "attempt");
    assert!(session.interrupted);
    assert!(matches!(session.events.last(), Some(Event::Error { .. })));
    supervisor.shutdown().await;
}
