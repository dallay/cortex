//! Deterministic end-to-end tests for turn usage accounting (#766):
//! - single and multi-inference turns (tool continuation + compaction + retries)
//! - failed/interrupted/cancelled attempts preserving reported usage
//! - cumulative vs incremental usage semantics without double counting
//! - missing fields, total absence, and explicit zero distinction
//! - mid-turn selection change A -> B preserving attribution to A

#[path = "support/providers.rs"]
mod support;

use async_trait::async_trait;
use futures::{stream::BoxStream, StreamExt};
use huginn_application::loop_engine::{LoopConfig, StandardLoop};
use huginn_core::{
    providers::{
        AuthenticationMethod, ModelDescriptor, ModelSelection, ProviderAction, ProviderAdapter,
        ProviderDescriptor,
    },
    AgentError, AgentLoop, ApprovalDecision, ApprovalPolicy, ApprovalRequest, CancellationToken,
    Event, EventSink, ImageAttachment, InstructionSource, Message, ModelDelta, ModelProvider,
    ModelRequest, PreparedAction, Result, Role, Session, SessionInfo, SessionStore, Tool, ToolCall,
    ToolContext, ToolDefinition, ToolRegistry, TurnUsage, UsageAttribution, UsageCompleteness,
    UsageSemantics,
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

struct EmptyInstructions;
#[async_trait]
impl InstructionSource for EmptyInstructions {
    async fn load(&self, _: &std::path::Path) -> Result<String> {
        Ok(String::new())
    }
}

#[derive(Clone, Default)]
struct MemorySessions(Arc<Mutex<Option<Session>>>);
#[async_trait]
impl SessionStore for MemorySessions {
    async fn save(&self, session: &Session) -> Result<()> {
        *self.0.lock().unwrap() = Some(session.clone());
        Ok(())
    }
    async fn load(&self, _: &str) -> Result<Session> {
        Ok(self.0.lock().unwrap().clone().unwrap())
    }
    async fn list(&self) -> Result<Vec<SessionInfo>> {
        Ok(vec![])
    }
}

#[derive(Default)]
struct RecordingSink(Mutex<Vec<Event>>);
impl EventSink for RecordingSink {
    fn emit(&self, event: Event) {
        self.0.lock().unwrap().push(event);
    }
}

struct AutoApprove;
#[async_trait]
impl ApprovalPolicy for AutoApprove {
    async fn approve(&self, _: &ApprovalRequest, _: CancellationToken) -> Result<ApprovalDecision> {
        Ok(ApprovalDecision::ApproveOnce)
    }
}

struct CallbackTool {
    name: String,
    on_execute: Option<Arc<dyn Fn() + Send + Sync>>,
}
#[async_trait]
impl Tool for CallbackTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: self.name.clone(),
            description: "deterministic test tool".into(),
            input_schema: serde_json::json!({}),
        }
    }
    async fn prepare(&self, _: &ToolCall, _: &ToolContext) -> Result<PreparedAction> {
        Ok(PreparedAction {
            approval: None,
            payload: serde_json::json!({}),
        })
    }
    async fn execute(&self, _: PreparedAction, _: &ToolContext) -> Result<String> {
        if let Some(cb) = &self.on_execute {
            cb();
        }
        Ok("tool-ok".into())
    }
}

struct StaticTools(Vec<Arc<dyn Tool>>);
impl ToolRegistry for StaticTools {
    fn definitions(&self) -> Vec<ToolDefinition> {
        self.0.iter().map(|tool| tool.definition()).collect()
    }
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.0
            .iter()
            .find(|tool| tool.definition().name == name)
            .cloned()
    }
}

type ScriptStep = Vec<Result<ModelDelta>>;

struct ScriptedProvider {
    attribution: Option<UsageAttribution>,
    steps: Mutex<VecDeque<ScriptStep>>,
    request_limits: Option<Arc<Mutex<Vec<u32>>>>,
    reject_image_requests: bool,
}

impl ScriptedProvider {
    fn new(attribution: Option<UsageAttribution>, steps: Vec<ScriptStep>) -> Self {
        Self {
            attribution,
            steps: Mutex::new(steps.into()),
            request_limits: None,
            reject_image_requests: false,
        }
    }

    fn rejecting_image_requests(mut self) -> Self {
        self.reject_image_requests = true;
        self
    }

