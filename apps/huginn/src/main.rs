mod config;
mod terminal;

use anyhow::Context;
use clap::{Parser, Subcommand};
use config::Config;
use huginn_core::{CancellationToken, LoopService, ModelProvider, Session, SessionStore};
use huginn_runtime::{
    composition,
    loop_engine::LoopConfig,
    mcp::McpClients,
    model::{MockProvider, OpenAiProvider},
    sessions::SqliteSessions,
    tools::Registry,
};
use std::{collections::BTreeSet, io::Write, path::PathBuf, sync::Arc};
use terminal::{Input, Output, Policy};

#[derive(Parser)]
#[command(name = "huginn", version, about = "Huginn, the Cortex coding agent")]
struct Cli {
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[arg(long, global = true)]
    workspace: Option<PathBuf>,
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    #[arg(long,global=true,value_parser=["openai","mock"])]
    provider: Option<String>,
    #[arg(long, global = true)]
    base_url: Option<String>,
    #[arg(long, global = true)]
    model: Option<String>,
    /// Emit newline-delimited JSON events to stdout.
    #[arg(long, global = true)]
    json: bool,
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut config = Config::load(cli.config.as_deref())?;
    if let Some(value) = cli.provider {
        config.provider = value;
    }
    if let Some(value) = cli.base_url {
        config.base_url = Some(value);
    }
    if let Some(value) = cli.model {
        config.model = Some(value);
    }
    if let Some(value) = cli.db {
        config.db = value;
    }
    if !matches!(cli.command, Some(Commands::Sessions)) {
        // Resolve HUGINN_* / AGENT_* env vars before validating so doctor
        // and one-shot modes have the same precedence as chat/run.
        config.fill_from_env();
    }
    // The sessions subcommand only reads the database and must work without model settings.
    if !matches!(cli.command, Some(Commands::Sessions)) {
        config.validate()?;
    }
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
    if interactive {
        use std::io::IsTerminal;
        anyhow::ensure!(
            std::io::stdin().is_terminal(),
            "interactive mode needs a terminal; use `huginn run <prompt>`"
        );
    }
    let input = if interactive {
        Some(Arc::new(Input::new()))
    } else {
        None
    };
    let policy = Policy::new(
        input.clone(),
        cli.allow.into_iter().collect::<BTreeSet<_>>(),
    );
    let output = Output { json: cli.json };
    let model: Arc<dyn ModelProvider> = if config.provider == "mock" {
        Arc::new(MockProvider)
    } else {
        let key = match config
            .api_key_env
            .as_deref()
            .filter(|name| !name.is_empty())
        {
            Some(name) => Some(
                config
                    .resolve_api_key()?
                    .with_context(|| format!("credential environment variable {name} is unset"))?,
            ),
            None => None,
        };
        Arc::new(OpenAiProvider::new(
            config.base_url.as_deref().context("base_url is required")?,
            config.model.clone().context("model is required")?,
            key,
            config.timeout_secs,
        )?)
    };
    let registry = Arc::new(Registry::native(config.tool_timeout_secs)?);
    let startup = CancellationToken::new();
    let mut mcp = if is_doctor {
        None
    } else {
        Some(
            interruptible(
                startup.clone(),
                McpClients::connect(
                    &config.mcp,
                    &session.workspace,
                    &registry,
                    &policy,
                    startup.clone(),
                    config.tool_timeout_secs,
                ),
            )
            .await?,
        )
    };
    let mut supervisor = composition::build(
        model.clone(),
        registry.clone(),
        sessions.clone(),
        LoopConfig {
            max_iterations: config.max_iterations,
            context_tokens: config.context_tokens,
            max_output_tokens: config.max_output_tokens,
        },
    )
    .await?;
    if is_doctor {
        // Surface legacy env-var warnings even when the user only ran
        // `huginn doctor`. resolve_api_key is the single place that knows
        // the canonical->legacy fallback order.
        let resolved_key = config.resolve_api_key()?;
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "workspace": session.workspace,
                "database": config.db,
                "provider": config.provider,
                "base_url": config.base_url,
                "model": config.model,
                "api_key_env": config.api_key_env,
                "api_key_resolved": resolved_key.is_some(),
                "mcp_servers": config.mcp.iter().map(|c| &c.name).collect::<Vec<_>>(),
                "services": supervisor.diagnostics(),
                "rook_compatibility": "unsupported",
            }))?
        );
        supervisor.shutdown().await;
        return Ok(());
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
            policy.reset_turn();
            let cancel = CancellationToken::new();
            return interruptible(
                cancel.clone(),
                implementation
                    .0
                    .run(&mut session, prompt, &policy, &output, cancel),
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
            policy.reset_turn();
            let cancel = CancellationToken::new();
            if let Err(error) = interruptible(
                cancel.clone(),
                implementation
                    .0
                    .run(&mut session, line, &policy, &output, cancel),
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
    if let Some(clients) = &mut mcp {
        clients.shutdown().await?;
    }
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
