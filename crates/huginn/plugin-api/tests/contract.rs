use async_trait::async_trait;
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext, ServiceId, State, Supervisor},
    v1::*,
    AgentError, Result,
};
use std::sync::{Arc, Mutex};

struct Host(Arc<Contributions>);
#[async_trait]
impl Plugin for Host {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "host".into(),
            provides: vec![contributions_id()],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(contributions_id(), UiService(self.0.clone()))
    }
}
struct Author {
    id: &'static str,
    fail: bool,
    held: Arc<Mutex<Vec<Registration>>>,
}
#[async_trait]
impl Plugin for Author {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: self.id.into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        let command = ui.register(
            ctx,
            format!("{}.command", self.id),
            Contribution::Command {
                name: format!("/{}", self.id),
                handler: Arc::new(|args| Ok(CommandResult::Notice(View::text(args)))),
            },
        )?;
        assert!(
            ui.command(&format!("/{}", self.id)).is_none(),
            "staged command is invisible"
        );
        self.held.lock().unwrap().push(command);
        ui.register(
            ctx,
            format!("{}.renderer", self.id),
            Contribution::Renderer {
                tool: self.id.into(),
                handler: Arc::new(|input| Ok(View::text(format!("custom: {}", input.output)))),
            },
        )?;
        ui.register(
            ctx,
            format!("{}.dialog", self.id),
            Contribution::Dialog(Dialog {
                title: "Select".into(),
                body: "Choose".into(),
                choices: vec!["yes".into(), "no".into()],
                on_select: Arc::new(|index| Ok(View::text(index.to_string()))),
            }),
        )?;
        ui.register(
            ctx,
            format!("{}.widget", self.id),
            Contribution::Widget {
                slot: self.id.into(),
                view: View::text("status"),
            },
        )?;
        if self.fail {
            Err(AgentError::Composition("injected".into()))
        } else {
            Ok(())
        }
    }
}
async fn setup(
    fail: bool,
) -> (
    Supervisor,
    Arc<Contributions>,
    Arc<Mutex<Vec<Registration>>>,
) {
    let ui = Arc::new(Contributions::default());
    let held = Arc::new(Mutex::new(vec![]));
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(Host(ui.clone()))).unwrap();
    supervisor.start().await.unwrap();
    supervisor
        .register(Box::new(Author {
            id: "alpha",
            fail,
            held: held.clone(),
        }))
        .unwrap();
    assert_eq!(supervisor.start().await.is_err(), fail);
    (supervisor, ui, held)
}
#[tokio::test]
async fn independent_contributions_dispatch_and_revoke_all_retained_handles() {
    let (mut supervisor, ui, held) = setup(false).await;
    supervisor
        .register(Box::new(Author {
            id: "beta",
            fail: false,
            held: held.clone(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    let command = ui.command("/alpha").unwrap();
    assert_eq!(
        command.run("argument").unwrap(),
        CommandResult::Notice(View::text("argument"))
    );
    let renderer = ui.renderer("alpha").unwrap();
    assert_eq!(
        renderer
            .render(&ToolResult {
                tool: "alpha".into(),
                output: "raw".into(),
                is_error: false
            })
            .unwrap()
            .body,
        "custom: raw"
    );
    assert!(ui.renderer("unknown").is_none());
    let dialog = ui.dialog("alpha.dialog").unwrap();
    assert_eq!(dialog.choose(1).unwrap().body, "1");
    assert!(dialog.choose(2).is_err());
    assert_eq!(ui.widgets().len(), 2);
    supervisor.remove("alpha").await.unwrap();
    assert!(command.run("").is_err());
    assert!(renderer
        .render(&ToolResult {
            tool: "alpha".into(),
            output: "".into(),
            is_error: true
        })
        .is_err());
    assert!(dialog.choose(0).is_err());
    assert!(!held.lock().unwrap()[0].is_active());
    assert_eq!(ui.widgets().len(), 1);
    assert_eq!(
        ui.len(),
        4,
        "physical cleanup, not just visibility filtering"
    );
    drop(supervisor);
    assert_eq!(ui.len(), 0);
}
#[tokio::test]
async fn failed_activation_rolls_back_registrations_even_when_author_retains_handles() {
    let (supervisor, ui, held) = setup(true).await;
    assert_eq!(ui.len(), 0);
    assert!(!held.lock().unwrap()[0].is_active());
    assert!(ui.command("/alpha").is_none());
    drop(supervisor);
}
#[tokio::test]
async fn missing_and_wrong_major_dependencies_block_without_ui() {
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(Author {
            id: "headless",
            fail: false,
            held: Arc::default(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    assert_eq!(supervisor.diagnostics()[0].state, State::Blocked);
    struct WrongMajor;
    #[async_trait]
    impl Plugin for WrongMajor {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "wrong-major".into(),
                provides: vec![ServiceId::new("agent:ui-contributions", 2)],
                requires: vec![],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            ctx.provide(
                ServiceId::new("agent:ui-contributions", 2),
                UiService(Arc::default()),
            )
        }
    }
    supervisor.register(Box::new(WrongMajor)).unwrap();
    supervisor.start().await.unwrap();
    assert_eq!(
        supervisor
            .diagnostics()
            .iter()
            .find(|entry| entry.manifest.id == "headless")
            .unwrap()
            .state,
        State::Blocked
    );
}
#[tokio::test]
async fn collisions_are_explicit_and_failed_second_author_preserves_first() {
    let (mut supervisor, ui, _) = setup(false).await;
    struct Collision(Arc<Contributions>);
    #[async_trait]
    impl Plugin for Collision {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "collision".into(),
                provides: vec![],
                requires: vec![contributions_id()],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            let attempts = vec![
                (
                    "reserved",
                    Contribution::Command {
                        name: "/quit".into(),
                        handler: Arc::new(|_| Ok(CommandResult::Notice(View::text("")))),
                    },
                ),
                (
                    "invalid",
                    Contribution::Command {
                        name: "NoSlash".into(),
                        handler: Arc::new(|_| Ok(CommandResult::Notice(View::text("")))),
                    },
                ),
                (
                    "command",
                    Contribution::Command {
                        name: "/alpha".into(),
                        handler: Arc::new(|_| Ok(CommandResult::Notice(View::text("")))),
                    },
                ),
                (
                    "renderer",
                    Contribution::Renderer {
                        tool: "alpha".into(),
                        handler: Arc::new(|_| Ok(View::text(""))),
                    },
                ),
                (
                    "widget",
                    Contribution::Widget {
                        slot: "alpha".into(),
                        view: View::text(""),
                    },
                ),
                (
                    "alpha.dialog",
                    Contribution::Widget {
                        slot: "other".into(),
                        view: View::text(""),
                    },
                ),
            ];
            for (id, value) in attempts {
                assert!(self.0.register(ctx, id.into(), value).is_err());
            }
            Err(AgentError::Composition("collision probe".into()))
        }
    }
    supervisor
        .register(Box::new(Collision(ui.clone())))
        .unwrap();
    assert!(supervisor.start().await.is_err());
    assert!(ui.command("/alpha").is_some());
    assert_eq!(ui.len(), 4);
}
#[tokio::test]
async fn callback_output_is_sanitized_bounded_and_panics_become_errors() {
    struct Outputs;
    #[async_trait]
    impl Plugin for Outputs {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "outputs".into(),
                provides: vec![],
                requires: vec![contributions_id()],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            let registry = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
            registry.register(
                ctx,
                "output.safe".into(),
                Contribution::Command {
                    name: "/safe".into(),
                    handler: Arc::new(|_| {
                        Ok(CommandResult::Notice(View::text("\x1b[31mhello\u{202E}")))
                    }),
                },
            )?;
            registry.register(
                ctx,
                "output.large".into(),
                Contribution::Command {
                    name: "/large".into(),
                    handler: Arc::new(|_| {
                        Ok(CommandResult::Notice(View::text("x".repeat(65_537))))
                    }),
                },
            )?;
            registry.register(
                ctx,
                "output.panic".into(),
                Contribution::Command {
                    name: "/panic".into(),
                    handler: Arc::new(|_| panic!("injected callback panic")),
                },
            )?;
            registry.register(
                ctx,
                "output.fallback".into(),
                Contribution::Renderer {
                    tool: "*".into(),
                    handler: Arc::new(|_| Ok(View::text("fallback"))),
                },
            )?;
            Ok(())
        }
    }
    let (mut supervisor, ui, _) = setup(false).await;
    supervisor.register(Box::new(Outputs)).unwrap();
    supervisor.start().await.unwrap();
    assert_eq!(
        ui.command("/safe").unwrap().run("").unwrap(),
        CommandResult::Notice(View::text("[31mhello"))
    );
    assert!(ui.command("/large").unwrap().run("").is_err());
    assert!(ui.command("/panic").unwrap().run("").is_err());
    let input = ToolResult {
        tool: "alpha".into(),
        output: "exact".into(),
        is_error: false,
    };
    assert_eq!(
        ui.renderer("alpha").unwrap().render(&input).unwrap().body,
        "custom: exact"
    );
    assert_eq!(
        ui.renderer("other").unwrap().render(&input).unwrap().body,
        "fallback"
    );
}
