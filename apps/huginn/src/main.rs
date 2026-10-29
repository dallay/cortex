mod auth;
mod prompts;
mod terminal;

use anyhow::Context;
use clap::{Parser, Subcommand};
use huginn::{
    config::{Config, ModelReadiness},
    resolve_workspace, skill_completion, ui_settings,
};
use huginn_application::authorization::HostPolicy;
use huginn_core::{
    kernel::ServiceSlotHandle, ApprovalPolicy, CancellationToken, EventSink, LoopService,
    ModelService, Session, SessionStore,
};
use huginn_presentation::settings::UiConfig;
use huginn_runtime::{
    composition, loop_engine::LoopConfig, mcp, sessions::SqliteSessions, tools::Registry,
};
use std::{collections::BTreeSet, io::Write, path::PathBuf, sync::Arc};
use terminal::{Input, LinePrompt, Output};

fn escape_terminal_value(value: &str) -> String {
    value.chars().fold(String::new(), |mut escaped, character| {
        if character.is_control()
            || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            escaped.extend(character.escape_default());
        } else {
            escaped.push(character);
        }
        escaped
    })
}

#[derive(Parser)]
#[command(name = "huginn", version, about = "Huginn, the Cortex coding agent")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    #[arg(long, global = true)]
    provider: Option<String>,
    #[arg(long, global = true)]
    base_url: Option<String>,
    #[arg(long, global = true)]
    model: Option<String>,
    /// Emit newline-delimited JSON events to stdout.
    #[arg(long, global = true)]
    json: bool,
    /// Use the explicit line-oriented recovery adapter instead of Ratatui.
    #[arg(long, global = true)]
    line_mode: bool,
    /// Explicitly load one experimental local WASM outline component in headless mode.
    #[cfg(feature = "experimental-wasm")]
    #[arg(long, global = true)]
    experimental_wasm_outline: Option<PathBuf>,
    /// Explicit action grants for this invocation only (e.g. native.shell).
    #[arg(long, global = true, value_delimiter = ',')]
    allow: Vec<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Subcommand)]
enum Commands {
    /// Configure provider credentials without starting a conversation.
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
    /// Interactive conversation (default).
    Chat,
    /// Run one prompt. Effects are denied unless explicitly granted with --allow.
    Run { prompt: String },
    /// Resume a session; optionally run a single prompt without interaction.
    Resume {
        id: String,
        #[arg(long)]
        prompt: Option<String>,
    },
    /// List saved sessions without starting a model or MCP server.
    Sessions,
    /// Inspect scoped Agent Skills without starting models or sessions.
    Skills {
        #[command(subcommand)]
        command: SkillsCommand,
    },
    /// Validate configuration and show service diagnostics without network/model calls.
    Doctor,
}

#[derive(Subcommand)]
enum SkillsCommand {
    /// List fresh metadata and diagnostics from global and project skill roots.
    List,
}

#[derive(Subcommand)]
enum AuthCommands {
    /// Connect a provider using secure interactive forms (requires a terminal).
    Login,
}

fn prepare_config(cli: &Cli) -> anyhow::Result<Config> {
    let mut config = Config::load(cli.config.as_deref())?;
    if let Some(provider) = &cli.provider {
        config.provider = provider.clone();
        config.provider_explicit = true;
    }
    if let Some(base_url) = &cli.base_url {
        config.base_url = Some(base_url.clone());
    }
    if let Some(model) = &cli.model {
        config.model = Some(model.clone());
    }
    if let Some(db) = &cli.db {
        config.db = db.clone();
    }
    let sessions_only = matches!(cli.command, Some(Commands::Sessions));
    if !sessions_only {
        config.fill_from_env();
        config.validate_common()?;
    }
    // Reject an invalid explicit winner before opening any database, but give
    // an unconfigured invocation a chance to recover its durable selection.
    if !sessions_only && config.model_readiness() == ModelReadiness::Invalid {
        config.require_model()?;
    }
    Ok(config)
}

