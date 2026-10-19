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
        assert!(ui
            .commands()
            .iter()
            .all(|command| command.name != format!("/{}", self.id)));
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
async fn contributions_reject_a_foreign_registry() {
    struct Rogue(Arc<Contributions>);
    #[async_trait]
    impl Plugin for Rogue {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "rogue".into(),
                provides: vec![],
                requires: vec![contributions_id()],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            let rejected = self.0.register(
                ctx,
                "rogue.command".into(),
                Contribution::Command {
                    name: "/rogue".into(),
                    handler: Arc::new(|_| Ok(CommandResult::Notice(View::text("rogue")))),
                },
            );
            assert!(rejected.is_err());
            Err(AgentError::Composition("rogue probe".into()))
        }
    }
    let (mut supervisor, ui, _) = setup(false).await;
    supervisor
        .register(Box::new(Rogue(Arc::default())))
        .unwrap();
    assert!(supervisor.start().await.is_err());
    assert!(ui.command("/rogue").is_none());
    assert!(ui.command("/alpha").is_some());
}
#[tokio::test]
async fn concurrent_recovered_panics_stay_isolated_and_keep_the_hook() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Barrier,
    };
    static HOOK_CALLS: AtomicUsize = AtomicUsize::new(0);
    // Sentinel host hook: observe every panic, then delegate. It stands in for
    // TerminalGuard's hook: recovered callback panics must reach the installed
    // hook (which cooperates via the thread marker), and the hook must still be
    // installed afterwards instead of a stale per-invocation replacement.
    let previous = std::panic::take_hook();
    let previous = Arc::new(Mutex::new(previous));
    std::panic::set_hook(Box::new(move |info| {
        HOOK_CALLS.fetch_add(1, Ordering::SeqCst);
        if let Ok(previous) = previous.lock() {
            previous(info);
        }
    }));
    struct Flood {
        count: usize,
        barrier: Arc<Barrier>,
    }
    #[async_trait]
    impl Plugin for Flood {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "flood".into(),
                provides: vec![],
                requires: vec![contributions_id()],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
            for index in 0..self.count {
                let barrier = self.barrier.clone();
                ui.register(
                    ctx,
                    format!("flood.{index}"),
                    Contribution::Command {
                        name: format!("/flood-{index}"),
                        handler: Arc::new(move |_| {
                            barrier.wait();
                            panic!("flood-payload-{index}")
                        }),
                    },
                )?;
            }
            Ok(())
        }
    }
    const THREADS: usize = 8;
    let (mut supervisor, ui, _) = setup(false).await;
    supervisor
        .register(Box::new(Flood {
            count: THREADS,
            barrier: Arc::new(Barrier::new(THREADS)),
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    let commands: Vec<_> = (0..THREADS)
        .map(|index| ui.command(&format!("/flood-{index}")).unwrap())
        .collect();
    let baseline = HOOK_CALLS.load(Ordering::SeqCst);
    std::thread::scope(|scope| {
        for (index, command) in commands.iter().enumerate() {
            scope.spawn(move || {
                let failure = command.run("").unwrap_err();
                assert!(
                    failure
                        .to_string()
                        .contains(&format!("flood-payload-{index}")),
                    "thread {index} got another thread's report: {failure}"
                );
            });
        }
    });
    // Every recovered panic reached the installed hook, which was never
    // replaced: a stale per-invocation hook would have swallowed observations.
    // (No follow-up run here: the flood handlers rendezvous on a barrier that
    // is spent once all threads have passed it.)
    assert!(HOOK_CALLS.load(Ordering::SeqCst) >= baseline + THREADS);
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
async fn command_catalog_keeps_metadata_and_revokes_old_generations() {
    struct Described;
    #[async_trait]
    impl Plugin for Described {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "described".into(),
                provides: vec![],
                requires: vec![contributions_id()],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
            ui.register_command(
                ctx,
                "described.command".into(),
                "/describe".into(),
                CommandMetadata {
                    description: "Describe\u{202e}".into(),
                    argument_hint: "<format>".into(),
                    arguments: vec!["short".into(), "long".into()],
                    workspace_paths: true,
                },
                Arc::new(|args| Ok(CommandResult::Notice(View::text(args)))),
            )?;
            assert!(!ui
                .commands()
                .iter()
                .any(|command| command.name == "/describe"));
            Ok(())
        }
    }
    let (mut supervisor, ui, _) = setup(false).await;
    assert_eq!(ui.commands()[0].metadata.description, "");
    supervisor.register(Box::new(Described)).unwrap();
    supervisor.start().await.unwrap();
    let old = ui
        .commands()
        .into_iter()
        .find(|command| command.name == "/describe")
        .unwrap();
    assert_eq!(old.metadata.description, "Describe");
    assert_eq!(old.metadata.arguments, ["short", "long"]);
    assert!(old.command.is_active());
    supervisor.remove("described").await.unwrap();
    supervisor.register(Box::new(Described)).unwrap();
    supervisor.start().await.unwrap();
    assert!(!old.command.is_active());
    assert!(old.command.run("short").is_err());
    assert!(ui.command("/describe").unwrap().is_active());
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
async fn recovered_callback_marker_is_thread_scoped_and_cleared() {
    struct Marker {
        seen: Arc<Mutex<Vec<bool>>>,
    }
    #[async_trait]
    impl Plugin for Marker {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "marker".into(),
                provides: vec![],
                requires: vec![contributions_id()],
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
            let seen = self.seen.clone();
            ui.register(
                ctx,
                "marker.command".into(),
                Contribution::Command {
                    name: "/mark".into(),
                    handler: Arc::new(move |_| {
                        seen.lock().unwrap().push(in_recovered_callback());
                        Ok(CommandResult::Notice(View::text("marked")))
                    }),
                },
            )?;
            ui.register(
                ctx,
                "marker.panic".into(),
                Contribution::Command {
                    name: "/mark-panic".into(),
                    handler: Arc::new(|_| panic!("marker panic payload")),
                },
            )?;
            Ok(())
        }
    }
    let (mut supervisor, ui, _) = setup(false).await;
    let seen = Arc::new(Mutex::new(vec![]));
    supervisor
        .register(Box::new(Marker { seen: seen.clone() }))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(!in_recovered_callback());
    ui.command("/mark").unwrap().run("").unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![true]);
    assert!(!in_recovered_callback());
    // Stray host reports outside `invoke` are ignored and never leak.
    capture_recovered_panic("stray".into());
    let failure = ui.command("/mark-panic").unwrap().run("").unwrap_err();
    assert!(
        failure.to_string().contains("marker panic payload"),
        "unexpected report: {failure}"
    );
    assert!(!failure.to_string().contains("stray"));
    assert!(!in_recovered_callback());
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
    let panic = ui.command("/panic").unwrap().run("").unwrap_err();
    assert!(
        panic.to_string().contains("injected callback panic"),
        "caught panic keeps its payload: {panic}"
    );
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
