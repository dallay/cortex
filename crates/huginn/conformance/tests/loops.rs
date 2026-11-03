//! Certification: two distinct loops must pass the same suite, and a broken one must not.

use async_trait::async_trait;
use huginn_application::{composition, loop_engine::StandardLoop};
use huginn_conformance::{broken::ReplayingLoop, hook_plugin, run, Capabilities, UnderTest};
use huginn_core::{
    kernel::Supervisor, AgentLoop, ApprovalPolicy, CancellationToken, EventSink, LoopService,
    Result, Session,
};
use std::sync::Arc;

/// Keeps the supervisor alive for as long as the loop it published, and forwards.
struct Supervised {
    _supervisor: Arc<Supervisor>,
    inner: Arc<dyn AgentLoop>,
}
#[async_trait]
impl AgentLoop for Supervised {
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
    UnderTest::new("StandardLoop", Capabilities::FULL, |host| {
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

/// The production adapter: a leased loop published by the composition kernel.
fn leased() -> UnderTest {
    UnderTest::new("LeasedAgentLoop", Capabilities::FULL, |host| {
        Box::pin(async move {
            let mut supervisor = composition::build(
                host.script.clone(),
                host.tools(),
                host.sessions.clone(),
                host.loop_config(),
                host.instructions(),
            )
            .await
            .unwrap();
            // The composition kernel owns the hook registry: a supervisor-based
            // implementation must be given the host hook here, not through the config.
            if let Some(hook) = host.hook() {
                supervisor.register(hook_plugin(hook)).unwrap();
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
    UnderTest::new("ReplayingLoop", Capabilities::FULL, |host| {
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
