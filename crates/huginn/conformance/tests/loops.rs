//! Certification: two distinct loops must pass the same suite, and a broken one must not.

use async_trait::async_trait;
use huginn_application::{composition, loop_engine::StandardLoop};
use huginn_conformance::{
    broken::ReplayingLoop, hook_plugin, inference_hook_plugin, run, UnderTest,
};
use huginn_core::{
    kernel::{SubstitutionPolicy, Supervisor},
    AgentLoop, ApprovalPolicy, CancellationToken, EventSink, LoopService, Result, Session,
};
use huginn_reference_extension::step_loop::{step_loop_plugin, StepLoopConfig};
use std::sync::Arc;
use std::time::Duration;

/// Keeps the supervisor alive for as long as the loop it published, and forwards.
struct Supervised {
    _supervisor: Arc<Supervisor>,
    inner: Arc<dyn AgentLoop>,
}
#[async_trait]
impl AgentLoop for Supervised {
    fn capabilities(&self) -> huginn_core::AgentLoopCapabilities {
        self.inner.capabilities()
    }

    async fn skill_commands(
        &self,
        workspace: &std::path::Path,
        cancellation: CancellationToken,
    ) -> Result<Vec<(String, String)>> {
        self.inner.skill_commands(workspace, cancellation).await
    }
    async fn run(
        &self,
        session: &mut Session,
        prompt: String,
        approvals: &dyn ApprovalPolicy,
        events: &dyn EventSink,
        cancellation: CancellationToken,
    ) -> Result<()> {
        self.inner
            .run(session, prompt, approvals, events, cancellation)
            .await
    }
    async fn compact(
        &self,
        session: &mut Session,
        events: &dyn EventSink,
        cancellation: CancellationToken,
    ) -> Result<()> {
        self.inner.compact(session, events, cancellation).await
    }
}

fn standard() -> UnderTest {
    UnderTest::new("StandardLoop", |host| {
        Box::pin(async move {
            let implementation = StandardLoop {
                model: host.script.clone(),
                tools: host.tools(),
                sessions: host.sessions.clone(),
                instructions: host.instructions(),
                config: host.loop_config(),
                lifecycle: CancellationToken::new(),
            };
            Arc::new(implementation) as Arc<dyn AgentLoop>
        })
    })
}

#[tokio::test]
async fn standard_skill_entrypoints_reject_during_compaction_and_release_on_drop() {
    use futures::FutureExt;
    use huginn_conformance::{Events, Host, Reply};
    use huginn_core::{AgentError, Message, Role};

    let host = Host::new(vec![Reply::Hang]).await;
    let implementation = StandardLoop {
        model: host.script.clone(),
        tools: host.tools(),
        sessions: host.sessions.clone(),
        instructions: host.instructions(),
        config: host.loop_config(),
        lifecycle: CancellationToken::new(),
    };
    let mut session = Session::new(host.workspace.clone());
    session.messages = vec![
        Message::text(Role::User, "request"),
        Message::text(Role::Assistant, "answer"),
    ];
    let mut clone = session.clone();
    let before = serde_json::to_value(&clone).unwrap();
    let events = Events::default();
    let approval = host.approval(true);
    let mut compact =
        Box::pin(implementation.compact(&mut session, &events, CancellationToken::new()));
    assert!(futures::poll!(&mut compact).is_pending());
    assert_eq!(host.script.calls(), 1);
    let saves = host.sessions.saves();
    for explicit in [false, true] {
        let result = if explicit {
            implementation
                .run_with_skills(
                    &mut clone,
                    "blocked".into(),
                    &["missing".into()],
                    &*approval,
                    &events,
                    CancellationToken::new(),
                )
                .now_or_never()
        } else {
            implementation
                .run(
                    &mut clone,
                    "/skill:missing blocked".into(),
                    &*approval,
                    &events,
                    CancellationToken::new(),
                )
                .now_or_never()
        };
        assert!(matches!(result, Some(Err(AgentError::Session(ref message)))
            if message == "session already has an active in-process operation (capacity 1; no waiting queue)"));
    }
    assert_eq!(serde_json::to_value(&clone).unwrap(), before);
    assert_eq!(host.script.calls(), 1);
    assert_eq!(host.sessions.saves(), saves);
    assert!(events.snapshot().is_empty());
    drop(compact);
    implementation
        .run_with_skills(
            &mut session,
            "after drop".into(),
            &[],
            &*approval,
            &events,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    // An ordinary error also releases admission.
    let mut empty = Session::new(host.workspace.clone());
    assert!(implementation
        .compact(&mut empty, &events, CancellationToken::new())
        .await
        .is_err());
    implementation
        .run(
            &mut empty,
            "after error".into(),
            &*approval,
            &events,
            CancellationToken::new(),
        )
        .await
        .unwrap();
}

/// A store whose first save waits, so admission can be probed during final persistence.
#[derive(Default)]
struct PausingSessions {
    saving: tokio::sync::Notify,
    release: tokio::sync::Notify,
    saves: std::sync::atomic::AtomicUsize,
}
#[async_trait]
impl huginn_core::SessionStore for PausingSessions {
    async fn save(&self, _: &Session) -> Result<()> {
        if self.saves.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            self.saving.notify_one();
            self.release.notified().await;
        }
        Ok(())
    }
    async fn load(&self, id: &str) -> Result<Session> {
        Err(huginn_core::AgentError::Session(format!("no session {id}")))
    }
    async fn list(&self) -> Result<Vec<huginn_core::SessionInfo>> {
        Ok(vec![])
    }
}

