//! Muninn command-line entry point and composition root.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use muninn_application::{MemoryClient, MemoryOptions, MemoryPorts};
use muninn_bridge::BridgeState;
use muninn_core::ports::{Embedder, GraphMaintenance};
use muninn_neo4j::{
    Neo4jClient, Neo4jConfig, Neo4jCypherQuery, Neo4jLongTermStore, Neo4jMaintenance,
    Neo4jReasoningStore, Neo4jShortTermStore, SchemaManager,
};
use muninn_providers::{HashEmbedder, OpenAiEmbedder};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "muninn",
    version,
    about = "Persistent graph memory for AI agents"
)]
struct Cli {
    #[command(flatten)]
    neo4j: Neo4jArgs,
    #[command(flatten)]
    embedding: EmbeddingArgs,
    #[command(subcommand)]
    command: Command,
}

/// Environment names follow upstream `MemorySettings` (`NAM_` prefix, `__`
/// nesting).
#[derive(Args)]
struct Neo4jArgs {
    #[arg(long, env = "NAM_NEO4J__URI", default_value = "bolt://localhost:7687")]
    neo4j_uri: String,
    #[arg(long, env = "NAM_NEO4J__USERNAME", default_value = "neo4j")]
    neo4j_user: String,
    #[arg(long, env = "NAM_NEO4J__PASSWORD", hide_env_values = true)]
    neo4j_password: String,
    #[arg(long, env = "NAM_NEO4J__DATABASE")]
    neo4j_database: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
enum EmbeddingProvider {
    /// OpenAI-compatible `/embeddings` endpoint (OpenAI, Rook, Ollama...).
    Openai,
    /// Deterministic hash vectors (TCK `MockEmbedder`); no semantics.
    Hash,
    /// Disable embeddings; semantic search returns nothing.
    None,
}

#[derive(Args)]
struct EmbeddingArgs {
    #[arg(
        long,
        env = "NAM_EMBEDDING__PROVIDER",
        value_enum,
        default_value = "openai"
    )]
    embedding_provider: EmbeddingProvider,
    #[arg(
        long,
        env = "NAM_EMBEDDING__MODEL",
        default_value = "text-embedding-3-small"
    )]
    embedding_model: String,
    #[arg(long, env = "NAM_EMBEDDING__DIMENSIONS", default_value_t = 1536)]
    embedding_dimensions: usize,
    #[arg(
        long,
        env = "NAM_EMBEDDING__BASE_URL",
        default_value = "https://api.openai.com/v1"
    )]
    embedding_base_url: String,
    #[arg(long, env = "NAM_EMBEDDING__API_KEY", hide_env_values = true)]
    embedding_api_key: Option<String>,
}

#[derive(Subcommand)]
enum Command {
    /// Create or verify constraints and indexes.
    Schema,
    /// Serve the agent-memory TCK HTTP bridge.
    Bridge {
        #[arg(long, env = "TCK_BRIDGE_PORT", default_value_t = 3001)]
        port: u16,
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Allow `clear_all_data` to wipe the whole database. Only for
        /// disposable test databases.
        #[arg(long, env = "MUNINN_BRIDGE_ALLOW_CLEAR", default_value_t = false)]
        allow_destructive_clear: bool,
    },
}

fn embedder(args: &EmbeddingArgs) -> Result<Option<Arc<dyn Embedder>>> {
    Ok(match args.embedding_provider {
        EmbeddingProvider::None => None,
        EmbeddingProvider::Hash => Some(Arc::new(HashEmbedder::new(args.embedding_dimensions))),
        EmbeddingProvider::Openai => Some(Arc::new(OpenAiEmbedder::new(
            &args.embedding_base_url,
            args.embedding_api_key.clone(),
            &args.embedding_model,
            args.embedding_dimensions,
        )?)),
    })
}

async fn connect(args: &Neo4jArgs, dimensions: usize) -> Result<Neo4jClient> {
    let config = Neo4jConfig {
        uri: args.neo4j_uri.clone(),
        username: args.neo4j_user.clone(),
        password: args.neo4j_password.clone(),
        database: args.neo4j_database.clone(),
        ..Neo4jConfig::default()
    };
    let client = Neo4jClient::connect(&config).await?;
    let schema = SchemaManager::new(&client, dimensions);
    schema.setup_all().await?;
    schema.validate_vector_index_dimensions(dimensions).await?;
    Ok(client)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();
    let cli = Cli::parse();
    let embedder = embedder(&cli.embedding)?;
    let client = connect(&cli.neo4j, cli.embedding.embedding_dimensions)
        .await
        .context("Neo4j initialisation failed")?;

    match cli.command {
        Command::Schema => {
            tracing::info!("schema is up to date");
            Ok(())
        }
        Command::Bridge {
            port,
            host,
            allow_destructive_clear,
        } => {
            let memory = Arc::new(MemoryClient::new(
                MemoryPorts {
                    short_term: Arc::new(Neo4jShortTermStore::new(client.clone())),
                    long_term: Arc::new(Neo4jLongTermStore::new(client.clone())),
                    reasoning: Arc::new(Neo4jReasoningStore::new(client.clone())),
                    embedder,
                    cypher_query: Some(Arc::new(Neo4jCypherQuery::new(client.clone()))),
                },
                &MemoryOptions::default(),
            ));
            let maintenance: Option<Arc<dyn GraphMaintenance>> = if allow_destructive_clear {
                tracing::warn!("clear_all_data is enabled: this database will be wiped by the TCK");
                Some(Arc::new(Neo4jMaintenance::new(client)))
            } else {
                None
            };
            let ip: IpAddr = host.parse().context("invalid bridge host")?;
            let addr = SocketAddr::new(ip, port);
            if !addr.ip().is_loopback() && allow_destructive_clear {
                bail!("refusing to expose a destructive bridge on a non-loopback address");
            }
            let app = muninn_bridge::router(Arc::new(BridgeState {
                memory,
                maintenance,
            }));
            let listener = tokio::net::TcpListener::bind(addr).await?;
            tracing::info!(%addr, "TCK bridge listening");
            axum::serve(listener, app).await?;
            Ok(())
        }
    }
}
