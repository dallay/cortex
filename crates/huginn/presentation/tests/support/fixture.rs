//! Fault/stream fixture compiled only with the explicit test-driver feature.
use async_trait::async_trait;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext, ServiceId, Supervisor},
    AgentLoop, ApprovalPolicy, CancellationToken, Event, EventSink, LoopService, Message, Result,
    Role, Session,
};
use huginn_plugin_api::v1::{
    contributions_id, CommandMetadata, CommandResult, Contribution, Dialog, UiService, View,
};
use huginn_presentation::{
    contributions::ConversationPlugin, presentation_id, PresentationService, RatatuiPlugin,
};
use std::{sync::Arc, time::Duration};

struct Engine;
#[async_trait]
impl AgentLoop for Engine {
    async fn run(
        &self,
        session: &mut Session,
        prompt: String,
        policy: &dyn ApprovalPolicy,
        events: &dyn EventSink,
        cancel: CancellationToken,
    ) -> Result<()> {
        session
            .messages
            .push(Message::text(Role::User, prompt.clone()));
        if prompt == "panic-now" {
            panic!("uncaught panic after recovered callbacks");
        }
        if prompt == "hold" {
            events.emit(Event::Text {
                text: "STREAM_HELD".into(),
            });
            cancel.cancelled().await;
            session.interrupted = true;
            return Err(huginn_core::AgentError::Cancelled);
        }
        if prompt == "approve" {
            let approved = policy
                .approve(
                    &huginn_core::ApprovalRequest {
                        id: "test".into(),
                        action: "native.edit".into(),
                        preview: std::iter::once("PREVIEW_FIRST".to_owned())
                            .chain((0..40).map(|index| format!("diff-line-{index:02}")))
                            .chain(std::iter::once("PREVIEW_LAST".to_owned()))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    },
                    cancel.clone(),
                )
                .await?;
            session.messages.push(Message::text(
                Role::Assistant,
                if approved { "APPROVED" } else { "DENIED" },
            ));
            return Ok(());
        }
        let gated_resize_stream = std::env::args().nth(1).as_deref() == Some("resize-stream");
        for index in 0..40 {
            let marker = gated_resize_stream && index % 4 == 0;
            tokio::select! {
                _ = cancel.cancelled() => { session.interrupted = true; return Err(huginn_core::AgentError::Cancelled); },
                _ = tokio::time::sleep(Duration::from_millis(20)) => events.emit(Event::Text { text: format!("{}chunk{index} ", if marker { "\n" } else { "" }) }),
            }
            if marker {
                let signal = format!("resize-{index}");
                while !std::path::Path::new(&signal).exists() {
                    tokio::select! {
                        _ = cancel.cancelled() => { session.interrupted = true; return Err(huginn_core::AgentError::Cancelled); },
                        _ = tokio::time::sleep(Duration::from_millis(10)) => {},
                    }
                }
            }
        }
        session
            .messages
            .push(Message::text(Role::Assistant, "STREAM_COMPLETE"));
        events.emit(Event::TurnFinished);
        Ok(())
    }
}
struct EnginePlugin;
struct Replaceable(&'static str);
#[async_trait]
impl Plugin for Replaceable {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fixture.replaceable".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        let notice = self.0;
        ui.register(
            ctx,
            "fixture.describe".into(),
            Contribution::Command {
                name: "/describe".into(),
                handler: Arc::new(move |_| Ok(CommandResult::Notice(View::text(notice)))),
            },
        )?;
        Ok(())
    }
}
/// Test-only 40-choice dialog: overflows the PTY viewport so tests can prove
/// the selected choice stays visible while navigating past the first page.
struct LongDialogPlugin;
#[async_trait]
impl Plugin for LongDialogPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fixture.long-dialog".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        let choices: Vec<String> = (0..40).map(|index| format!("choice-{index:02}")).collect();
        ui.register_command(
            ctx,
            "fixture.format".into(),
            "/format".into(),
            CommandMetadata {
                description: "Example format".into(),
                argument_hint: "<formato>".into(),
                arguments: vec!["json".into(), "text".into()],
                ..Default::default()
            },
            Arc::new(|args| Ok(CommandResult::Notice(View::text(format!("FORMAT:{args}"))))),
        )?;
        let options = choices.clone();
        ui.register(
            ctx,
            "fixture.long-dialog".into(),
            Contribution::Dialog(Dialog {
                title: "Long dialog".into(),
                body: "Navigate past the first page.".into(),
                choices,
                on_select: Arc::new(move |index| {
                    Ok(View::text(format!("Picked {}", options[index])))
                }),
            }),
        )?;
        ui.register(
            ctx,
            "fixture.long-pick".into(),
            Contribution::Command {
                name: "/long-pick".into(),
                handler: Arc::new(|_| Ok(CommandResult::Dialog("fixture.long-dialog".into()))),
            },
        )?;
        let long_body: Vec<String> = (0..30).map(|index| format!("body-{index:02}")).collect();
        let long_body_text = long_body.join("\n");
        ui.register(
            ctx,
            "fixture.long-body-dialog".into(),
            Contribution::Dialog(Dialog {
                title: "Long body dialog".into(),
                body: long_body_text,
                choices: vec!["blong-00".into(), "blong-01".into(), "blong-02".into()],
                on_select: Arc::new(|index| {
                    Ok(View::text(format!(
                        "Picked {}",
                        ["blong-00", "blong-01", "blong-02"][index]
                    )))
                }),
            }),
        )?;
        ui.register(
            ctx,
            "fixture.long-body-pick".into(),
            Contribution::Command {
                name: "/long-body-pick".into(),
                handler: Arc::new(|_| Ok(CommandResult::Dialog("fixture.long-body-dialog".into()))),
            },
        )?;
        Ok(())
    }
}
/// Test-only recovered panic through the normal command-dispatch path.
/// Mirrors the provider onboarding chain: a selector returns a second form.
struct ProviderSelectPlugin;
#[async_trait]
impl Plugin for ProviderSelectPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fixture.provider-select".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        use huginn_plugin_api::v1::{Field, FieldKind, OverlayKind, Workflow};
        ctx.resolve::<UiService>(&contributions_id())?
            .0
            .clone()
            .register(
                ctx,
                "fixture.provider-select".into(),
                Contribution::Command {
                    name: "/provider-select".into(),
                    handler: Arc::new(|_| {
                        // Mirrors /login: an auto-executed workflow returns the
                        // provider selector, whose continuation returns the form.
                        Ok(CommandResult::Workflow(
                            Workflow::new("Providers", "Loading…", vec![], Arc::new(|_, _| {
                                Box::pin(async {
                                    Ok(CommandResult::Workflow(
                                        Workflow::new(
                                            "Connect provider",
                                            "Choose a provider.",
                                            vec![Field {
                                                id: "provider".into(),
                                                label: "Provider".into(),
                                                kind: FieldKind::Choice(vec!["Ollama local".into()]),
                                                initial: String::new(),
                                            }],
                                            Arc::new(|_, _| {
                                                Box::pin(async {
                                                    Ok(CommandResult::Workflow(
                                                        Workflow::new(
                                                            "New connection",
                                                            "Public settings and secrets are separate.",
                                                            vec![
                                                                Field {
                                                                    id: "endpoint".into(),
                                                                    label: "Endpoint".into(),
                                                                    kind: FieldKind::Text,
                                                                    initial: "http://127.0.0.1:11434".into(),
                                                                },
                                                                Field {
                                                                    id: "http".into(),
                                                                    label: "Allow HTTP on literal loopback only".into(),
                                                                    kind: FieldKind::Choice(vec![
                                                                        "No".into(),
                                                                        "Yes, explicitly allow local HTTP".into(),
                                                                    ]),
                                                                    initial: String::new(),
                                                                },
                                                            ],
                                                            Arc::new(|_, _| {
                                                                Box::pin(async {
                                                                    Ok(CommandResult::Notice(View::text(
                                                                        "CONNECTION_SAVED",
                                                                    )))
                                                                })
                                                            }),
                                                        )?
                                                        .with_overlay_kind(OverlayKind::Login)?,
                                                    ))
                                                })
                                            }),
                                        )?
                                        .with_overlay_kind(OverlayKind::Login)?,
                                    ))
                                })
                            }))?
                            .with_overlay_kind(OverlayKind::Login)?,
                        ))
                    }),
                },
            )?;
        Ok(())
    }
}
struct BoomPlugin;
struct BarPlugin(tokio::sync::watch::Sender<huginn_core::providers::ConnectionSnapshot>);
#[async_trait]
impl Plugin for BarPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fixture.bar".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        use huginn_plugin_api::v1::{Field, FieldKind, OverlayKind, Workflow};
        let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        for (name, kind, title) in [
            ("/bar-provider", OverlayKind::Provider, "Bar Provider"),
            ("/bar-login", OverlayKind::Login, "Bar Login"),
            ("/bar-model", OverlayKind::ModelPicker, "Bar Model"),
            ("/bar-confirm", OverlayKind::Confirm, "Bar Confirm"),
        ] {
            let sender = self.0.clone();
            ui.register(
                ctx,
                format!("fixture.{}", name.trim_start_matches('/')),
                Contribution::Command {
                    name: name.into(),
                    handler: Arc::new(move |_| {
                        sender.send_modify(|snapshot| {
                            snapshot.revision += 1;
                            snapshot.selection.as_mut().unwrap().model = title.replace(' ', "-");
                        });
                        Ok(CommandResult::Workflow(
                            Workflow::new(
                                title,
                                "Local fixture, not remote verification",
                                vec![Field {
                                    id: "secret".into(),
                                    label: "Credential".into(),
                                    kind: FieldKind::Secret,
                                    initial: String::new(),
                                }],
                                Arc::new(|_, _| {
                                    Box::pin(async {
                                        Ok(CommandResult::Notice(View::text("done")))
                                    })
                                }),
                            )?
                            .with_overlay_kind(kind)?,
                        ))
                    }),
                },
            )?;
        }
        Ok(())
    }
}
struct SecretFormPlugin;
#[async_trait]
impl Plugin for SecretFormPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fixture.secret-form".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        use huginn_plugin_api::v1::{Field, FieldKind, Workflow};
        let cancellation = ctx.cancellation.clone();
        ctx.resolve::<UiService>(&contributions_id())?
            .0
            .clone()
            .register(
                ctx,
                "fixture.secret-form".into(),
                Contribution::Command {
                    name: "/secret-form".into(),
                    handler: Arc::new(move |_| {
                        let cancellation = cancellation.clone();
                        Ok(CommandResult::Workflow(Workflow::new(
                            "Secret form fixture",
                            "Host-owned input",
                            vec![
                                Field {
                                    id: "key".into(),
                                    label: "Credential".into(),
                                    kind: FieldKind::Secret,
                                    initial: String::new(),
                                },
                                Field {
                                    id: "mode".into(),
                                    label: "Mode".into(),
                                    kind: FieldKind::Choice(vec![
                                        "wait".into(),
                                        "fail".into(),
                                        "panic".into(),
                                        "invalidate".into(),
                                    ]),
                                    initial: String::new(),
                                },
                            ],
                            Arc::new(move |input, _| {
                                let cancellation = cancellation.clone();
                                Box::pin(async move {
                                    assert!(input.secrets.contains_key("key"));
                                    match input.public["mode"].as_str() {
                                        "1" => {
                                            return Err(huginn_core::AgentError::Composition(
                                                "secret redacted".into(),
                                            ))
                                        }
                                        "2" => panic!("secret redacted"),
                                        "3" => {
                                            cancellation.cancel();
                                            std::future::pending::<()>().await;
                                        }
                                        _ => std::future::pending::<()>().await,
                                    }
                                    Ok(CommandResult::Notice(View::text("unexpected completion")))
                                })
                            }),
                        )?))
                    }),
                },
            )?;
        Ok(())
    }
}
#[async_trait]
impl Plugin for BoomPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fixture.boom".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        ui.register(
            ctx,
            "fixture.boom".into(),
            Contribution::Command {
                name: "/boom".into(),
                handler: Arc::new(|_| panic!("recovered boom for PTY")),
            },
        )?;
        Ok(())
    }
}
#[async_trait]
impl Plugin for EnginePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "fixture.loop".into(),
            provides: vec![ServiceId::new("agent:loop", 1)],
            requires: vec![],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        ctx.provide(
            ServiceId::new("agent:loop", 1),
            LoopService(Arc::new(Engine)),
        )
    }
}
#[tokio::main]
async fn main() -> Result<()> {
    let mut supervisor = Supervisor::default();
    supervisor.register(Box::new(EnginePlugin))?;
    let bar = std::env::args().nth(1).as_deref() == Some("bar");
    let terminal = if bar {
        use huginn_core::providers::*;
        let (sender, receiver) = tokio::sync::watch::channel(ConnectionSnapshot {
            source: uuid::Uuid::new_v4(),
            revision: 0,
            phase: ConnectionPhase::Unconfigured,
            selection: None,
        });
        // A cloned receiver with an already-unread update must initialize from latest.
        let terminal = RatatuiPlugin::default().with_connection(receiver);
        sender.send_modify(|snapshot| {
            snapshot.revision = 1;
            snapshot.phase = ConnectionPhase::Selected;
            snapshot.selection = Some(SelectionSnapshot {
                provider: "fixture".into(),
                connection: "fixture".into(),
                model: "alpha".into(),
                mode: SelectionMode::Managed(huginn_core::connections::SecretMode::Ephemeral),
            });
        });
        supervisor.register(Box::new(BarPlugin(sender)))?;
        terminal
    } else {
        RatatuiPlugin::default()
    };
    supervisor.register(Box::new(terminal))?;
    supervisor.register(Box::new(ConversationPlugin))?;
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some("replace" | "replace-approval")
    ) {
        supervisor.register(Box::new(Replaceable("OLD_COMMAND_EXECUTED")))?;
    }
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some("extension" | "bar")
    ) {
        if std::env::args().nth(1).as_deref() == Some("extension") {
            supervisor.register(Box::new(huginn_reference_extension::ReferenceExtension))?;
        }
        supervisor.register(Box::new(LongDialogPlugin))?;
    }
    if std::env::args().nth(1).as_deref() == Some("panic-after-callback") {
        supervisor.register(Box::new(BoomPlugin))?;
    }
    if std::env::args().nth(1).as_deref() == Some("provider-select") {
        supervisor.register(Box::new(ProviderSelectPlugin))?;
    }
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some("secret-form" | "secret-unload" | "secret-refresh")
    ) {
        supervisor.register(Box::new(SecretFormPlugin))?;
    }
    supervisor.start().await?;
    let mut session = Session::new(".".into());
    let presentation = supervisor.resolve::<PresentationService>(&presentation_id())?;
    let connection = presentation.0.open(&session).await?;
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some("secret-unload" | "secret-refresh")
    ) {
        while !std::path::Path::new("invalidate-secret").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        supervisor.remove("fixture.secret-form").await?;
        if std::env::args().nth(1).as_deref() == Some("secret-refresh") {
            supervisor.register(Box::new(SecretFormPlugin))?;
            supervisor.start().await?;
        }
    }
    if matches!(
        std::env::args().nth(1).as_deref(),
        Some("replace" | "replace-approval")
    ) {
        while !std::path::Path::new("replace-plugin").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        supervisor.remove("fixture.replaceable").await?;
        supervisor.register(Box::new(Replaceable("NEW_COMMAND_EXECUTED")))?;
        supervisor.start().await?;
        connection.ui.notification("REPLACEMENT_READY".into());
        if std::env::args().nth(1).as_deref() == Some("replace-approval") {
            while !std::path::Path::new("show-approval").exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            connection.ui.begin(CancellationToken::new());
            connection
                .ui
                .request_decision(
                    &huginn_core::ApprovalRequest {
                        id: "modal-probe".into(),
                        action: "native.edit".into(),
                        preview: "MODAL_PROBE".into(),
                    },
                    CancellationToken::new(),
                )
                .await?;
            connection.ui.synchronize(&session);
            connection.ui.notification("APPROVAL_CLOSED".into());
        }
    }
    if std::env::args().nth(1).as_deref() == Some("drop-reopen") {
        let ui = connection.ui.clone();
        assert_eq!(ui.prompt().await.as_deref(), Some("reopen"));
        drop(connection);
        assert!(!crossterm::terminal::is_raw_mode_enabled()?);
        let next = presentation.0.open(&session).await?;
        next.ui.notification("NEW_GENERATION_READY".into());
        assert_eq!(next.ui.prompt().await.as_deref(), Some("replacement input"));
        next.finish().await?;
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("drop") {
        drop(connection);
        assert!(
            !crossterm::terminal::is_raw_mode_enabled()?,
            "connection Drop must synchronously restore raw mode"
        );
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("unload") {
        supervisor.remove("presentation.ratatui").await?;
        assert!(
            !crossterm::terminal::is_raw_mode_enabled()?,
            "unload must complete terminal cleanup"
        );
        supervisor.register(Box::new(RatatuiPlugin::default()))?;
        supervisor.start().await?;
        let next = supervisor
            .resolve::<PresentationService>(&presentation_id())?
            .0
            .open(&session)
            .await?;
        next.finish().await?;
        drop(connection);
        return Ok(());
    }
    if std::env::args().nth(1).as_deref() == Some("panic") {
        panic!("intentional PTY restoration fixture");
    }
    let ui = connection.ui.clone();
    let policy = huginn_application::authorization::HostPolicy::new(ui.clone(), Default::default());
    while let Some(prompt) = ui.prompt().await {
        let cancel = CancellationToken::new();
        ui.begin(cancel.clone());
        let result = Engine
            .run(&mut session, prompt, &policy, ui.as_ref(), cancel)
            .await;
        ui.synchronize(&session);
        if let Err(error) = result {
            ui.notification(error.to_string());
        }
    }
    connection.finish().await?;
    supervisor.shutdown().await;
    Ok(())
}