#[tokio::test]
async fn admission_is_held_through_final_persistence() {
    use futures::FutureExt;
    use huginn_conformance::{Events, Host, Reply};
    use huginn_core::AgentError;

    let host = Host::new(vec![Reply::Text("answer")]).await;
    let sessions = Arc::new(PausingSessions::default());
    let implementation = StandardLoop {
        model: host.script.clone(),
        tools: host.tools(),
        sessions: sessions.clone(),
        instructions: host.instructions(),
        config: host.loop_config(),
        lifecycle: CancellationToken::new(),
    };
    let mut session = Session::new(host.workspace.clone());
    let mut overlapping = session.clone();
    let approval = host.approval(true);
    let events = Events::default();
    let mut run = Box::pin(implementation.run(
        &mut session,
        "hi".into(),
        &*approval,
        &events,
        CancellationToken::new(),
    ));
    tokio::select! {
        result = &mut run => panic!("the turn finished before its final save: {result:?}"),
        _ = sessions.saving.notified() => {}
        _ = tokio::time::sleep(Duration::from_secs(1)) => panic!("the turn never persisted"),
    }
    // The final save is still in flight, so admission must still be held.
    let result = implementation
        .run(
            &mut overlapping,
            "overlap".into(),
            &*approval,
            &events,
            CancellationToken::new(),
        )
        .now_or_never();
    assert!(matches!(result, Some(Err(AgentError::Session(ref message)))
        if message == "session already has an active in-process operation (capacity 1; no waiting queue)"));
    sessions.release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), run)
        .await
        .expect("the released turn must finish")
        .expect("the turn must succeed");

    // Completion released admission for the same id, not just for a fresh one.
    implementation
        .run(
            &mut session,
            "after".into(),
            &*approval,
            &events,
            CancellationToken::new(),
        )
        .await
        .unwrap();
}