fn doctor_report(
    config: &Config,
    session: &Session,
    services: impl serde::Serialize,
    model_readiness: &str,
) -> anyhow::Result<serde_json::Value> {
    Ok(serde_json::json!({
        "workspace": session.workspace,
        "database": config.db,
        "provider": config.provider,
        "base_url": config.base_url,
        "model": config.model,
        "api_key_env": config.api_key_env,
        "api_key_resolved": config.resolve_api_key()?.is_some(),
        "mcp_servers": config.mcp.iter().map(|server| &server.name).collect::<Vec<_>>(),
        "services": services,
        "model_readiness": model_readiness,
        "rook_compatibility": "unsupported",
    }))
}

fn current_model_ready(model_slot: &ServiceSlotHandle<ModelService>) -> bool {
    // `acquire` resolves the current slot generation; an unavailable slot is not ready.
    model_slot
        .acquire()
        .is_ok_and(|model| model.0.is_configured())
}

async fn list_sessions_if_requested(
    cli: &Cli,
    sessions: &Arc<SqliteSessions>,
) -> anyhow::Result<bool> {
    if !matches!(cli.command, Some(Commands::Sessions)) {
        return Ok(false);
    }
    for session in sessions.list().await? {
        if cli.json {
            println!("{}", serde_json::to_string(&session)?);
        } else {
            println!(
                "{}\t{}{}",
                session.id,
                session.workspace.display(),
                if session.interrupted {
                    "\tinterrupted"
                } else {
                    ""
                }
            );
        }
    }
    Ok(true)
}

async fn prepare_session(
    cli: &Cli,
    sessions: &Arc<SqliteSessions>,
) -> anyhow::Result<(
    Session,
    Option<String>,
    Option<huginn_runtime::sessions::SessionLease>,
)> {
    let (session, prompt, lease) = match &cli.command {
        Some(Commands::Resume { id, prompt }) => {
            // Lock before loading the snapshot to prevent a stale load-then-lock race.
            let lease = sessions.lease(id)?;
            let session = sessions.load(id).await?;
            if let Some(workspace) = &cli.workspace {
                anyhow::ensure!(
                    workspace.canonicalize()? == session.workspace,
                    "resume workspace differs from saved session"
                );
            }
            (session, prompt.clone(), Some(lease))
        }
        _ => {
            let workspace = cli
                .workspace
                .clone()
                .unwrap_or(std::env::current_dir()?)
                .canonicalize()?;
            let prompt = match &cli.command {
                Some(Commands::Run { prompt }) => Some(prompt.clone()),
                _ => None,
            };
            (Session::new(workspace), prompt, None)
        }
    };
    anyhow::ensure!(session.workspace.is_dir(), "saved workspace is unavailable");
    Ok((session, prompt, lease))
}

