use async_trait::async_trait;
use huginn_core::CancellationToken;
use huginn_plugin_api::v1::View;
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    v1::*,
    Result,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[test]
fn overlay_metadata_preserves_workflow_payload_and_registration() {
    let workflow = Workflow::new(
        "Provider login",
        "Enter credentials",
        vec![Field {
            id: "token".into(),
            label: "Token".into(),
            kind: FieldKind::Secret,
            initial: String::new(),
        }],
        Arc::new(|_, _| Box::pin(async { Ok(CommandResult::Notice(View::text("ok"))) })),
    )
    .unwrap();
    assert_eq!(workflow.overlay_kind(), None);
    let tagged = workflow
        .clone()
        .with_overlay_kind(OverlayKind::Login)
        .unwrap();
    assert_eq!(tagged.overlay_kind(), Some(OverlayKind::Login));
    assert_eq!(tagged.title(), workflow.title());
    assert_eq!(tagged.fields(), workflow.fields());
}

#[test]
fn public_choice_defaults_are_validated_without_allowing_secret_defaults() {
    let make = |kind, initial: &str| {
        Workflow::new(
            "Form",
            "",
            vec![Field {
                id: "value".into(),
                label: "Value".into(),
                kind,
                initial: initial.into(),
            }],
            Arc::new(|_, _| Box::pin(async { Ok(CommandResult::Notice(View::text("done"))) })),
        )
    };
    for initial in ["", "0", "1"] {
        assert!(make(
            FieldKind::Choice(vec!["first".into(), "second".into()]),
            initial
        )
        .is_ok());
    }
    for initial in ["2", "-1", "second"] {
        assert!(make(
            FieldKind::Choice(vec!["first".into(), "second".into()]),
            initial
        )
        .is_err());
    }
    assert!(make(FieldKind::Secret, "1").is_err());
    assert!(make(FieldKind::Secret, "").is_ok());
}

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
    calls: Arc<AtomicUsize>,
    entered: Arc<tokio::sync::Notify>,
    fail: bool,
}
#[async_trait]
impl Plugin for Author {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "author".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        let calls = self.calls.clone();
        let entered = self.entered.clone();
        ui.register(
            ctx,
            "author.action".into(),
            Contribution::Command {
                name: "/action".into(),
                handler: Arc::new(move |_| {
                    let calls = calls.clone();
                    let entered = entered.clone();
                    Ok(CommandResult::Workflow(Workflow::new(
                        "Action",
                        "Wait",
                        vec![],
                        Arc::new(move |_, _| {
                            calls.fetch_add(1, Ordering::SeqCst);
                            let entered = entered.clone();
                            Box::pin(async move {
                                entered.notify_one();
                                std::future::pending::<()>().await;
                                Ok(CommandResult::Notice(View::text("late")))
                            })
                        }),
                    )?))
                }),
            },
        )?;
        if self.fail {
            return Err(huginn_core::AgentError::Composition("injected".into()));
        }
        Ok(())
    }
}
#[tokio::test]
async fn declarative_action_does_not_execute_in_callback_and_old_generation_cannot_execute_replacement(
) {
    let ui = Arc::new(Contributions::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(Host(ui.clone()))).unwrap();
    supervisor
        .register(Box::new(Author {
            calls: calls.clone(),
            entered: entered.clone(),
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    let CommandResult::Workflow(workflow) = ui.command("/action").unwrap().run("").unwrap() else {
        panic!()
    };
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let action = workflow.clone();
    let pending = tokio::spawn(async move {
        action
            .execute(FormInput::default(), CancellationToken::new())
            .await
    });
    entered.notified().await;
    supervisor.remove("author").await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), pending)
            .await
            .unwrap()
            .unwrap()
            .is_err()
    );
    supervisor
        .register(Box::new(Author {
            calls: calls.clone(),
            entered,
            fail: false,
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(workflow
        .execute(FormInput::default(), CancellationToken::new())
        .await
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(ui.command("/action").unwrap().is_active());
}

#[tokio::test]
async fn failed_activation_rolls_back_async_contribution_without_executing_it() {
    let ui = Arc::new(Contributions::default());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(Host(ui.clone()))).unwrap();
    supervisor.start().await.unwrap();
    supervisor
        .register(Box::new(Author {
            calls: calls.clone(),
            entered: Arc::default(),
            fail: true,
        }))
        .unwrap();
    assert!(supervisor.start().await.is_err());
    assert!(ui.command("/action").is_none());
    assert!(ui.is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn secret_defaults_and_duplicate_fields_are_rejected() {
    let handler: WorkflowHandler =
        Arc::new(|_, _| Box::pin(async { Ok(CommandResult::Notice(View::text("ok"))) }));
    let secret = Field {
        id: "key".into(),
        label: "API key".into(),
        kind: FieldKind::Secret,
        initial: "sentinel".into(),
    };
    assert!(Workflow::new("Form", "", vec![secret.clone()], handler.clone()).is_err());
    let public = Field {
        initial: String::new(),
        kind: FieldKind::Text,
        ..secret
    };
    assert!(Workflow::new("Form", "", vec![public.clone(), public], handler).is_err());
}

struct ResultsAuthor {
    handler: CommandHandler,
    registrations: Arc<std::sync::Mutex<Vec<Registration>>>,
}
#[async_trait]
impl Plugin for ResultsAuthor {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "results".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        for (id, name) in [("results.first", "/first"), ("results.second", "/second")] {
            let registration = ui.register(
                ctx,
                id.into(),
                Contribution::Command {
                    name: name.into(),
                    handler: self.handler.clone(),
                },
            )?;
            self.registrations.lock().unwrap().push(registration);
        }
        Ok(())
    }
}

async fn results_host(
    handler: CommandHandler,
) -> (
    Supervisor,
    Arc<Contributions>,
    Arc<std::sync::Mutex<Vec<Registration>>>,
) {
    let ui = Arc::new(Contributions::default());
    let registrations = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(Host(ui.clone()))).unwrap();
    supervisor
        .register(Box::new(ResultsAuthor {
            handler,
            registrations: registrations.clone(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    (supervisor, ui, registrations)
}

#[tokio::test]
async fn workflow_result_validation_is_exhaustive_and_notices_are_checked() {
    let (_supervisor, ui, _) = results_host(Arc::new(|argument| {
        let result = match argument {
            "help" => CommandResult::Overlay(OverlayKind::Help),
            "settings" => CommandResult::Overlay(OverlayKind::Settings),
            "provider" => CommandResult::Overlay(OverlayKind::Provider),
            "login" => CommandResult::Overlay(OverlayKind::Login),
            "model" => CommandResult::Overlay(OverlayKind::ModelPicker),
            "confirm" => CommandResult::Overlay(OverlayKind::Confirm),
            "dialog" => CommandResult::Dialog("results.dialog".into()),
            "large" => CommandResult::Notice(View::text("x".repeat(65_537))),
            _ => CommandResult::Notice(View::text("safe\u{202e}\x1b")),
        };
        Ok(CommandResult::Workflow(Workflow::new(
            "Action",
            "",
            vec![],
            Arc::new(move |_, _| {
                let result = result.clone();
                Box::pin(async move { Ok(result) })
            }),
        )?))
    }))
    .await;
    for (argument, allowed) in [
        ("help", true),
        ("settings", true),
        ("provider", false),
        ("login", false),
        ("model", false),
        ("confirm", false),
        ("dialog", false),
        ("large", false),
        ("notice", true),
    ] {
        let CommandResult::Workflow(workflow) =
            ui.command("/first").unwrap().run(argument).unwrap()
        else {
            panic!()
        };
        let result = workflow
            .execute(FormInput::default(), CancellationToken::new())
            .await;
        assert_eq!(result.is_ok(), allowed, "{argument}: {result:?}");
        if argument == "notice" {
            assert_eq!(result.unwrap(), CommandResult::Notice(View::text("safe")));
        }
    }
}

#[tokio::test]
async fn tagged_continuation_keeps_own_metadata_payload_callback_and_exact_registration() {
    let calls = Arc::new(AtomicUsize::new(0));
    let callback_calls = calls.clone();
    let next = Workflow::new(
        "Choose model",
        "Original body",
        vec![
            Field {
                id: "model".into(),
                label: "Model".into(),
                kind: FieldKind::Text,
                initial: "default".into(),
            },
            Field {
                id: "token".into(),
                label: "Token".into(),
                kind: FieldKind::Secret,
                initial: String::new(),
            },
        ],
        Arc::new(move |input, _| {
            callback_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(input.public["model"], "chosen");
            assert_eq!(input.secrets["token"].expose(), "overlay-secret-sentinel");
            assert!(!input.public.contains_key("token"));
            Box::pin(async { Ok(CommandResult::Notice(View::text("selected"))) })
        }),
    )
    .unwrap()
    .with_overlay_kind(OverlayKind::ModelPicker)
    .unwrap();
    let expected = next.clone();
    let (mut supervisor, ui, registrations) = results_host(Arc::new(move |_| {
        let next = next.clone();
        Ok(CommandResult::Workflow(
            Workflow::new(
                "Login",
                "",
                vec![],
                Arc::new(move |_, _| {
                    let next = next.clone();
                    Box::pin(async move { Ok(CommandResult::Workflow(next)) })
                }),
            )?
            .with_overlay_kind(OverlayKind::Login)?,
        ))
    }))
    .await;
    let CommandResult::Workflow(first) = ui.command("/first").unwrap().run("").unwrap() else {
        panic!()
    };
    let CommandResult::Workflow(next) = first
        .execute(FormInput::default(), CancellationToken::new())
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(next, expected);
    assert_eq!(first.overlay_kind(), Some(OverlayKind::Login));
    assert_eq!(next.overlay_kind(), Some(OverlayKind::ModelPicker));
    assert_eq!(next.body(), "Original body");
    let retagged = next
        .clone()
        .with_overlay_kind(OverlayKind::Confirm)
        .unwrap();
    assert!(retagged.is_active());
    assert_eq!(retagged.fields(), next.fields());
    assert_eq!(next.overlay_kind(), Some(OverlayKind::ModelPicker));
    let input = || FormInput {
        public: [("model".into(), "chosen".into())].into(),
        secrets: [(
            "token".into(),
            huginn_core::connections::Secret::new("overlay-secret-sentinel".into()).unwrap(),
        )]
        .into(),
    };
    assert!(next
        .execute(input(), CancellationToken::new())
        .await
        .is_ok());
    supervisor.remove("results").await.unwrap();
    assert!(!registrations.lock().unwrap()[0].is_active());
    supervisor
        .register(Box::new(ResultsAuthor {
            handler: Arc::new(|_| Ok(CommandResult::Notice(View::text("replacement")))),
            registrations: Arc::default(),
        }))
        .unwrap();
    supervisor.start().await.unwrap();
    assert!(ui.command("/second").unwrap().is_active());
    assert!(next
        .execute(input(), CancellationToken::new())
        .await
        .is_err());
    assert!(retagged
        .execute(input(), CancellationToken::new())
        .await
        .is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn workflow_metadata_builder_only_accepts_payload_backed_surfaces() {
    let workflow = Workflow::new(
        "Form",
        "",
        vec![],
        Arc::new(|_, _| Box::pin(async { Ok(CommandResult::Notice(View::text("ok"))) })),
    )
    .unwrap();
    for kind in [
        OverlayKind::Provider,
        OverlayKind::Login,
        OverlayKind::ModelPicker,
        OverlayKind::Confirm,
    ] {
        assert_eq!(
            workflow
                .clone()
                .with_overlay_kind(kind)
                .unwrap()
                .overlay_kind(),
            Some(kind)
        );
    }
    for kind in [OverlayKind::Help, OverlayKind::Settings] {
        assert!(workflow.clone().with_overlay_kind(kind).is_err());
    }
    assert_eq!(workflow.overlay_kind(), None);
}

#[tokio::test]
async fn every_payload_backed_surface_binds_and_untagged_continuation_stays_untagged() {
    for kind in [
        OverlayKind::Provider,
        OverlayKind::Login,
        OverlayKind::ModelPicker,
        OverlayKind::Confirm,
    ] {
        let next = Workflow::new(
            "Next",
            "",
            vec![],
            Arc::new(|_, _| Box::pin(async { Ok(CommandResult::Notice(View::text("done"))) })),
        )
        .unwrap();
        let (_supervisor, ui, _) = results_host(Arc::new(move |_| {
            let next = next.clone();
            Ok(CommandResult::Workflow(
                Workflow::new(
                    "Form",
                    "",
                    vec![],
                    Arc::new(move |_, _| {
                        let next = next.clone();
                        Box::pin(async move { Ok(CommandResult::Workflow(next)) })
                    }),
                )?
                .with_overlay_kind(kind)?,
            ))
        }))
        .await;
        let CommandResult::Workflow(first) = ui.command("/first").unwrap().run("").unwrap() else {
            panic!()
        };
        assert_eq!(first.overlay_kind(), Some(kind));
        assert!(first.is_active());
        let CommandResult::Workflow(next) = first
            .execute(FormInput::default(), CancellationToken::new())
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(next.overlay_kind(), None);
        assert_eq!(
            next.execute(FormInput::default(), CancellationToken::new())
                .await
                .unwrap(),
            CommandResult::Notice(View::text("done"))
        );
    }
}