/// The production adapter: a leased loop published by the composition kernel.
fn leased() -> UnderTest {
    UnderTest::new("LeasedAgentLoop", |host| {
        Box::pin(async move {
            let mut supervisor = composition::build(
                host.script.clone(),
                host.tools(),
                host.sessions.clone(),
                host.kernel_config(),
                host.instructions(),
            )
            .await
            .unwrap();
            // The composition kernel owns the hook registry: a supervisor-based
            // implementation must be given the host hook here, not through the config.
            let tool_hook = host.hook();
            let inference_hook = host.inference_hook();
            if let Some(hook) = &tool_hook {
                supervisor.register(hook_plugin(hook.clone())).unwrap();
            }
            if let Some(hook) = &inference_hook {
                supervisor
                    .register(inference_hook_plugin(hook.clone()))
                    .unwrap();
            }
            if tool_hook.is_some() || inference_hook.is_some() {
                supervisor.start().await.unwrap();
            }
            let inner = supervisor
                .resolve::<LoopService>(&composition::loop_id())
                .unwrap()
                .0
                .clone();
            Arc::new(Supervised {
                _supervisor: Arc::new(supervisor),
                inner,
            }) as Arc<dyn AgentLoop>
        })
    })
}

fn replaying() -> UnderTest {
    UnderTest::new("ReplayingLoop", |host| {
        Box::pin(async move {
            let implementation = ReplayingLoop::new(host.script.clone(), host.tools());
            Arc::new(implementation) as Arc<dyn AgentLoop>
        })
    })
}

#[tokio::test]
async fn standard_loop_passes_the_suite() {
    let report = run(&standard()).await;
    assert!(
        report.failures().is_empty(),
        "StandardLoop must pass every case:\n{}",
        report.describe()
    );
    assert!(report.passed("compaction_keeps_turn_boundary"));
    assert!(
        report.passed("context_is_pinned_for_the_turn"),
        "a case silently missing from the run is not a pass:\n{}",
        report.describe()
    );
    assert_eq!(
        report.outcomes.len(),
        17,
        "the suite must run every case:\n{}",
        report.describe()
    );
}

#[tokio::test]
async fn production_leased_loop_passes_the_same_suite() {
    let report = run(&leased()).await;
    assert!(
        report.failures().is_empty(),
        "the production loop must pass every case:\n{}",
        report.describe()
    );
}

#[tokio::test]
async fn a_loop_that_replays_effects_fails_the_suite() {
    let report = run(&replaying()).await;
    assert!(
        !report.failures().is_empty(),
        "a loop that replays effects must not be certified:\n{}",
        report.describe()
    );
    assert!(report.failed("concurrent_session_admission_is_bounded"));
    assert!(
        report.failed("effect_unknown_is_not_replayed"),
        "the suite must catch the replay:\n{}",
        report.describe()
    );
    assert!(
        report.failed("denial_performs_no_effect"),
        "the suite must catch an effect taken without the host decision:\n{}",
        report.describe()
    );
}

/// A loop authored only against the plugin contracts, installed by substitution.
fn extension_step_loop() -> UnderTest {
    UnderTest::new("StepLoop (extension)", |host| {
        Box::pin(async move {
            let mut supervisor = composition::build(
                host.script.clone(),
                host.tools(),
                host.sessions.clone(),
                host.kernel_config(),
                host.instructions(),
            )
            .await
            .unwrap();
            let plugin = step_loop_plugin(
                host.script.clone(),
                host.tools(),
                host.sessions.clone(),
                host.instructions(),
                StepLoopConfig {
                    max_iterations: host.config.max_iterations,
                    max_output_tokens: host.config.max_output_tokens,
                    hooks: host.loop_config().hooks,
                    inference_hooks: host.loop_config().inference_hooks,
                    steering: host.loop_config().steering,
                    max_turn_tokens: host.config.max_turn_tokens,
                    max_model_retries: host.config.max_model_retries,
                },
            );
            supervisor
                .substitute(
                    &composition::loop_id(),
                    plugin,
                    SubstitutionPolicy::ReportUndrained,
                    Duration::from_secs(1),
                )
                .await
                .unwrap();
            let inner = supervisor
                .resolve::<LoopService>(&composition::loop_id())
                .unwrap()
                .0
                .clone();
            Arc::new(Supervised {
                _supervisor: Arc::new(supervisor),
                inner,
            }) as Arc<dyn AgentLoop>
        })
    })
}

