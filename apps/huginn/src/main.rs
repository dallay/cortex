mod terminal;

use anyhow::Context;
use clap::{Parser, Subcommand};
use huginn::{
    config::{Config, ModelReadiness},
    ui_settings,
};
use huginn_application::authorization::HostPolicy;
use huginn_core::{
    ApprovalPolicy, CancellationToken, EventSink, LoopService, Session, SessionStore,
};
use huginn_presentation::settings::UiConfig;
use huginn_runtime::{
    composition, loop_engine::LoopConfig, mcp, sessions::SqliteSessions, tools::Registry,
};
use std::{collections::BTreeSet, io::Write, path::PathBuf, sync::Arc};
use terminal::{Input, LinePrompt, Output};

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
    /// Explicit action grants for this invocation only (e.g. native.shell).
    #[arg(long, global = true, value_delimiter = ',')]
    allow: Vec<String>,
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Subcommand)]
enum Commands {
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
    /// Validate configuration and show service diagnostics without network/model calls.
    Doctor,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = prepare_config(&cli)?;
    let sessions = Arc::new(SqliteSessions::open(&config.db)?);
    if matches!(cli.command, Some(Commands::Sessions)) {
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
        return Ok(());
    }
    let (mut session, prompt, lease) = match &cli.command {
        Some(Commands::Resume { id, prompt }) => {
            // Acquire session ownership before reading the snapshot used for this
            // resumed execution. This prevents a stale load-then-lock race.
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
    let _lease = match lease {
        Some(lease) => lease,
        None => sessions.lease(&session.id)?,
    };
    let is_doctor = matches!(cli.command, Some(Commands::Doctor));
    let interactive = prompt.is_none() && !is_doctor;
    let tui = interactive && !cli.line_mode && !cli.json;
    if interactive {
        use std::io::IsTerminal;
        // The Ratatui path (tui) always needs a TTY: it implies neither
        // --line-mode nor --json. The line/NDJSON adapter works headless,
        // so piped stdin is allowed with --json (or --line-mode recovery).
        anyhow::ensure!(
            std::io::stdin().is_terminal() || cli.line_mode || cli.json,
            "interactive mode needs a terminal; use `huginn run <prompt>` or explicit --line-mode recovery"
        );
    }
    let input = if interactive && !tui {
        Some(Arc::new(Input::new()))
    } else {
        None
    };
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
        max_iterations: config.max_iterations,
        context_tokens: config.context_tokens,
        max_output_tokens: config.max_output_tokens,
    };
    let access = huginn_application::connections::ConnectionAccess::new(Arc::new(
        huginn_runtime::connections::HostSecrets::default(),
    ));
    let mut supervisor = composition::build_native_with_connections(
        model.clone(),
        access.clone(),
        registry.clone(),
        sessions.clone(),
        loop_config,
        config.tool_timeout_secs,
    )
    .await?;
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
    let router = supervisor
        .resolve::<huginn_core::ModelService>(&composition::model_id())?
        .0
        .clone();
    if is_doctor {
        let readiness = if router.is_configured() {
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
            router,
            connection_management,
        )
        .await;
        supervisor.shutdown().await;
        return result;
    }
    if prompt.is_some() && !router.is_configured() {
        supervisor.shutdown().await;
        config.require_model()?;
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
            session.workspace.display()
        );
    }
    if session.interrupted {
        eprintln!("Previous turn was interrupted. Effect completion may be unknown; nothing will be replayed automatically.");
    }
    let result = async {
        if let Some(prompt) = prompt {
            prompt_adapter.reset_turn();
            let cancel = CancellationToken::new();
            return interruptible(
                cancel.clone(),
                implementation
                    .0
                    .run(&mut session, prompt, policy.as_ref(), &output, cancel),
            )
            .await
            .map_err(anyhow::Error::from);
        }
        let terminal_input = input.context("missing terminal input")?;
        loop {
            eprint!("\nhuginn> ");
            let line = tokio::select! {value=terminal_input.line()=>value?,_=tokio::signal::ctrl_c()=>break};
            let Some(line) = line else {
                break;
            };
            let line = line.trim().to_string();
            if matches!(line.as_str(), "/quit" | "/exit") {
                break;
            }
            if line.is_empty() {
                continue;
            }
            if !router.is_configured() {
                eprintln!("\nNo model configured. Set HUGINN_BASE_URL and HUGINN_MODEL to enable inference.");
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
                    run_compact(
                        implementation.0.as_ref(),
                        &mut session,
                        &terminal_input,
                        &output,
                        cancel,
                    ),
                )
                .await
                {
                    eprintln!("\n{error}");
                }
                continue;
            }
            prompt_adapter.reset_turn();
            let cancel = CancellationToken::new();
            if let Err(error) = interruptible(
                cancel.clone(),
                implementation
                    .0
                    .run(&mut session, line, policy.as_ref(), &output, cancel),
            )
            .await
            {
                eprintln!("\n{error}");
            }
        }
        Ok(())
    }
    .await;
    drop(implementation);
    supervisor.shutdown().await;
    result
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
    router: Arc<dyn huginn_core::ModelProvider>,
    connection_management: bool,
) -> anyhow::Result<()> {
    use huginn_presentation::{
        contributions::ConversationPlugin, presentation_id, PresentationService, RatatuiPlugin,
    };
    let readiness = router.clone();
    // Reading may fall back to the legacy layout; writing never does. The
    // legacy file seeds the new one, so a save cannot hide provider, model or
    // database settings and the deprecated file stays untouched.
    let read_path = Config::resolve_path(cli_config);
    let write_path = cli_config.map_or_else(Config::primary_config_path, |path| path.to_path_buf());
    let seed_path = (read_path != write_path).then(|| read_path.clone());
    let ui_config = Arc::new(config.ui.clone());
    let save_path = write_path.clone();
    let save_ui_config: huginn_presentation::settings::SaveUiConfig =
        Arc::new(move |ui: &UiConfig| {
            ui_settings::persist_seeded(ui, &save_path, seed_path.as_deref())
        });
    supervisor.register(Box::new(
        RatatuiPlugin::with_readiness(Arc::new(move || readiness.is_configured()))
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
    let connection = presentation.0.open(session).await?;
    let ui = connection.ui.clone();
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
            if !router.is_configured() {
                ui.notification("No model configured. Use /login, then /model. No restart needed.".into());
                ui.synchronize(session);
                continue;
            }
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
