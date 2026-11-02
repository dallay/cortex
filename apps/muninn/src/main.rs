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
    /// Local verified MiniLM CPU/F32 bundle (requires minilm-cpu feature).
    Minilm,
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
    /// Immutable remote model/configuration identity; required for semantic persistence.
    #[arg(long, env = "NAM_EMBEDDING__IDENTITY")]
    embedding_identity: Option<String>,
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
    /// Existing local artifact cache; never downloaded implicitly.
    #[arg(long, env = "NAM_EMBEDDING__BUNDLE_PATH")]
    embedding_bundle_path: Option<std::path::PathBuf>,
    #[arg(long, env = "NAM_EMBEDDING__BUNDLE_ID")]
    embedding_bundle_id: Option<String>,
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
        EmbeddingProvider::Minilm => minilm_embedder(args)?,
        EmbeddingProvider::None => None,
        EmbeddingProvider::Hash => Some(Arc::new(HashEmbedder::new(args.embedding_dimensions))),
        EmbeddingProvider::Openai => {
            let identity = args.embedding_identity.as_ref().context("remote embeddings require --embedding-identity pinning model revision and preprocessing")?;
            Some(Arc::new(
                OpenAiEmbedder::new(
                    &args.embedding_base_url,
                    args.embedding_api_key.clone(),
                    &args.embedding_model,
                    args.embedding_dimensions,
                )?
                .with_identity(muninn_core::ports::EmbeddingIdentity::new(
                    identity,
                    args.embedding_dimensions,
                )?)?,
            ))
        }
    })
}

fn minilm_embedder(args: &EmbeddingArgs) -> Result<Option<Arc<dyn Embedder>>> {
    let path = args
        .embedding_bundle_path
        .as_ref()
        .context("MiniLM requires a local bundle path")?;
    let bundle = args
        .embedding_bundle_id
        .as_ref()
        .context("MiniLM requires an explicit bundle id")?;
    #[cfg(feature = "minilm-cpu")]
    {
        Ok(Some(Arc::new(muninn_providers::MiniLmEmbedder::load(
            path, bundle,
        )?)))
    }
    #[cfg(not(feature = "minilm-cpu"))]
    {
        let _ = (path, bundle);
        bail!("MiniLM bundle support requires building with --features minilm-cpu")
    }
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
    let dimensions = embedder
        .as_ref()
        .map_or(cli.embedding.embedding_dimensions, |provider| {
            provider.dimensions()
        });
    let client = connect(&cli.neo4j, dimensions)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_cli(arguments: &[&str]) -> Cli {
        Cli::try_parse_from(arguments).expect("synthetic CLI arguments must parse")
    }

    #[test]
    fn hash_and_none_provider_selections_are_explicit() {
        let hash = parse_cli(&[
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--embedding-provider",
            "hash",
            "--embedding-dimensions",
            "2",
            "bridge",
        ]);
        let provider = embedder(&hash.embedding).unwrap().unwrap();
        assert_eq!(provider.dimensions(), 2);
        assert_eq!(
            provider.identity().unwrap().key(),
            "muninn:tck:sha256-bytes:v1"
        );

        let disabled = parse_cli(&[
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--embedding-provider",
            "none",
            "bridge",
        ]);
        assert!(embedder(&disabled.embedding).unwrap().is_none());
    }

    #[test]
    fn remote_provider_uses_the_supplied_immutable_identity() {
        let cli = parse_cli(&[
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--embedding-provider",
            "openai",
            "--embedding-model",
            "model-revision-7",
            "--embedding-identity",
            "openai:model-revision-7:preprocess-v1:pool-v1",
            "--embedding-dimensions",
            "2",
            "bridge",
        ]);
        let provider = embedder(&cli.embedding).unwrap().unwrap();
        assert_eq!(provider.dimensions(), 2);
        assert_eq!(
            provider.identity().unwrap().key(),
            "openai:model-revision-7:preprocess-v1:pool-v1"
        );
    }

    #[test]
    fn remote_provider_requires_explicit_revision_identity() {
        let cli = Cli::try_parse_from([
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "bridge",
        ])
        .unwrap();
        let error = match embedder(&cli.embedding) {
            Ok(_) => panic!("mutable model alias must not identify persisted vectors"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("identity"));
    }

    #[test]
    fn minilm_requires_explicit_local_bundle_before_connecting() {
        let cli = Cli::try_parse_from([
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--embedding-provider",
            "minilm",
            "bridge",
        ])
        .expect("MiniLM must be selectable explicitly");
        let error = match embedder(&cli.embedding) {
            Ok(_) => panic!("missing bundle must be rejected"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("bundle"));
    }

    #[cfg(not(feature = "minilm-cpu"))]
    #[test]
    fn local_bundle_is_rejected_when_minilm_support_is_not_compiled() {
        let cli = parse_cli(&[
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--embedding-provider",
            "minilm",
            "--embedding-bundle-path",
            "/synthetic/cache",
            "--embedding-bundle-id",
            "synthetic-bundle",
            "bridge",
        ]);
        let error = match embedder(&cli.embedding) {
            Ok(_) => panic!("feature-off build must not load a model"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("minilm-cpu"));
    }
}
