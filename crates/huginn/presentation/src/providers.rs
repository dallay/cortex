//! Optional Providers author. The host renders forms and runs asynchronous work.
use async_trait::async_trait;
use huginn_core::{
    connections::{
        connections_id, ConnectionError, ConnectionManagement, ConnectionRecord,
        ConnectionSettings, ConnectionsService, SecretMode,
    },
    kernel::{Manifest, Plugin, PluginContext, ServiceId},
    providers::{
        AuthenticationMethod, ProviderCatalog, ProviderCatalogService, ProviderDescriptor,
    },
    Result,
};
use huginn_plugin_api::v1::{
    contributions_id, CommandMetadata, CommandResult, Field, FieldKind, FormInput, OverlayKind,
    UiService, View, Workflow,
};
use std::sync::Arc;

pub struct ProvidersUiPlugin;
#[derive(Clone)]
struct Management {
    connections: Arc<dyn ConnectionManagement>,
    catalog: Arc<dyn ProviderCatalog>,
}
fn catalog_id() -> ServiceId {
    ServiceId::new("agent:provider-catalog", 1)
}
#[async_trait]
impl Plugin for ProvidersUiPlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "builtin.providers.ui".into(),
            provides: vec![],
            requires: vec![contributions_id(), connections_id(), catalog_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        let management = Management {
            connections: ctx
                .resolve::<ConnectionsService>(&connections_id())?
                .0
                .clone(),
            catalog: ctx
                .resolve::<ProviderCatalogService>(&catalog_id())?
                .0
                .clone(),
        };
        for (name, description) in [
            (
                "/login",
                "Connect a provider using a host-owned secret form",
            ),
            ("/providers", "Manage connections and credential status"),
            ("/model", "Choose the model for the next turn"),
            (
                "/logout",
                "Revoke a connection; remote completion may be unknown",
            ),
        ] {
            let management = management.clone();
            ui.register_command(ctx, format!("builtin.providers.ui.{}", &name[1..]), name.into(),
                CommandMetadata { description: description.into(), ..CommandMetadata::default() },
                Arc::new(move |args| {
                    if !args.is_empty() {
                        return Ok(notice("Use the host form, not command arguments. Never type credentials in commands."));
                    }
                    let management = management.clone();
                    Ok(CommandResult::Workflow(Workflow::new("Providers", "Loading…", vec![], Arc::new(move |_, _| {
                        let management = management.clone();
                        Box::pin(async move {
                            if name == "/login" { management.login() } else { management.connections(name).await }
                        })
                    }))?.with_overlay_kind(command_surface(name))?))
                }))?;
        }
        Ok(())
    }
}
fn command_surface(command: &str) -> OverlayKind {
    match command {
        "/login" => OverlayKind::Login,
        "/providers" => OverlayKind::Provider,
        "/model" => OverlayKind::ModelPicker,
        "/logout" => OverlayKind::Confirm,
        _ => unreachable!("registered provider command"),
    }
}
fn notice(text: impl Into<String>) -> CommandResult {
    CommandResult::Notice(View::text(text))
}
fn failure(error: ConnectionError) -> CommandResult {
    CommandResult::Notice(View {
        title: "Connection action failed".into(),
        body: format!(
            "{error}. Reopen /providers to check actual status; durable completion may be unknown."
        ),
        is_error: true,
    })
}
fn field(id: &str, label: &str, kind: FieldKind, initial: impl Into<String>) -> Field {
    Field {
        id: id.into(),
        label: label.into(),
        kind,
        initial: initial.into(),
    }
}
fn choice(id: &str, label: &str, options: Vec<String>) -> Field {
    field(id, label, FieldKind::Choice(options), "")
}
fn index(input: &FormInput, id: &str) -> usize {
    input
        .public
        .get(id)
        .and_then(|v| v.parse().ok())
        .unwrap_or(usize::MAX)
}
fn value(input: &mut FormInput, id: &str) -> String {
    input.public.remove(id).unwrap_or_default()
}
impl Management {
    fn authentication_diagnostics(&self) -> String {
        self.catalog
            .adapters()
            .into_iter()
            .map(|(_, descriptor)| {
                let available: Vec<_> = descriptor
                    .authentication
                    .iter()
                    .filter_map(|method| match method {
                        AuthenticationMethod::ApiKey => Some("API key"),
                        AuthenticationMethod::None => Some("no authentication"),
                        AuthenticationMethod::OAuth => None,
                    })
                    .collect();
                format!(
                    "{} — OAuth: unsupported in Huginn (no verified interactive local flow). Available: {}.",
                    descriptor.display_name,
                    if available.is_empty() { "none".into() } else { available.join(", ") }
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    fn login(&self) -> Result<CommandResult> {
        let diagnostics = self.authentication_diagnostics();
        let options: Vec<_> = self
            .catalog
            .adapters()
            .into_iter()
            .flat_map(|(_, d)| {
                d.authentication
                    .clone()
                    .into_iter()
                    .filter(|a| !matches!(a, AuthenticationMethod::OAuth))
                    .map(move |a| (d.clone(), a))
            })
            .collect();
        if options.is_empty() {
            if !diagnostics.is_empty() {
                return Ok(notice(format!(
                    "No supported login methods.\n{diagnostics}"
                )));
            }
            return Ok(notice(
                "No provider adapters available. The Providers core still works headless.",
            ));
        }
        let labels = options
            .iter()
            .map(|(d, a)| {
                format!(
                    "{} — {}",
                    d.display_name,
                    if *a == AuthenticationMethod::ApiKey {
                        "API key"
                    } else {
                        "no authentication"
                    }
                )
            })
            .collect();
        let management = self.clone();
        Ok(CommandResult::Workflow(
            Workflow::new(
                "Connect provider",
                format!("Choose a provider/authentication method.\n{diagnostics}"),
                vec![choice("provider", "Provider", labels)],
                Arc::new(move |input, _| {
                    let management = management.clone();
                    let selected = options.get(index(&input, "provider")).cloned();
                    Box::pin(async move {
                        let Some((descriptor, authentication)) = selected else {
                            return Ok(failure(ConnectionError::InvalidConfiguration));
                        };
                        management.connection_form(
                            descriptor,
                            authentication == AuthenticationMethod::ApiKey,
                            None,
                        )
                    })
                }),
            )?
            .with_overlay_kind(OverlayKind::Login)?,
        ))
    }
    fn connection_form(
        &self,
        descriptor: ProviderDescriptor,
        requires_secret: bool,
        previous: Option<ConnectionRecord>,
    ) -> Result<CommandResult> {
        let mut fields = vec![field(
            "endpoint",
            "Endpoint (TLS or explicitly allowed literal loopback)",
            FieldKind::Text,
            previous
                .as_ref()
                .and_then(|r| r.settings.endpoint.clone())
                .unwrap_or_default(),
        )];
        let default_model = previous
            .as_ref()
            .map(|r| r.settings.model.clone())
            .unwrap_or_default();
        if descriptor.models.is_empty() {
            fields.push(field(
                "model",
                "Model ID (candidate; validated at inference)",
                FieldKind::Text,
                default_model,
            ));
        } else {
            fields.push(choice(
                "model",
                "Model",
                descriptor.models.iter().map(|m| m.id.clone()).collect(),
            ));
            fields.last_mut().unwrap().initial = descriptor
                .models
                .iter()
                .position(|m| m.id == default_model)
                .unwrap_or(0)
                .to_string();
        }
        fields.push(choice(
            "http",
            "Allow HTTP on literal loopback only",
            vec!["No".into(), "Yes, explicitly allow local HTTP".into()],
        ));
        fields.last_mut().unwrap().initial = usize::from(
            previous
                .as_ref()
                .is_some_and(|r| r.settings.allow_loopback_http),
        )
        .to_string();
        if requires_secret {
            fields.push(choice(
                "storage",
                "Credential storage",
                vec![
                    "Secure OS store (no automatic fallback)".into(),
                    "Ephemeral — lost on restart".into(),
                ],
            ));
            fields.last_mut().unwrap().initial = usize::from(
                previous
                    .as_ref()
                    .is_some_and(|r| r.mode == SecretMode::Ephemeral),
            )
            .to_string();
            fields.push(field("credential", "API key", FieldKind::Secret, ""));
        }
        let management = self.clone();
        let title = if previous.is_some() {
            "Update connection (rotates authorization)"
        } else {
            "New connection"
        };
        Ok(CommandResult::Workflow(Workflow::new(title,
            "Public settings and secrets are separate. Esc discards unsubmitted values. Updating revokes existing turns; select again afterward.", fields,
            Arc::new(move |mut input, _| {
                let management = management.clone();
                let descriptor = descriptor.clone();
                let previous = previous.clone();
                Box::pin(async move {
                    let model = if descriptor.models.is_empty() { value(&mut input, "model") } else {
                        descriptor.models.get(index(&input, "model")).map(|m| m.id.clone()).unwrap_or_default()
                    };
                    let allow_loopback_http = index(&input, "http") == 1;
                    let mode = if requires_secret && index(&input, "storage") == 1 { SecretMode::Ephemeral } else { SecretMode::Secure };
                    let endpoint = value(&mut input, "endpoint");
                    let settings = ConnectionSettings { provider: descriptor.id, endpoint: (!endpoint.is_empty()).then_some(endpoint), model, requires_secret, allow_loopback_http };
                    let secret = input.secrets.remove("credential");
                    match management.connections.save(previous.map(|r| r.id), settings, secret, mode).await {
                        Ok(_) => Ok(notice("Connection saved. Use /model to select it for the next turn; no restart needed.")),
                        Err(error) => Ok(failure(error)),
                    }
                })
            }))?.with_overlay_kind(OverlayKind::Login)?))
    }
    async fn connections(&self, command: &'static str) -> Result<CommandResult> {
        if let Err(error) = self.connections.recover().await {
            return Ok(failure(error));
        }
        let records = match self.connections.list().await {
            Ok(records) => records,
            Err(error) => return Ok(failure(error)),
        };
        if records.is_empty() {
            return Ok(notice(format!(
                "No connections. Use /login to connect a provider, then /model.{}",
                if command == "/providers" {
                    format!("\n{}", self.authentication_diagnostics())
                } else {
                    String::new()
                }
            )));
        }
        if records.len() > 256 {
            return Ok(notice(
                "Too many connections for this UI; manage connections headless.",
            ));
        }
        let mut labels = Vec::new();
        for record in &records {
            let status = match self.connections.status(record.id).await {
                Ok(()) => "credentials ready (remote availability not checked)".into(),
                Err(error) => error.to_string(),
            };
            labels.push(format!(
                "{} · {} · {} · {:?} · {status}",
                record.id, record.settings.provider, record.settings.model, record.mode
            ));
        }
        let mut fields = vec![choice("connection", "Connection", labels)];
        if command == "/providers" {
            fields.push(choice(
                "action",
                "Action",
                vec![
                    "Select model for next turn".into(),
                    "Update settings/credentials".into(),
                    "Disconnect".into(),
                ],
            ));
        }
        if command == "/logout" {
            fields.push(choice(
                "confirm",
                "Confirm revocation",
                vec![
                    "Keep connection".into(),
                    "Disconnect and revoke locally".into(),
                ],
            ));
        }
        let management = self.clone();
        let diagnostics = if command == "/providers" {
            format!("\n{}", self.authentication_diagnostics())
        } else {
            String::new()
        };
        Ok(CommandResult::Workflow(Workflow::new("Providers", format!("Selection affects the NEXT turn, not the model pinned in an active turn. Revocation blocks subsequent inference even in that turn. Local cancellation does not prove remote revocation.{diagnostics}"), fields,
            Arc::new(move |input, _| {
                let management = management.clone();
                let record = records.get(index(&input, "connection")).cloned();
                Box::pin(async move {
                    let Some(record) = record else { return Ok(failure(ConnectionError::InvalidSelection)); };
                    if command == "/logout" {
                        if index(&input, "confirm") != 1 { return Ok(notice("Connection kept.")); }
                        return Ok(match management.connections.revoke(record.id).await {
                            Ok(()) => notice("Connection revoked locally. Next inference is blocked, including in a pinned turn. Remote completion remains unknown."),
                            Err(error) => failure(error),
                        });
                    }
                    if command == "/providers" {
                        match index(&input, "action") {
                            1 => {
                                let Some((_, descriptor)) = management.catalog.adapters().into_iter().find(|(_, d)| d.id == record.settings.provider) else { return Ok(failure(ConnectionError::AdapterUnavailable)); };
                                return management.connection_form(descriptor, record.settings.requires_secret, Some(record));
                            }
                            2 => return management.revoke_form(record),
                            _ => {}
                        }
                    }
                    management.model_form(record)
                })
            }))?.with_overlay_kind(command_surface(command))?))
    }
    fn revoke_form(&self, record: ConnectionRecord) -> Result<CommandResult> {
        let management = self.clone();
        Ok(CommandResult::Workflow(Workflow::new("Disconnect", "Revoke local authorization and delete stored credentials. This does not revoke a remote API key.",
            vec![choice("confirm", "Confirm", vec!["Keep connection".into(), "Disconnect".into()])], Arc::new(move |input, _| {
                let management = management.clone();
                let id = record.id;
                Box::pin(async move {
                    if index(&input, "confirm") != 1 { return Ok(notice("Connection kept.")); }
                    Ok(match management.connections.revoke(id).await { Ok(()) => notice("Connection revoked locally. Remote completion remains unknown."), Err(error) => failure(error) })
                })
            }))?.with_overlay_kind(OverlayKind::Confirm)?))
    }
    fn model_form(&self, record: ConnectionRecord) -> Result<CommandResult> {
        let Some((_, descriptor)) = self
            .catalog
            .adapters()
            .into_iter()
            .find(|(_, d)| d.id == record.settings.provider)
        else {
            return Ok(failure(ConnectionError::AdapterUnavailable));
        };
        let field = if descriptor.models.is_empty() {
            field(
                "model",
                "Model ID (candidate)",
                FieldKind::Text,
                record.settings.model.clone(),
            )
        } else {
            choice(
                "model",
                "Model",
                descriptor.models.iter().map(|m| m.id.clone()).collect(),
            )
        };
        let management = self.clone();
        Ok(CommandResult::Workflow(Workflow::new("Next-turn model", "The current turn keeps its pinned connection/model. No remote availability is promised.", vec![field], Arc::new(move |mut input, _| {
            let management = management.clone();
            let descriptor = descriptor.clone();
            let id = record.id;
            Box::pin(async move {
                let model = if descriptor.models.is_empty() { value(&mut input, "model") } else { descriptor.models.get(index(&input, "model")).map(|m| m.id.clone()).unwrap_or_default() };
                Ok(match management.connections.select_model(id, model).await {
                    Ok(_) => notice("Model selected for the NEXT turn. An active turn keeps its pinned model; no restart needed."),
                    Err(error) => failure(error),
                })
            })
        }))?.with_overlay_kind(OverlayKind::ModelPicker)?))
    }
}