fn prepare_interaction(
    cli: &Cli,
    has_prompt: bool,
    is_doctor: bool,
) -> anyhow::Result<(bool, bool, Option<Arc<Input>>)> {
    let interactive = !has_prompt && !is_doctor;
    let tui = interactive && !cli.line_mode && !cli.json;
    if interactive {
        use std::io::IsTerminal;
        // TUI requires a TTY. Line and JSON modes support piped input.
        anyhow::ensure!(
            std::io::stdin().is_terminal() || cli.line_mode || cli.json,
            "interactive mode needs a terminal; use `huginn run <prompt>` or explicit --line-mode recovery"
        );
    }
    let input = (interactive && !tui).then(|| Arc::new(Input::new()));
    Ok((interactive, tui, input))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    #[cfg(feature = "experimental-wasm")]
    if cli.experimental_wasm_outline.is_some()
        && !matches!(
            &cli.command,
            Some(
                Commands::Run { .. }
                    | Commands::Resume {
                        prompt: Some(_),
                        ..
                    }
            )
        )
    {
        anyhow::bail!("--experimental-wasm-outline requires run or resume --prompt; interactive loading is not supported");
    }
    if matches!(cli.command, Some(Commands::Skills { .. })) {
        use huginn_application::skills::SkillCatalogQuery;
        let workspace = resolve_workspace(cli.workspace.clone())?;
        let home = std::env::var_os("HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from);
        let catalog =
            huginn_runtime::skills::FilesystemSkillCatalog.refresh(&workspace, home.as_deref())?;
        let mut out = std::io::stdout().lock();
        if cli.json {
            writeln!(out, "{}", catalog.to_json())?;
        } else {
            writeln!(out, "Agent Skills ({})", catalog.fingerprint)?;
            for skill in &catalog.skills {
                writeln!(
                    out,
                    "{} [{}; {}; untrusted]\n  {}\n  logical: {}\n  canonical: {}",
                    escape_terminal_value(&skill.name),
                    skill.source_scope.as_str(),
                    if skill.selected {
                        "selected"
                    } else {
                        "overridden"
                    },
                    escape_terminal_value(&skill.description),
                    escape_terminal_value(&skill.logical_path.to_string_lossy()),
                    escape_terminal_value(&skill.canonical_path.to_string_lossy())
                )?;
            }
            for diagnostic in &catalog.diagnostics {
                writeln!(
                    out,
                    "{} [{}; {}] {}: {}",
                    diagnostic.code,
                    diagnostic.scope.as_str(),
                    diagnostic.severity,
                    escape_terminal_value(&diagnostic.path.to_string_lossy()),
                    escape_terminal_value(&diagnostic.message)
                )?;
            }
        }
        return Ok(());
    }
    if matches!(cli.command, Some(Commands::Auth { .. })) {
        auth::require_interactive(cli.json, cli.line_mode)?;
        return auth::login(prepare_config(&cli)?, cli.workspace.clone()).await;
    }
    let config = prepare_config(&cli)?;
    let sessions = Arc::new(SqliteSessions::open(&config.db)?);
    if list_sessions_if_requested(&cli, &sessions).await? {
        return Ok(());
    }
    let (mut session, prompt, lease) = prepare_session(&cli, &sessions).await?;
    let guards = huginn_runtime::guardrails::load_guardrails(&session.workspace)?;
    let _lease = match lease {
        Some(lease) => lease,
        None => sessions.lease(&session.id)?,
    };
    let is_doctor = matches!(cli.command, Some(Commands::Doctor));
    let (_, tui, input) = prepare_interaction(&cli, prompt.is_some(), is_doctor)?;
    let allowed = cli.allow.into_iter().collect::<BTreeSet<_>>();
    let prompt_adapter = Arc::new(LinePrompt::new(input.clone()));
    let policy = Arc::new(HostPolicy::new(prompt_adapter.clone(), allowed.clone()));
    let output = Output { json: cli.json };
    let model = if is_doctor && config.model_readiness() == ModelReadiness::Unconfigured {
        Arc::new(huginn_runtime::model::UnconfiguredModelProvider)
            as Arc<dyn huginn_core::ModelProvider>
    } else {
        config.create_model()?
    };
    let registry = Registry::default();
    let startup = CancellationToken::new();
    let loop_config = LoopConfig {
        skills: None,
        hooks: None,
        max_iterations: config.max_iterations,
        context_tokens: config.context_tokens,
        max_output_tokens: config.max_output_tokens,
    };
    let access = huginn_application::connections::ConnectionAccess::new(Arc::new(
        huginn_runtime::connections::HostSecrets::default(),
    ));
    let mut supervisor = composition::build_native_with_connections_and_guards(
        model.clone(),
        access.clone(),
        registry.clone(),
        sessions.clone(),
        loop_config,
        config.tool_timeout_secs,
        guards.clone(),
    )
    .await?;
    drop(model);
    // Explicit headless configurations need neither durable metadata nor its
    // global writer lease. TUI always needs management for deliberate UI changes.
    let mut connection_management = false;
    if tui || config.model_readiness() == ModelReadiness::Unconfigured {
        let result: anyhow::Result<bool> = async {
            use huginn_application::{
                connections::{ConnectionManager, ConnectionsPlugin},
                providers::{catalog_id, providers_id},
            };
            use huginn_core::{
                connections::ConnectionStore,
                providers::{ProviderAction, ProviderCatalogService, ProvidersService},
            };
            supervisor.register(Box::new(huginn_runtime::providers::OllamaLocalPlugin {
                timeout_secs: config.timeout_secs,
            }))?;
            supervisor.register(Box::new(huginn_runtime::providers::OllamaCloudPlugin {
                timeout_secs: config.timeout_secs,
            }))?;
            supervisor.register(Box::new(huginn_runtime::providers::CustomProviderPlugin {
                timeout_secs: config.timeout_secs,
            }))?;
            supervisor.start().await?;
            let providers = supervisor.resolve::<ProvidersService>(&providers_id())?;
            let catalog = supervisor.resolve::<ProviderCatalogService>(&catalog_id())?;
            let explicit_selection = if config.model_readiness() == ModelReadiness::Ready {
                providers.0.diagnostic()?.selection
            } else {
                None
            };
            let store = match huginn_runtime::connections::SqliteConnections::open(
                &config.db.with_extension("providers.db"),
            ) {
                Ok(store) => Arc::new(store) as Arc<dyn ConnectionStore>,
                Err(huginn_core::connections::ConnectionError::Conflict) if tui || is_doctor => {
                    // Another process owns durable connection management. Keep
                    // the TUI/doctor available; management is optional here.
                    return Ok(false);
                }
                Err(error) => return Err(error.into()),
            };
            let manager = Arc::new(
                ConnectionManager::open(store, access, providers.0.clone(), catalog.0.clone())
                    .await?,
            );
            // Explicit configuration still wins at startup; UI choices are deliberate
            // runtime changes. Never combine stored credentials with legacy settings.
            if let Some(selection) = explicit_selection {
                providers
                    .0
                    .execute(ProviderAction::Select(selection), CancellationToken::new())
                    .await?;
            }
            supervisor.register(Box::new(ConnectionsPlugin(manager)))?;
            supervisor.start().await?;
            Ok(true)
        }
        .await;
        match result {
            Ok(enabled) => connection_management = enabled,
            Err(error) => {
                supervisor.shutdown().await;
                return Err(error);
            }
        }
    }
    let model_readiness =
        supervisor.service_slot_handle::<ModelService>(&composition::model_id())?;
    if is_doctor {
        let readiness = if current_model_ready(&model_readiness) {
            "ready"
        } else {
            "unconfigured"
        };
        let report = doctor_report(&config, &session, supervisor.diagnostics(), readiness)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        supervisor.shutdown().await;
        return Ok(());
    }
    if tui {
        let result = tui_chat(
            &config,
            cli.config.as_deref(),
            &mut session,
            &mut supervisor,
            allowed,
            model_readiness,
            connection_management,
        )
        .await;
        supervisor.shutdown().await;
        return result;
    }
    if prompt.is_some() && !current_model_ready(&model_readiness) {
        supervisor.shutdown().await;
        config.require_model()?;
    }
    #[cfg(feature = "experimental-wasm")]
    if let Some(path) = &cli.experimental_wasm_outline {
        let result =
            huginn_runtime::wasm::WasmOutlinePlugin::load(path, &session.workspace, guards)
                .and_then(|plugin| supervisor.register(Box::new(plugin)));
        if let Err(error) = result {
            supervisor.shutdown().await;
            return Err(error.into());
        }
    }
    if let Err(error) = mcp::register_plugins(
        &mut supervisor,
        &config.mcp,
        &session.workspace,
        policy.clone(),
        startup.clone(),
        config.tool_timeout_secs,
    ) {
        supervisor.shutdown().await;
        return Err(error.into());
    }
    if let Err(error) = interruptible(startup, supervisor.start()).await {
        supervisor.shutdown().await;
        return Err(error.into());
    }
    let implementation = supervisor.resolve::<LoopService>(&composition::loop_id())?;
    if !cli.json {
        eprintln!(
            "Session: {}\nWorkspace: {}",
            session.id,
            escape_terminal_value(&session.workspace.display().to_string())
        );
    }
    if session.interrupted {
        eprintln!("Previous turn was interrupted. Effect completion may be unknown; nothing will be replayed automatically.");
    }
    let result = run_prompt_loop(
        prompt,
        &mut session,
        implementation.0.as_ref(),
        input.as_deref(),
        &prompt_adapter,
        policy.as_ref(),
        &output,
        &model_readiness,
    )
    .await;
    drop(implementation);
    supervisor.shutdown().await;
    result
}

#[allow(clippy::too_many_arguments)]
async fn run_prompt_loop(
    prompt: Option<String>,
    session: &mut Session,
    implementation: &dyn huginn_core::AgentLoop,
    input: Option<&Input>,
    prompt_adapter: &LinePrompt,
    policy: &dyn ApprovalPolicy,
    output: &Output,
    model_readiness: &ServiceSlotHandle<ModelService>,
) -> anyhow::Result<()> {
    if let Some(prompt) = prompt {
        if !current_model_ready(model_readiness) {
            anyhow::bail!(
                "No model configured. Set HUGINN_BASE_URL and HUGINN_MODEL to enable inference."
            );
        }
        let prompt = prompts::expand(session, prompt)?;
        prompt_adapter.reset_turn();
        let cancel = CancellationToken::new();
        return interruptible(
            cancel.clone(),
            implementation.run(session, prompt, policy, output, cancel),
        )
        .await
        .map_err(anyhow::Error::from);
    }
    let terminal_input = input.context("missing terminal input")?;
    loop {
        eprint!("\nhuginn> ");
        let line =
            tokio::select! {value=terminal_input.line()=>value?,_=tokio::signal::ctrl_c()=>break};
        let Some(line) = line else {
            break;
        };
        let line = if line.trim_start().starts_with("/skill:") {
            line
        } else {
            line.trim().to_string()
        };
        if matches!(line.as_str(), "/quit" | "/exit") {
            break;
        }
        if line.is_empty() {
            continue;
        }
        if !current_model_ready(model_readiness) {
            eprintln!(
                "\nNo model configured. Set HUGINN_BASE_URL and HUGINN_MODEL to enable inference."
            );
            continue;
        }
        // `/compact` and `/summarize` (alias) run the same real-model
        // summarizer the conservative byte budget would call. We
        // require explicit confirmation through the same `Input`
        // reader that handles approvals.
        if matches!(line.as_str(), "/compact" | "/summarize") {
            let cancel = CancellationToken::new();
            if let Err(error) = interruptible(
                cancel.clone(),
                run_compact(implementation, session, terminal_input, output, cancel),
            )
            .await
            {
                eprintln!("\n{error}");
            }
            continue;
        }
        let line = match prompts::expand(session, line) {
            Ok(line) => line,
            Err(error) => {
                eprintln!("\n{error}");
                continue;
            }
        };
        prompt_adapter.reset_turn();
        let cancel = CancellationToken::new();
        if let Err(error) = interruptible(
            cancel.clone(),
            implementation.run(session, line, policy, output, cancel),
        )
        .await
        {
            eprintln!("\n{error}");
        }
    }
    Ok(())
}

async fn interruptible<T>(
    cancel: CancellationToken,
    future: impl std::future::Future<Output = huginn_core::Result<T>>,
) -> huginn_core::Result<T> {
    tokio::pin!(future);
    tokio::select! {
        result=&mut future=>result,
        signal=tokio::signal::ctrl_c()=>{
            signal.map_err(huginn_core::AgentError::Io)?;cancel.cancel();future.await
        }
    }
}

async fn run_compact(
    agent_loop: &dyn huginn_core::AgentLoop,
    session: &mut Session,
    input: &Input,
    output: &Output,
    cancel: CancellationToken,
) -> huginn_core::Result<()> {
    eprint!(
        "\nCompact session now? Older history will be summarized; originals stay in the database. [y/N] "
    );
    std::io::stderr().flush().ok();
    // Ctrl+C is handled by the surrounding `interruptible()` wrapper; we
    // only need to react to a cancel already in flight and to the readline.
    let line = tokio::select! {
        _ = cancel.cancelled() => return Err(huginn_core::AgentError::Cancelled),
        l = input.line() => l?,
    };
    let accepted = line
        .as_deref()
        .is_some_and(|s| matches!(s.trim(), "y" | "Y" | "yes"));
    if !accepted {
        eprintln!("Compaction skipped.");
        return Ok(());
    }
    agent_loop.compact(session, output, cancel).await
}

async fn tui_chat(
    config: &Config,
    cli_config: Option<&std::path::Path>,
    session: &mut Session,
    supervisor: &mut huginn_core::kernel::Supervisor,
    allowed: BTreeSet<String>,
    model_readiness: ServiceSlotHandle<ModelService>,
    connection_management: bool,
) -> anyhow::Result<()> {
    use huginn_presentation::{
        contributions::ConversationPlugin, presentation_id, PresentationService, RatatuiPlugin,
    };
    let readiness = model_readiness.clone();
    // Reading and writing share one path: an explicit --config, otherwise
    // ~/.cortex/huginn/config.toml. There is no legacy layout to seed from.
    let save_path = cli_config.map_or_else(Config::primary_config_path, |path| path.to_path_buf());
    let ui_config = Arc::new(config.ui.clone());
    let save_ui_config: huginn_presentation::settings::SaveUiConfig =
        Arc::new(move |ui: &UiConfig| ui_settings::persist_seeded(ui, &save_path, None));
    supervisor.register(Box::new(
        RatatuiPlugin::with_readiness(Arc::new(move || current_model_ready(&readiness)))
            .with_connection(
                supervisor
                    .resolve::<huginn_core::providers::ProvidersService>(
                        &huginn_application::providers::providers_id(),
                    )?
                    .0
                    .subscribe(),
            )
            .with_settings(ui_config, save_ui_config),
    ))?;
    supervisor.register(Box::new(ConversationPlugin))?;
    if connection_management {
        supervisor.register(Box::new(huginn_presentation::providers::ProvidersUiPlugin))?;
    }
    #[cfg(feature = "reference-extension")]
    supervisor.register(Box::new(huginn_reference_extension::ReferenceExtension))?;
    supervisor.start().await?;
    let presentation = supervisor.resolve::<PresentationService>(&presentation_id())?;
    let catalog = prompts::catalog(session)?;
    let skill_result = {
        let implementation = supervisor.resolve::<LoopService>(&composition::loop_id())?;
        implementation
            .0
            .skill_commands(&session.workspace, CancellationToken::new())
            .await
    };
    let (mut host_commands, skill_error) = skill_completion::completion_result(skill_result);
    host_commands.extend(catalog.entries.iter().map(|entry| {
        (
            format!("/prompts:{}", entry.name),
            format!(
                "{} {} [{}]",
                entry.description,
                entry.argument_hint,
                if entry.project { "proyecto" } else { "usuario" }
            ),
        )
    }));
    let connection = presentation.0.open(session).await?;
    let ui = connection.ui.clone();
    ui.set_prompt_templates(host_commands);
    if let Some(message) = skill_completion::startup_notification(skill_error, catalog.diagnostics)
    {
        ui.notification(message);
    }
    let policy = Arc::new(HostPolicy::new(ui.clone(), allowed));
    let result: anyhow::Result<()> = async {
        let startup = CancellationToken::new();
        let startup_operation = ui.begin(startup.clone());
        mcp::register_plugins(supervisor, &config.mcp, &session.workspace, policy.clone(), startup.clone(), config.tool_timeout_secs)?;
        interruptible(startup, supervisor.start()).await?;
        ui.synchronize(session);
        ui.finish_operation(startup_operation);
        let implementation = supervisor.resolve::<LoopService>(&composition::loop_id())?;
        while let Some(prompt) = ui.prompt().await {
            if !current_model_ready(&model_readiness) {
                ui.notification("No model configured. Use /login, then /model. No restart needed.".into());
                ui.synchronize(session);
                continue;
            }
            let prompt = match prompts::expand(session, prompt) {
                Ok(prompt) => prompt,
                Err(error) => {
                    ui.notification(error.to_string());
                    ui.synchronize(session);
                    continue;
                }
            };
            let cancel = CancellationToken::new();
            let operation = ui.begin(cancel.clone());
            let scoped_events = huginn_core::ScopedEventSink { sink: ui.as_ref(), operation };
            let result = if matches!(prompt.as_str(), "/compact" | "/summarize") {
                let request = huginn_core::ApprovalRequest {
                    id: uuid::Uuid::new_v4().to_string(),
                    action: "session.compact".into(),
                    preview: "Older history will be summarized by the model. Original messages stay in the database. No effects are replayed.".into(),
                };
                match policy.approve(&request, cancel.clone()).await {
                    Ok(true) => {
                        interruptible(
                            cancel.clone(),
                            implementation.0.compact(session, &scoped_events, cancel),
                        )
                        .await
                    }
                    Ok(false) => {
                        ui.notification("Compaction skipped.".into());
                        Ok(())
                    }
                    Err(error) => Err(error),
                }
            } else {
                interruptible(
                    cancel.clone(),
                    implementation
                        .0
                        .run(session, prompt, policy.as_ref(), &scoped_events, cancel),
                )
                .await
            };
            ui.synchronize(session);
            if let Err(error) = &result {
                scoped_events.emit(huginn_core::Event::Error {
                    message: error.to_string(),
                });
            }
            ui.finish_operation(operation);
        }
        Ok(())
    }
    .await;
    // Restore the TTY before any shutdown error can be printed by the host.
    let restored = connection.finish().await;
    result?;
    restored?;
    Ok(())
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    use async_trait::async_trait;
    use futures::stream::BoxStream;
    use huginn_core::{
        kernel::{Manifest, Plugin, PluginContext, SubstitutionPolicy, Supervisor},
        AgentError, ModelDelta, ModelProvider, ModelRequest, ModelService,
    };

    struct ConfiguredModel(bool);

    #[async_trait]
    impl ModelProvider for ConfiguredModel {
        fn is_configured(&self) -> bool {
            self.0
        }

        async fn stream(
            &self,
            _: ModelRequest,
            _: CancellationToken,
        ) -> huginn_core::Result<BoxStream<'static, huginn_core::Result<ModelDelta>>> {
            Err(AgentError::Model("readiness test does not infer".into()))
        }
    }

    struct ModelPlugin {
        id: &'static str,
        configured: bool,
    }

    #[async_trait]
    impl Plugin for ModelPlugin {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: self.id.into(),
                provides: vec![huginn_runtime::composition::model_id()],
                requires: Vec::new(),
            }
        }

        async fn activate(&mut self, ctx: &mut PluginContext) -> huginn_core::Result<()> {
            ctx.provide(
                huginn_runtime::composition::model_id(),
                ModelService(Arc::new(ConfiguredModel(self.configured))),
            )
        }
    }

    #[tokio::test]
    async fn retained_readiness_slot_tracks_replacement_and_fails_closed() {
        let service = huginn_runtime::composition::model_id();
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(ModelPlugin {
                id: "test.model.initial",
                configured: false,
            }))
            .unwrap();
        supervisor.start().await.unwrap();
        let readiness = supervisor
            .service_slot_handle::<ModelService>(&service)
            .unwrap();

        assert!(!current_model_ready(&readiness));
        supervisor
            .substitute(
                &service,
                Box::new(ModelPlugin {
                    id: "test.model.replacement",
                    configured: true,
                }),
                SubstitutionPolicy::ReportUndrained,
                std::time::Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert!(current_model_ready(&readiness));

        supervisor.remove("test.model.replacement").await.unwrap();
        assert!(!current_model_ready(&readiness));
        supervisor.shutdown().await;
    }
}