    fn recording_requests(
        attribution: Option<UsageAttribution>,
        steps: Vec<ScriptStep>,
    ) -> (Self, Arc<Mutex<Vec<u32>>>) {
        let request_limits = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                attribution,
                steps: Mutex::new(steps.into()),
                request_limits: Some(request_limits.clone()),
                reject_image_requests: false,
            },
            request_limits,
        )
    }
}

#[async_trait]
impl ModelProvider for ScriptedProvider {
    fn attribution(&self) -> Option<UsageAttribution> {
        self.attribution.clone()
    }

    async fn stream(
        &self,
        request: ModelRequest,
        _: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
        if self.reject_image_requests
            && request
                .messages
                .iter()
                .any(|message| !message.images.is_empty())
        {
            return Err(AgentError::Model(
                "model does not support image inputs".into(),
            ));
        }
        if let Some(request_limits) = &self.request_limits {
            request_limits.lock().unwrap().push(request.max_tokens);
        }
        let items = self
            .steps
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted provider ran out of steps");
        Ok(futures::stream::iter(items).boxed())
    }
}

fn attr(provider: &str, model: &str) -> UsageAttribution {
    UsageAttribution {
        provider: Some(provider.into()),
        connection: Some("conn-1".into()),
        model: model.into(),
    }
}

fn one_pixel_png() -> ImageAttachment {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    bytes.extend_from_slice(&13u32.to_be_bytes());
    bytes.extend_from_slice(b"IHDR");
    bytes.extend_from_slice(&1u32.to_be_bytes());
    bytes.extend_from_slice(&1u32.to_be_bytes());
    bytes.extend_from_slice(&[8, 2, 0, 0, 0]);
    ImageAttachment::from_bytes(bytes, None).unwrap()
}

fn make_loop(
    model: Arc<dyn ModelProvider>,
    tools: Vec<Arc<dyn Tool>>,
    config: LoopConfig,
) -> (StandardLoop, MemorySessions) {
    let sessions = MemorySessions::default();
    let engine = StandardLoop {
        model,
        tools: Arc::new(StaticTools(tools)),
        sessions: Arc::new(sessions.clone()),
        instructions: Arc::new(EmptyInstructions),
        config,
        lifecycle: CancellationToken::new(),
    };
    (engine, sessions)
}