#[tokio::test]
async fn extension_loop_passes_the_same_suite() {
    let report = run(&extension_step_loop()).await;
    assert!(
        report.failures().is_empty(),
        "a loop authored outside the host crate must pass every case:\n{}",
        report.describe()
    );
}

struct DefaultCapabilitiesLoop(ReplayingLoop);
#[async_trait]
impl AgentLoop for DefaultCapabilitiesLoop {
    async fn run(
        &self,
        session: &mut Session,
        prompt: String,
        approvals: &dyn ApprovalPolicy,
        events: &dyn EventSink,
        cancellation: CancellationToken,
    ) -> Result<()> {
        self.0
            .run(session, prompt, approvals, events, cancellation)
            .await
    }
}

#[tokio::test]
async fn optional_cases_read_the_built_loops_default_declaration() {
    let candidate = UnderTest::new("default capabilities", |host| {
        Box::pin(async move {
            Arc::new(DefaultCapabilitiesLoop(ReplayingLoop::new(
                host.script.clone(),
                host.tools(),
            ))) as Arc<dyn AgentLoop>
        })
    });
    let report = run(&candidate).await;
    let compaction = report
        .outcomes
        .iter()
        .find(|case| case.case == "compaction_keeps_turn_boundary")
        .unwrap();
    assert_eq!(compaction.status, huginn_conformance::Status::Skipped);
    assert_eq!(
        compaction.detail,
        "the implementation does not declare compaction"
    );
    assert!(run(&replaying())
        .await
        .failed("compaction_keeps_turn_boundary"));
}

/// Discovery must inspect configuration only, never call the loader.
struct UnreachableSkills;
#[async_trait]
impl huginn_application::skills::SkillLoader for UnreachableSkills {
    async fn catalog(
        &self,
        _: &std::path::Path,
        _: CancellationToken,
    ) -> Result<huginn_application::skills::SkillCatalog> {
        panic!("capability discovery must not load a catalog")
    }
    async fn load(
        &self,
        _: &std::path::Path,
        _: &str,
        _: &[String],
        _: CancellationToken,
    ) -> Result<String> {
        panic!("capability discovery must not load a skill")
    }
}

#[tokio::test]
async fn implementations_declare_configured_capabilities_without_host_access() {
    use huginn_conformance::Host;
    use huginn_core::AgentLoopCapabilities;

    let host = Host::new(vec![]).await;
    let extension = huginn_reference_extension::step_loop::StepLoop::new(
        host.script.clone(),
        host.tools(),
        host.sessions.clone(),
        host.instructions(),
        StepLoopConfig::default(),
    );
    assert_eq!(
        extension.capabilities(),
        AgentLoopCapabilities {
            compaction: true,
            skill_commands: false,
        }
    );
    for configured in [false, true] {
        let mut config = host.kernel_config();
        if configured {
            config.skills = Some(Arc::new(UnreachableSkills));
        }
        let direct = StandardLoop {
            model: host.script.clone(),
            tools: host.tools(),
            sessions: host.sessions.clone(),
            instructions: host.instructions(),
            config: config.clone(),
            lifecycle: CancellationToken::new(),
        };
        let mut supervisor = composition::build(
            host.script.clone(),
            host.tools(),
            host.sessions.clone(),
            config,
            host.instructions(),
        )
        .await
        .unwrap();
        let implementation = supervisor
            .resolve::<LoopService>(&composition::loop_id())
            .unwrap()
            .0
            .clone();
        assert_eq!(direct.capabilities().skill_commands, configured);
        assert_eq!(implementation.capabilities(), direct.capabilities());
        supervisor.shutdown().await;
        // Discovery still works after providers are revoked: it needs no leases.
        assert_eq!(implementation.capabilities(), direct.capabilities());
        let wrapped = Supervised {
            _supervisor: Arc::new(supervisor),
            inner: implementation,
        };
        assert_eq!(wrapped.capabilities(), direct.capabilities());
        assert_eq!(host.script.calls(), 0);
        assert_eq!(host.sessions.saves(), 0);
    }
}
