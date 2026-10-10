use async_trait::async_trait;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, ServiceId, State, Supervisor},
    AgentLoop, ApprovalPolicy, CancellationToken, EventSink, LoopService, Result, Session,
};
use huginn_presentation::{
    contributions::{Contribution, Contributions, ConversationPlugin, Lease},
    contributions_id, presentation_id, PresentationService, RatatuiPlugin,
};
use std::sync::{Arc, Mutex};

struct EmptyLoop;
#[async_trait]
impl AgentLoop for EmptyLoop {
    async fn run(
        &self,
        _: &mut Session,
        _: String,
        _: &dyn ApprovalPolicy,
        _: &dyn EventSink,
        _: CancellationToken,
    ) -> Result<()> {
        Ok(())
    }
}
struct LoopPlugin;
#[async_trait]
impl Plugin for LoopPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "test.loop".into(),
            provides: vec![ServiceId::new("agent:loop", 1)],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(
            ServiceId::new("agent:loop", 1),
            LoopService(Arc::new(EmptyLoop)),
        )
    }
}
#[tokio::test]
async fn missing_loop_blocks_terminal_plugin_without_opening_stdin() {
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(RatatuiPlugin::default()))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(supervisor
        .resolve::<PresentationService>(&presentation_id())
        .is_err());
    assert_eq!(supervisor.diagnostics()[0].state, State::Blocked);
}
#[tokio::test]
async fn contribution_registration_rejects_invalid_reserved_and_duplicate_commands() {
    struct ProbePlugin {
        id: &'static str,
        command: &'static str,
        outcomes: Arc<Mutex<Vec<bool>>>,
        hold: Arc<Mutex<Vec<Lease>>>,
    }
    #[async_trait]
    impl Plugin for ProbePlugin {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: self.id.into(),
                provides: vec![],
                requires: vec![contributions_id()],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            let registry = ctx.resolve::<Arc<Contributions>>(&contributions_id())?;
            let attempt = |command: &str| {
                registry.register(
                    ctx,
                    Contribution {
                        command: command.into(),
                        help: "probe".into(),
                        tool_label: "Probe".into(),
                    },
                )
            };
            let mut local = Vec::new();
            // Malformed names must fail without touching the registry.
            // `/help-me` is malformed because `-` is not ASCII lowercase.
            for bad in [
                "help",
                "/",
                "/Help",
                "/help-me",
                "/quit",
                "/exit",
                "/compact",
                "/summarize",
            ] {
                local.push(attempt(bad).is_err());
            }
            // A syntactically valid registration succeeds and is held so the
            // entry survives for the cross-generation collision below.
            match attempt(self.command) {
                Ok(lease) => {
                    local.push(true);
                    self.hold.lock().unwrap().push(lease);
                }
                Err(_) => local.push(false),
            }
            // The same generation registering again is a duplicate even with
            // a different, syntactically valid command name.
            local.push(attempt("/probetwo").is_err());
            self.outcomes.lock().unwrap().extend(local);
            Ok(())
        }
    }
    // `test.probe-b` activates after `test.probe-a` (plugin id order) and
    // re-attempts the already-taken `/probe` from its own generation, which
    // must fail on the command-name collision branch.
    struct CollisionPlugin {
        outcomes: Arc<Mutex<Vec<bool>>>,
    }
    #[async_trait]
    impl Plugin for CollisionPlugin {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "test.probe-b".into(),
                provides: vec![],
                requires: vec![contributions_id()],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            let registry = ctx.resolve::<Arc<Contributions>>(&contributions_id())?;
            let taken = registry
                .register(
                    ctx,
                    Contribution {
                        command: "/probe".into(),
                        help: "probe".into(),
                        tool_label: "Probe".into(),
                    },
                )
                .is_err();
            self.outcomes.lock().unwrap().push(taken);
            Ok(())
        }
    }
    let outcomes = Arc::new(Mutex::new(Vec::new()));
    let hold = Arc::new(Mutex::new(Vec::new()));
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(LoopPlugin)).unwrap();
    supervisor
        .register(Box::new(RatatuiPlugin::default()))
        .unwrap();
    supervisor
        .register(Box::new(ProbePlugin {
            id: "test.probe-a",
            command: "/probe",
            outcomes: outcomes.clone(),
            hold: hold.clone(),
        }))
        .unwrap();
    supervisor
        .register(Box::new(CollisionPlugin {
            outcomes: outcomes.clone(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    assert_eq!(
        *outcomes.lock().unwrap(),
        vec![true; 11],
        "invalid/reserved names rejected, valid accepted, same-generation duplicate and cross-generation collision rejected"
    );
    supervisor.shutdown().await;
}
#[tokio::test]
async fn unload_invalidates_command_renderer_and_old_presentation_generation() {
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(LoopPlugin)).unwrap();
    supervisor
        .register(Box::new(RatatuiPlugin::default()))
        .unwrap();
    supervisor
        .register(Box::new(ConversationPlugin::default()))
        .unwrap();
    supervisor.start().await.unwrap();
    let registry = supervisor
        .resolve::<Arc<Contributions>>(&contributions_id())
        .unwrap();
    assert!(registry.command("/help").is_some());
    assert_eq!(registry.tool_label(), "Tool output");
    supervisor
        .remove("presentation.conversation")
        .await
        .unwrap();
    assert!(registry.command("/help").is_none());
    assert_eq!(registry.tool_label(), "Tool result");
    supervisor
        .register(Box::new(ConversationPlugin::default()))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(registry.command("/help").is_some());
    let presentation = supervisor
        .resolve::<PresentationService>(&presentation_id())
        .unwrap();
    supervisor.remove("presentation.ratatui").await.unwrap();
    assert!(registry.command("/help").is_none());
    assert!(presentation
        .0
        .open(&Session::new(".".into()), Default::default())
        .await
        .is_err());
    supervisor.shutdown().await;
}