#[tokio::test]
async fn continuation_request_uses_only_the_remaining_turn_output_budget() {
    let (model, request_limits) = ScriptedProvider::recording_requests(
        Some(attr("openai", "budgeted-model")),
        vec![
            vec![
                Ok(ModelDelta::ToolCall(ToolCall {
                    id: "call-1".into(),
                    name: "inspect".into(),
                    arguments: serde_json::json!({}),
                })),
                Ok(ModelDelta::Usage {
                    input_tokens: Some(10),
                    output_tokens: Some(5),
                    semantics: UsageSemantics::Cumulative,
                }),
                Ok(ModelDelta::Finished),
            ],
            vec![
                Ok(ModelDelta::Text("final answer".into())),
                Ok(ModelDelta::Usage {
                    input_tokens: Some(10),
                    output_tokens: Some(4),
                    semantics: UsageSemantics::Cumulative,
                }),
                Ok(ModelDelta::Finished),
            ],
        ],
    );
    let config = LoopConfig {
        max_turn_tokens: Some(40),
        max_output_tokens: 100,
        ..Default::default()
    };
    let (engine, _) = make_loop(
        Arc::new(model),
        vec![Arc::new(CallbackTool {
            name: "inspect".into(),
            on_execute: None,
        })],
        config,
    );
    let mut session = Session::new(".".into());
    engine
        .run(
            &mut session,
            "inspect the workspace".into(),
            &AutoApprove,
            &RecordingSink::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(*request_limits.lock().unwrap(), vec![40, 25]);
}

#[tokio::test]
async fn rejected_image_request_does_not_poison_later_text_only_turns() {
    let model = ScriptedProvider::new(
        Some(attr("openai", "text-only-model")),
        vec![vec![
            Ok(ModelDelta::Text("text-only response".into())),
            Ok(ModelDelta::Finished),
        ]],
    )
    .rejecting_image_requests();
    let (engine, _) = make_loop(Arc::new(model), vec![], LoopConfig::default());
    let mut session = Session::new(".".into());

    let rejected = engine
        .run_with_images(
            &mut session,
            "describe this image".into(),
            vec![one_pixel_png()],
            &AutoApprove,
            &RecordingSink::default(),
            CancellationToken::new(),
        )
        .await;
    assert!(rejected.is_err());
    let rejected_message = session
        .messages
        .iter()
        .find(|message| message.role == Role::User && message.content == "describe this image")
        .unwrap();
    assert!(rejected_message.images.is_empty());

    engine
        .run(
            &mut session,
            "continue with text only".into(),
            &AutoApprove,
            &RecordingSink::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn multi_inference_turn_aggregates_tool_continuation_and_compaction() {
    let model = Arc::new(ScriptedProvider::new(
        Some(attr("openai", "model-a")),
        vec![
            // Step 1: in-turn compaction (summarize)
            vec![
                Ok(ModelDelta::Text("compact summary".into())),
                Ok(ModelDelta::Usage {
                    input_tokens: Some(40),
                    output_tokens: Some(15),
                    semantics: UsageSemantics::Cumulative,
                }),
                Ok(ModelDelta::Finished),
            ],
            // Step 1: main inference requests a tool
            vec![
                Ok(ModelDelta::ToolCall(ToolCall {
                    id: "call-1".into(),
                    name: "inspect".into(),
                    arguments: serde_json::json!({}),
                })),
                Ok(ModelDelta::Usage {
                    input_tokens: Some(100),
                    output_tokens: Some(25),
                    semantics: UsageSemantics::Cumulative,
                }),
                Ok(ModelDelta::Finished),
            ],
            // Step 2: continuation after tool
            vec![
                Ok(ModelDelta::Text("final answer".into())),
                Ok(ModelDelta::Usage {
                    input_tokens: Some(120),
                    output_tokens: Some(30),
                    semantics: UsageSemantics::Cumulative,
                }),
                Ok(ModelDelta::Finished),
            ],
        ],
    ));
    // Small enough so the pre-seeded completed turns trigger in-turn compaction,
    // while leaving room for the summary request itself.
    let config = LoopConfig {
        context_tokens: 4000,
        max_output_tokens: 100,
        ..Default::default()
    };
    let (engine, _) = make_loop(
        model,
        vec![Arc::new(CallbackTool {
            name: "inspect".into(),
            on_execute: None,
        })],
        config,
    );

    let mut session = Session::new(".".into());
    // Two completed prior turns large enough to exceed 80% of context_tokens (3200 bytes),
    // while the first turn alone fits inside the compaction budget.
    session
        .messages
        .push(Message::text(Role::User, "u".repeat(1000)));
    session
        .messages
        .push(Message::text(Role::Assistant, "a".repeat(1000)));
    session
        .messages
        .push(Message::text(Role::User, "u".repeat(1000)));
    session
        .messages
        .push(Message::text(Role::Assistant, "a".repeat(1000)));

    let sink = RecordingSink::default();
    engine
        .run(
            &mut session,
            "continue".into(),
            &AutoApprove,
            &sink,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert!(
        session
            .events
            .iter()
            .any(|event| matches!(event, Event::Compacted { .. })),
        "expected in-turn compaction to run"
    );

    let turn_usage =
        TurnUsage::from_session_events(&session.events).expect("turn usage must be recorded");
    assert_eq!(turn_usage.requests, 3);
    assert_eq!(turn_usage.input_tokens, Some(260));
    assert_eq!(turn_usage.output_tokens, Some(70));
    assert_eq!(turn_usage.completeness(), UsageCompleteness::Complete);
    assert_eq!(turn_usage.attribution, Some(attr("openai", "model-a")));
}

#[tokio::test]
async fn retry_and_interruption_after_usage_preserve_failed_attempt_consumption() {
    let model = Arc::new(ScriptedProvider::new(
        Some(attr("openai", "model-retry")),
        vec![
            // Attempt 1: reports cumulative usage, then stream drops (retryable)
            vec![
                Ok(ModelDelta::Text("partial".into())),
                Ok(ModelDelta::Usage {
                    input_tokens: Some(80),
                    output_tokens: Some(10),
                    semantics: UsageSemantics::Cumulative,
                }),
                Err(AgentError::Inference("stream interrupted".into())),
            ],
            // Attempt 2: succeeds with its own usage
            vec![
                Ok(ModelDelta::Text("recovered".into())),
                Ok(ModelDelta::Usage {
                    input_tokens: Some(80),
                    output_tokens: Some(20),
                    semantics: UsageSemantics::Cumulative,
                }),
                Ok(ModelDelta::Finished),
            ],
        ],
    ));
    let config = LoopConfig {
        max_model_retries: 1,
        ..Default::default()
    };
    let (engine, _) = make_loop(model, vec![], config);

    let mut session = Session::new(".".into());
    let sink = RecordingSink::default();
    engine
        .run(
            &mut session,
            "retry me".into(),
            &AutoApprove,
            &sink,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    let turn_usage = TurnUsage::from_session_events(&session.events).expect("turn usage");
    assert_eq!(turn_usage.requests, 2);
    assert_eq!(turn_usage.input_tokens, Some(160));
    assert_eq!(turn_usage.output_tokens, Some(30));
    assert_eq!(turn_usage.completeness(), UsageCompleteness::Complete);
}

#[tokio::test]
async fn failed_attempt_without_usage_after_output_marks_turn_partial() {
    let model = Arc::new(ScriptedProvider::new(
        Some(attr("openai", "model-partial")),
        vec![
            // Attempt 1: produces output then drops without reporting usage
            vec![
                Ok(ModelDelta::Text("half".into())),
                Err(AgentError::Inference("connection reset".into())),
            ],
            // Attempt 2: succeeds with complete usage
            vec![
                Ok(ModelDelta::Text("complete".into())),
                Ok(ModelDelta::Usage {
                    input_tokens: Some(50),
                    output_tokens: Some(12),
                    semantics: UsageSemantics::Cumulative,
                }),
                Ok(ModelDelta::Finished),
            ],
        ],
    ));
    let config = LoopConfig {
        max_model_retries: 1,
        ..Default::default()
    };
    let (engine, _) = make_loop(model, vec![], config);

    let mut session = Session::new(".".into());
    let sink = RecordingSink::default();
    engine
        .run(
            &mut session,
            "retry partial".into(),
            &AutoApprove,
            &sink,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    let turn_usage = TurnUsage::from_session_events(&session.events).expect("turn usage");
    assert_eq!(turn_usage.requests, 2);
    assert_eq!(turn_usage.input_tokens, Some(50));
    assert_eq!(turn_usage.output_tokens, Some(12));
    assert_eq!(turn_usage.missing_input_requests, 1);
    assert_eq!(turn_usage.missing_output_requests, 1);
    assert_eq!(turn_usage.completeness(), UsageCompleteness::Partial);
}

#[tokio::test]
async fn cumulative_and_incremental_semantics_are_distinguished_within_a_request() {
    // Cumulative: two snapshots in one request must not double-count.
    let cumulative_model = Arc::new(ScriptedProvider::new(
        Some(attr("openai", "cum")),
        vec![vec![
            Ok(ModelDelta::Usage {
                input_tokens: Some(100),
                output_tokens: Some(5),
                semantics: UsageSemantics::Cumulative,
            }),
            Ok(ModelDelta::Text("hello".into())),
            Ok(ModelDelta::Usage {
                input_tokens: Some(100),
                output_tokens: Some(12),
                semantics: UsageSemantics::Cumulative,
            }),
            Ok(ModelDelta::Finished),
        ]],
    ));
    let (engine, _) = make_loop(cumulative_model, vec![], LoopConfig::default());
    let mut session = Session::new(".".into());
    engine
        .run(
            &mut session,
            "hi".into(),
            &AutoApprove,
            &RecordingSink::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let cum = TurnUsage::from_session_events(&session.events).expect("cumulative usage");
    assert_eq!(cum.input_tokens, Some(100));
    assert_eq!(cum.output_tokens, Some(12));
    assert_eq!(cum.completeness(), UsageCompleteness::Complete);

    // Incremental: two deltas in one request must be summed.
    let incremental_model = Arc::new(ScriptedProvider::new(
        Some(attr("openai", "inc")),
        vec![vec![
            Ok(ModelDelta::Usage {
                input_tokens: Some(100),
                output_tokens: Some(5),
                semantics: UsageSemantics::Incremental,
            }),
            Ok(ModelDelta::Text("hello".into())),
            Ok(ModelDelta::Usage {
                input_tokens: None,
                output_tokens: Some(7),
                semantics: UsageSemantics::Incremental,
            }),
            Ok(ModelDelta::Finished),
        ]],
    ));
    let (engine, _) = make_loop(incremental_model, vec![], LoopConfig::default());
    let mut session = Session::new(".".into());
    engine
        .run(
            &mut session,
            "hi".into(),
            &AutoApprove,
            &RecordingSink::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let inc = TurnUsage::from_session_events(&session.events).expect("incremental usage");
    assert_eq!(inc.input_tokens, Some(100));
    assert_eq!(inc.output_tokens, Some(12));
    assert_eq!(inc.completeness(), UsageCompleteness::Complete);
}

#[tokio::test]
async fn absence_partial_and_explicit_zero_remain_distinct_through_loop() {
    // Total absence -> Unknown
    let absent = Arc::new(ScriptedProvider::new(
        Some(attr("openai", "m")),
        vec![vec![
            Ok(ModelDelta::Text("no usage".into())),
            Ok(ModelDelta::Finished),
        ]],
    ));
    let (engine, _) = make_loop(absent, vec![], LoopConfig::default());
    let mut session = Session::new(".".into());
    engine
        .run(
            &mut session,
            "a".into(),
            &AutoApprove,
            &RecordingSink::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let u = TurnUsage::from_session_events(&session.events).expect("unknown usage recorded");
    assert_eq!(u.completeness(), UsageCompleteness::Unknown);
    assert_eq!(u.input_tokens, None);
    assert_eq!(u.output_tokens, None);

    // Explicit zero -> Complete with Some(0)/Some(0)
    let zero = Arc::new(ScriptedProvider::new(
        Some(attr("openai", "m")),
        vec![vec![
            Ok(ModelDelta::Text("zero".into())),
            Ok(ModelDelta::Usage {
                input_tokens: Some(0),
                output_tokens: Some(0),
                semantics: UsageSemantics::Cumulative,
            }),
            Ok(ModelDelta::Finished),
        ]],
    ));
    let (engine, _) = make_loop(zero, vec![], LoopConfig::default());
    let mut session = Session::new(".".into());
    engine
        .run(
            &mut session,
            "b".into(),
            &AutoApprove,
            &RecordingSink::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();
    let u = TurnUsage::from_session_events(&session.events).expect("zero usage recorded");
    assert_eq!(u.completeness(), UsageCompleteness::Complete);
    assert_eq!(u.input_tokens, Some(0));
    assert_eq!(u.output_tokens, Some(0));
}

struct AdapterWithUsage {
    provider_id: String,
    model_id: String,
    steps: Arc<Mutex<VecDeque<ScriptStep>>>,
}

#[async_trait]
impl ProviderAdapter for AdapterWithUsage {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: self.provider_id.clone(),
            display_name: self.provider_id.clone(),
            models: vec![ModelDescriptor {
                id: self.model_id.clone(),
                supports_tools: true,
            }],
            authentication: vec![AuthenticationMethod::None],
        }
    }

    async fn create_model(
        &self,
        _: &ModelSelection,
        _: CancellationToken,
    ) -> Result<Arc<dyn ModelProvider>> {
        let steps = self.steps.clone();
        struct SharedScript(Arc<Mutex<VecDeque<ScriptStep>>>);
        #[async_trait]
        impl ModelProvider for SharedScript {
            async fn stream(
                &self,
                _: ModelRequest,
                _: CancellationToken,
            ) -> Result<BoxStream<'static, Result<ModelDelta>>> {
                let items = self.0.lock().unwrap().pop_front().unwrap();
                Ok(futures::stream::iter(items).boxed())
            }
        }
        Ok(Arc::new(SharedScript(steps)))
    }
}

struct SwitchTool {
    management: Arc<huginn_core::providers::ProvidersService>,
    target: ModelSelection,
}
#[async_trait]
impl Tool for SwitchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "switch_selection".into(),
            description: "switches provider selection mid-turn".into(),
            input_schema: serde_json::json!({}),
        }
    }
    async fn prepare(&self, _: &ToolCall, _: &ToolContext) -> Result<PreparedAction> {
        Ok(PreparedAction {
            approval: None,
            payload: serde_json::json!({}),
        })
    }
    async fn execute(&self, _: PreparedAction, _: &ToolContext) -> Result<String> {
        self.management
            .0
            .execute(
                ProviderAction::Select(self.target.clone()),
                CancellationToken::new(),
            )
            .await?;
        Ok("switched".into())
    }
}

#[tokio::test]
async fn mid_turn_selection_change_preserves_attribution_to_pinned_model_a() {
    let (mut supervisor, management, catalog, router) = support::start().await;
    let steps_a = Arc::new(Mutex::new(VecDeque::from(vec![
        vec![
            Ok(ModelDelta::ToolCall(ToolCall {
                id: "switch-call".into(),
                name: "switch_selection".into(),
                arguments: serde_json::json!({}),
            })),
            Ok(ModelDelta::Usage {
                input_tokens: Some(30),
                output_tokens: Some(10),
                semantics: UsageSemantics::Cumulative,
            }),
            Ok(ModelDelta::Finished),
        ],
        vec![
            Ok(ModelDelta::Text("finished on A".into())),
            Ok(ModelDelta::Usage {
                input_tokens: Some(20),
                output_tokens: Some(5),
                semantics: UsageSemantics::Cumulative,
            }),
            Ok(ModelDelta::Finished),
        ],
    ])));
    let steps_b = Arc::new(Mutex::new(VecDeque::from(vec![vec![
        Ok(ModelDelta::Text("next turn on B".into())),
        Ok(ModelDelta::Usage {
            input_tokens: Some(9),
            output_tokens: Some(4),
            semantics: UsageSemantics::Cumulative,
        }),
        Ok(ModelDelta::Finished),
    ]])));

    supervisor
        .register(Box::new(support::AdapterPlugin::new(
            "provider-a",
            Arc::new(AdapterWithUsage {
                provider_id: "provider-a".into(),
                model_id: "model-a".into(),
                steps: steps_a,
            }),
        )))
        .unwrap();
    supervisor
        .register(Box::new(support::AdapterPlugin::new(
            "provider-b",
            Arc::new(AdapterWithUsage {
                provider_id: "provider-b".into(),
                model_id: "model-b".into(),
                steps: steps_b,
            }),
        )))
        .unwrap();
    supervisor.start().await.unwrap();

    let adapters = catalog.0.adapters();
    let reg_a = adapters
        .iter()
        .find(|(_, d)| d.id == "provider-a")
        .unwrap()
        .0
        .clone();
    let reg_b = adapters
        .iter()
        .find(|(_, d)| d.id == "provider-b")
        .unwrap()
        .0
        .clone();

    let conn_a = match management
        .0
        .execute(
            ProviderAction::Connect {
                id: "conn-a".into(),
                adapter: reg_a,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap()
    {
        huginn_core::providers::ProviderActionOutput::Connected(id) => id,
        other => panic!("unexpected: {other:?}"),
    };
    let conn_b = match management
        .0
        .execute(
            ProviderAction::Connect {
                id: "conn-b".into(),
                adapter: reg_b,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap()
    {
        huginn_core::providers::ProviderActionOutput::Connected(id) => id,
        other => panic!("unexpected: {other:?}"),
    };

    let sel_a = ModelSelection {
        connection: conn_a,
        model: "model-a".into(),
        reasoning_effort: None,
    };
    let sel_b = ModelSelection {
        connection: conn_b,
        model: "model-b".into(),
        reasoning_effort: None,
    };
    management
        .0
        .execute(ProviderAction::Select(sel_a), CancellationToken::new())
        .await
        .unwrap();

    let switch_tool = Arc::new(SwitchTool {
        management: management.clone(),
        target: sel_b,
    });

    let (engine, _) = make_loop(router.0.clone(), vec![switch_tool], LoopConfig::default());
    let mut session = Session::new(".".into());

    // Turn 1 starts on A; tool switches router selection to B mid-turn.
    engine
        .run(
            &mut session,
            "turn 1".into(),
            &AutoApprove,
            &RecordingSink::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    let turn1_usage = TurnUsage::from_session_events(&session.events).expect("turn 1 usage");
    assert_eq!(turn1_usage.requests, 2);
    assert_eq!(turn1_usage.input_tokens, Some(50));
    assert_eq!(turn1_usage.output_tokens, Some(15));
    assert_eq!(
        turn1_usage.attribution,
        Some(UsageAttribution {
            provider: Some("provider-a".into()),
            connection: Some("conn-a".into()),
            model: "model-a".into(),
        })
    );

    // Turn 2 picks up B and attributes to B.
    engine
        .run(
            &mut session,
            "turn 2".into(),
            &AutoApprove,
            &RecordingSink::default(),
            CancellationToken::new(),
        )
        .await
        .unwrap();

    let turn2_usage = TurnUsage::from_session_events(&session.events).expect("turn 2 usage");
    assert_eq!(turn2_usage.requests, 1);
    assert_eq!(turn2_usage.input_tokens, Some(9));
    assert_eq!(turn2_usage.output_tokens, Some(4));
    assert_eq!(
        turn2_usage.attribution,
        Some(UsageAttribution {
            provider: Some("provider-b".into()),
            connection: Some("conn-b".into()),
            model: "model-b".into(),
        })
    );
}
