//! Optional Providers author. The host renders forms and runs asynchronous work.
use async_trait::async_trait;
use huginn_core::{
    connections::{
        connections_id, ConnectionError, ConnectionManagement, ConnectionMaterial,
        ConnectionRecord, ConnectionSettings, ConnectionsService, SecretMode,
    },
    kernel::{Manifest, Plugin, PluginContext, ServiceId},
    providers::{
        AuthenticationMethod, ModelCatalog, ProviderCatalog, ProviderCatalogService,
        ProviderDescriptor, ProviderRegistration,
    },
    Result,
};
use huginn_plugin_api::v1::{
    contributions_id, CommandMetadata, CommandResult, Field, FieldKind, FormInput, OverlayKind,
    UiService, View, Workflow,
};
use std::sync::{Arc, Mutex};

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
                "Search providers and connect with transient host-owned forms",
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
/// Trailing slashes do not change the official endpoint.
fn same_endpoint(left: &str, right: Option<&str>) -> bool {
    right.is_some_and(|right| left.trim_end_matches('/') == right.trim_end_matches('/'))
}
/// One actionable reason at a time, so the user knows exactly what to change.
fn connection_problem(settings: &ConnectionSettings, discover: bool) -> Option<String> {
    use huginn_core::connections::EndpointProblem;
    if let Some(problem) = huginn_core::connections::endpoint_problem(settings) {
        return Some(
            match problem {
                EndpointProblem::Malformed => "The endpoint is not a valid URL.",
                EndpointProblem::RemoteHttp => "HTTP is only allowed for a literal loopback address such as 127.0.0.1. Use HTTPS for any other host.",
                EndpointProblem::LoopbackHttpDisabled => "Local HTTP is disabled. Set Allow HTTP on literal loopback only to Yes, then submit again.",
            }
            .into(),
        );
    }
    if !discover
        && !settings.model.is_empty()
        && !huginn_core::connections::valid_identifier(&settings.model)
    {
        return Some(
            "The model ID may only contain letters, digits and the characters - _ . : /".into(),
        );
    }
    if settings.validate().is_err() {
        return Some("The endpoint is not valid. Use HTTPS, or explicitly allow HTTP for a literal loopback address.".into());
    }
    None
}
fn form_error(body: impl Into<String>) -> CommandResult {
    CommandResult::Notice(View {
        title: "Connection error".into(),
        body: body.into(),
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
fn provider_labels(options: &[(ProviderRegistration, ProviderDescriptor)]) -> Vec<String> {
    let mut totals = std::collections::HashMap::new();
    for (_, descriptor) in options {
        *totals
            .entry(descriptor.display_name.as_str())
            .or_insert(0_usize) += 1;
    }
    options
        .iter()
        .map(|(_, descriptor)| {
            if totals[descriptor.display_name.as_str()] > 1 {
                format!("{} ({})", descriptor.display_name, descriptor.id)
            } else {
                descriptor.display_name.clone()
            }
        })
        .collect()
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
        let options = self.catalog.adapters();
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
        let labels = provider_labels(&options);
        let management = self.clone();
        Ok(CommandResult::Workflow(
            Workflow::new(
                "Connect a provider — Step 1 of 4",
                format!("Search the installed provider catalog.\n{diagnostics}"),
                vec![choice("provider", "Provider", labels)],
                Arc::new(move |input, _| {
                    let management = management.clone();
                    let selected = options.get(index(&input, "provider")).cloned();
                    Box::pin(async move {
                        let Some((registration, descriptor)) = selected else {
                            return Ok(failure(ConnectionError::InvalidConfiguration));
                        };
                        if let Err(error) = management.connections.recover().await {
                            return Ok(failure(error));
                        }
                        let records = match management.connections.list().await {
                            Ok(records) => records,
                            Err(error) => return Ok(failure(error)),
                        };
                        if records.iter().any(|r| r.settings.provider == descriptor.id) {
                            management.existing_provider(registration, descriptor, records)
                        } else {
                            management.authentication_form(registration, descriptor)
                        }
                    })
                }),
            )?
            .searchable()
            .with_overlay_kind(OverlayKind::Login)?,
        ))
    }
    fn existing_provider(
        &self,
        registration: ProviderRegistration,
        descriptor: ProviderDescriptor,
        records: Vec<ConnectionRecord>,
    ) -> Result<CommandResult> {
        let owner = self.clone();
        let summary = records
            .iter()
            .filter(|r| r.settings.provider == descriptor.id)
            .map(|r| {
                format!(
                    "{} · {:?} · {}",
                    r.id,
                    r.mode,
                    r.settings.endpoint.as_deref().unwrap_or("default endpoint")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(CommandResult::Workflow(
            Workflow::new(
                "Provider already configured",
                format!(
                    "{}\n{summary}\nChoose explicitly; adding never overwrites a connection.",
                    descriptor.display_name
                ),
                vec![choice(
                    "action",
                    "Action",
                    vec![
                        "Manage existing connections".into(),
                        "Add another connection".into(),
                    ],
                )],
                Arc::new(move |input, _| {
                    let owner = owner.clone();
                    let registration = registration.clone();
                    let descriptor = descriptor.clone();
                    Box::pin(async move {
                        if index(&input, "action") == 0 {
                            owner.connections("/providers").await
                        } else {
                            owner.authentication_form(registration, descriptor)
                        }
                    })
                }),
            )?
            .with_overlay_kind(OverlayKind::Login)?,
        ))
    }
    fn authentication_form(
        &self,
        registration: ProviderRegistration,
        descriptor: ProviderDescriptor,
    ) -> Result<CommandResult> {
        let authentication = self
            .catalog
            .onboarding(&registration)?
            .authentication
            .unwrap_or_else(|| descriptor.authentication.clone());
        let methods: Vec<_> = authentication
            .iter()
            .filter(|a| !matches!(a, AuthenticationMethod::OAuth))
            .cloned()
            .collect();
        if methods.is_empty() {
            return Ok(CommandResult::Workflow(Workflow::new(
                "Unsupported authentication — Step 2 of 4",
                format!("{}: OAuth: unsupported. No implemented API-key or no-authentication method is available. Esc returns; Ctrl+C cancels. No connection was created.", descriptor.display_name),
                vec![choice("action", "Action", vec!["Back to provider catalog".into()])],
                { let owner = self.clone(); Arc::new(move |_, _| {
                    // The explicit action returns to the catalog as the first
                    // step. Without this the host would inherit the current
                    // history and stack another catalog on it.
                    let result = owner.login().map(|result| match result {
                        CommandResult::Workflow(workflow) => {
                            CommandResult::Workflow(workflow.without_history())
                        }
                        other => other,
                    });
                    Box::pin(async move { result })
                }) },
            )?.with_overlay_kind(OverlayKind::Login)?));
        }
        let owner = self.clone();
        Ok(CommandResult::Workflow(
            Workflow::new(
                "Authentication method — Step 2 of 4",
                format!(
                    "{}: choose authentication for this connection.",
                    descriptor.display_name
                ),
                vec![choice(
                    "authentication",
                    "Authentication",
                    methods
                        .iter()
                        .map(|a| match a {
                            AuthenticationMethod::ApiKey => "API key".into(),
                            _ => "No authentication".into(),
                        })
                        .collect(),
                )],
                Arc::new(move |input, _| {
                    let result = methods.get(index(&input, "authentication")).map_or_else(
                        || Ok(notice("Invalid authentication choice")),
                        |auth| {
                            owner.connection_form(
                                registration.clone(),
                                descriptor.clone(),
                                *auth == AuthenticationMethod::ApiKey,
                                None,
                            )
                        },
                    );
                    Box::pin(async move { result })
                }),
            )?
            .with_overlay_kind(OverlayKind::Login)?,
        ))
    }
    fn connection_form(
        &self,
        registration: ProviderRegistration,
        descriptor: ProviderDescriptor,
        requires_secret: bool,
        previous: Option<ConnectionRecord>,
    ) -> Result<CommandResult> {
        let onboarding = self.catalog.onboarding(&registration)?;
        let mut fields = vec![field(
            "endpoint",
            "Endpoint (TLS or explicitly allowed literal loopback)",
            FieldKind::Text,
            previous
                .as_ref()
                .and_then(|r| r.settings.endpoint.clone())
                .or_else(|| onboarding.default_endpoint.clone())
                .unwrap_or_default(),
        )];
        let default_model = previous
            .as_ref()
            .map(|r| r.settings.model.clone())
            .unwrap_or_default();
        if descriptor.models.is_empty() {
            fields.push(field(
                "model",
                "Model ID (optional; discover if empty)",
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
            "Connection settings — Step 3 of 4"
        };
        Ok(CommandResult::Workflow(Workflow::new(title,
            "Public settings and secrets are separate. Leave Model ID empty to discover models. Cloud uses its fixed official endpoint. Esc goes back and clears secrets; Ctrl+C cancels. After saving select with /model.", fields,
            Arc::new(move |mut input, cancel| {
                let management = management.clone();
                let descriptor = descriptor.clone();
                let previous = previous.clone();
                let onboarding = onboarding.clone();
                let registration = registration.clone();
                Box::pin(async move {
                    let model = if descriptor.models.is_empty() { value(&mut input, "model") } else {
                        descriptor.models.get(index(&input, "model")).map(|m| m.id.clone()).unwrap_or_default()
                    };
                    let allow_loopback_http = index(&input, "http") == 1;
                    let mode = if requires_secret && index(&input, "storage") == 1 { SecretMode::Ephemeral } else { SecretMode::Secure };
                    let endpoint = value(&mut input, "endpoint");
                    if !onboarding.allow_endpoint_edit && !same_endpoint(&endpoint, onboarding.default_endpoint.as_deref()) {
                        return Ok(form_error("This provider requires its official HTTPS endpoint. No request was sent."));
                    }
                    let discover = model.is_empty();
                    let settings = ConnectionSettings { provider: descriptor.id, endpoint: (!endpoint.is_empty()).then_some(endpoint), model: if discover { "discovery-placeholder".into() } else { model }, requires_secret, allow_loopback_http };
                    let secret = input.secrets.remove("credential");
                    if requires_secret && secret.is_none() && previous.is_none() { return Ok(form_error("An API key is required for this connection.")); }
                    let material = ConnectionMaterial { settings, secret };
                    if previous.is_some() && discover { return Ok(form_error("A model ID is required when updating. Use /login to discover models for a new connection.")); }
                    // The same precise diagnosis applies to new and updated connections.
                    if let Some(problem) = connection_problem(&material.settings, discover) {
                        return Ok(form_error(problem));
                    }
                    // Updating preserves the existing credential-store semantics. Discovery is
                    // for new connections; an update may reuse a key we must not resolve here.
                    if (previous.is_some() || !descriptor.models.is_empty()) && !discover {
                        return management.save_form(registration, previous.map(|r| r.id), material, mode, cancel).await;
                    }
                    match management.catalog.discover_models(&registration, &material, cancel.clone()).await {
                        Ok(ModelCatalog::Available(models)) if models.is_empty() => Ok(form_error("No models available. Install a local model or check the Cloud account, then submit again.")),
                        Ok(ModelCatalog::Available(models)) if !discover => {
                            if !models.iter().any(|m| m.id == material.settings.model) { return Ok(form_error("Model is not available from this provider. Leave Model ID empty to discover models.")); }
                            management.save_form(registration, None, material, mode, cancel).await
                        }
                        Ok(ModelCatalog::Available(models)) => {
                            let labels: Vec<_> = models.into_iter().map(|m| m.id).collect();
                            if labels.len() > 256 { return Ok(form_error("Too many models for the picker. Enter a model ID.")); }
                            let pending = Arc::new(Mutex::new(Some(material)));
                            Ok(CommandResult::Workflow(Workflow::new(
                                "Connection model", "Choose a discovered model. After saving, use /model to select it for chat.",
                                vec![choice("model", "Model", labels.clone())],
                                Arc::new(move |input, cancel| {
                                    let material = pending.lock().ok().and_then(|mut p| p.take());
                                    let selected = labels.get(index(&input, "model")).cloned();
                                    let management = management.clone(); let registration = registration.clone();
                                    Box::pin(async move {
                                        let (Some(mut material), Some(model)) = (material, selected) else { return Ok(form_error("This connection was not saved and its API key was already used. Reopen /login and enter the API key again.")); };
                                        material.settings.model = model;
                                        management.save_form(registration, None, material, mode, cancel).await
                                    })
                                }),
                            )?.with_overlay_kind(OverlayKind::ModelPicker)?))
                        }
                        Ok(ModelCatalog::Unsupported) if !discover => management.save_form(registration, None, material, mode, cancel).await,
                        Ok(ModelCatalog::Unsupported) => Ok(form_error("Model discovery is unsupported. Enter a model ID manually.")),
                        Ok(ModelCatalog::Unavailable) => Ok(form_error("Cannot reach the provider or authenticate. Check the endpoint, service and API key, then submit again.")),
                        Ok(ModelCatalog::Invalid) | Err(_) => Ok(form_error("Provider discovery failed. Check the provider configuration and reopen /login.")),
                    }
                })
            }))?.with_overlay_kind(OverlayKind::Login)?))
    }
    async fn save_form(
        &self,
        registration: ProviderRegistration,
        previous: Option<uuid::Uuid>,
        material: ConnectionMaterial,
        mode: SecretMode,
        cancel: huginn_core::CancellationToken,
    ) -> Result<CommandResult> {
        if cancel.is_cancelled()
            || !self
                .catalog
                .adapters()
                .iter()
                .any(|(r, _)| r == &registration)
        {
            return Ok(notice("Provider is no longer available. Reopen /login."));
        }
        match self
            .connections
            .save(previous, material.settings, material.secret, mode)
            .await
        {
            Ok(_) => self.configured_result(),
            Err(error) => Ok(form_error(format!(
                "Could not save the connection: {error}. Reopen /providers to check actual status; durable completion may be unknown."
            ))),
        }
    }
    fn configured_result(&self) -> Result<CommandResult> {
        let owner = self.clone();
        Ok(CommandResult::Workflow(Workflow::new(
            "Connection configured — Step 4 of 4",
            "Local configuration stored. Remote authorization and model discovery have not been verified. The active model is unchanged.",
            vec![choice("action", "Next", vec!["Select model (/model)".into(), "Manage connections".into(), "Close".into()])],
            Arc::new(move |input, _| {
                let owner = owner.clone();
                Box::pin(async move { match index(&input, "action") {
                    0 => owner.connections("/model").await,
                    1 => owner.connections("/providers").await,
                    _ => Ok(CommandResult::Dismiss),
                } })
            }),
        )?.without_history().with_overlay_kind(OverlayKind::Login)?))
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
                                let Some((registration, descriptor)) = management.catalog.adapters().into_iter().find(|(_, d)| d.id == record.settings.provider) else { return Ok(failure(ConnectionError::AdapterUnavailable)); };
                                return management.connection_form(registration, descriptor, record.settings.requires_secret, Some(record));
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
            let mut field = choice(
                "model",
                "Model",
                descriptor.models.iter().map(|m| m.id.clone()).collect(),
            );
            field.initial = descriptor
                .models
                .iter()
                .position(|model| model.id == record.settings.model)
                .unwrap_or(0)
                .to_string();
            field
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

#[cfg(test)]
mod provider_label_tests {
    use super::*;

    #[test]
    fn duplicate_display_names_include_provider_ids() {
        let options = ["provider-a", "provider-b"]
            .into_iter()
            .map(|id| {
                (
                    ProviderRegistration::new(id.into(), 1),
                    ProviderDescriptor {
                        id: id.into(),
                        display_name: "Same provider".into(),
                        models: vec![],
                        authentication: vec![],
                    },
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            provider_labels(&options),
            ["Same provider (provider-a)", "Same provider (provider-b)"]
        );
    }

    #[test]
    fn unique_display_names_remain_unchanged() {
        let options = vec![(
            ProviderRegistration::new("provider-a".into(), 1),
            ProviderDescriptor {
                id: "provider-a".into(),
                display_name: "Provider A".into(),
                models: vec![],
                authentication: vec![],
            },
        )];
        assert_eq!(provider_labels(&options), ["Provider A"]);
    }
}
