//! Muninn command-line entry point and composition root.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use muninn_application::{MemoryClient, MemoryOptions, MemoryPorts};
use muninn_bridge::BridgeState;
use muninn_core::extraction::EntityExtractor;
use muninn_core::ports::{Embedder, GraphMaintenance};
use muninn_neo4j::{
    Neo4jClient, Neo4jConfig, Neo4jCypherQuery, Neo4jLongTermStore, Neo4jMaintenance,
    Neo4jReasoningStore, Neo4jShortTermStore, SchemaManager,
};
use muninn_providers::{HashEmbedder, OpenAiEmbedder, SpacyEntityExtractor};
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
    #[command(flatten)]
    extraction: ExtractionArgs,
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

#[derive(Clone, Copy, ValueEnum)]
enum ExtractionProvider {
    /// Disabled: `extraction_mode: auto` links no entity and contacts no endpoint.
    None,
    /// Local spaCy `en_core_web_sm` sidecar behind the HTTP adapter.
    Spacy,
}

#[derive(Args)]
struct ExtractionArgs {
    #[arg(
        long,
        env = "NAM_EXTRACTION__PROVIDER",
        value_enum,
        default_value = "none"
    )]
    extraction_provider: ExtractionProvider,
    /// Trusted local sidecar `POST /extract` endpoint; required with provider `spacy`.
    #[arg(long, env = "NAM_EXTRACTION__ENDPOINT")]
    extraction_endpoint: Option<String>,
    /// Per-request extraction deadline in seconds; zero is rejected.
    #[arg(long, env = "NAM_EXTRACTION__TIMEOUT_SECS", default_value_t = 10)]
    extraction_timeout_secs: u64,
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

/// Build the configured extraction stage. The default provider adds nothing,
/// so ingestion keeps today's no-extraction behavior and contacts no endpoint.
fn extractor(args: &ExtractionArgs) -> Result<Option<Arc<dyn EntityExtractor>>> {
    match args.extraction_provider {
        ExtractionProvider::None => Ok(None),
        ExtractionProvider::Spacy => {
            let endpoint = args.extraction_endpoint.as_deref().context(
                "spaCy extraction requires --extraction-endpoint naming the trusted local sidecar",
            )?;
            let stage = SpacyEntityExtractor::new(
                endpoint,
                Duration::from_secs(args.extraction_timeout_secs),
            )
            .context(
                "spaCy extraction configuration rejected: use an http(s) endpoint without \
                 credentials and a nonzero timeout",
            )?;
            Ok(Some(Arc::new(stage)))
        }
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
    let extraction = extractor(&cli.extraction)?;
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
            let mut memory = MemoryClient::new(
                MemoryPorts {
                    short_term: Arc::new(Neo4jShortTermStore::new(client.clone())),
                    long_term: Arc::new(Neo4jLongTermStore::new(client.clone())),
                    reasoning: Arc::new(Neo4jReasoningStore::new(client.clone())),
                    embedder,
                    cypher_query: Some(Arc::new(Neo4jCypherQuery::new(client.clone()))),
                },
                &MemoryOptions::default(),
            );
            if let Some(extraction) = extraction {
                tracing::info!("entity extraction stage is configured");
                memory = memory.with_extractor(extraction);
            }
            let memory = Arc::new(memory);
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
    use clap::CommandFactory;

    fn parse_cli(arguments: &[&str]) -> Cli {
        Cli::try_parse_from(arguments).expect("synthetic CLI arguments must parse")
    }

    #[test]
    fn cli_accepts_the_documented_extraction_configuration() {
        Cli::try_parse_from([
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--extraction-provider",
            "spacy",
            "--extraction-endpoint",
            "http://127.0.0.1:8765/extract",
            "--extraction-timeout-secs",
            "5",
            "bridge",
        ])
        .expect("the documented extraction configuration must be accepted");
    }

    #[test]
    fn help_documents_the_extraction_options() {
        let help = Cli::command().render_help().to_string();
        for text in [
            "--extraction-provider",
            "--extraction-endpoint",
            "--extraction-timeout-secs",
        ] {
            assert!(help.contains(text), "{text} missing from help:\n{help}");
        }
        assert!(help.contains("spacy"), "spacy value missing:\n{help}");
    }

    /// Minimal loopback HTTP stand-in for the spaCy sidecar: no model, no
    /// egress, one canned response per request.
    struct FakeSidecar {
        endpoint: String,
        received: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl FakeSidecar {
        fn start(body: &'static str) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback bind");
            let endpoint = format!("http://{}/extract", listener.local_addr().unwrap());
            let received = Arc::new(std::sync::Mutex::new(Vec::new()));
            let recorded = received.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { return };
                    let request = read_http_request(&mut stream);
                    recorded.lock().unwrap().push(request);
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                    stream.flush().unwrap();
                }
            });
            Self { endpoint, received }
        }

        fn requests(&self) -> Vec<String> {
            self.received.lock().unwrap().clone()
        }
    }

    fn read_http_request(stream: &mut std::net::TcpStream) -> String {
        use std::io::Read;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                return String::from_utf8_lossy(&buffer).into_owned();
            }
            buffer.extend_from_slice(&chunk[..read]);
            let Some(head_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
            let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
            let length = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            while buffer.len() < head_end + 4 + length {
                let read = stream.read(&mut chunk).unwrap();
                if read == 0 {
                    break;
                }
                buffer.extend_from_slice(&chunk[..read]);
            }
            return String::from_utf8_lossy(&buffer).into_owned();
        }
    }

    fn spacy_args(endpoint: &str) -> Vec<&str> {
        vec![
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--extraction-provider",
            "spacy",
            "--extraction-endpoint",
            endpoint,
            "bridge",
        ]
    }

    #[test]
    fn extraction_is_disabled_by_default_and_contacts_no_endpoint() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        let endpoint = format!("http://{}/extract", listener.local_addr().unwrap());
        let cli = parse_cli(&[
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--extraction-endpoint",
            &endpoint,
            "bridge",
        ]);
        assert!(matches!(
            cli.extraction.extraction_provider,
            ExtractionProvider::None
        ));
        assert_eq!(cli.extraction.extraction_timeout_secs, 10);
        assert!(extractor(&cli.extraction).unwrap().is_none());
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[tokio::test]
    async fn configured_spacy_extraction_reaches_the_sidecar_contract() {
        let sidecar = FakeSidecar::start(
            r#"{"entities":[{"name":"Alice","type":"PERSON","subtype":null,"start_pos":0,"end_pos":5,"confidence":0.85,"extractor":"spacy"}],"relations":[],"preferences":[],"source_text":null}"#,
        );
        let cli = parse_cli(&spacy_args(&sidecar.endpoint));
        let stage = extractor(&cli.extraction)
            .unwrap()
            .expect("a configured provider must build the extractor");
        let result = stage
            .extract(
                "Alice in Paris",
                &muninn_core::extraction::ExtractOptions {
                    extract_relations: Some(false),
                    extract_preferences: Some(false),
                    ..Default::default()
                },
            )
            .await
            .expect("the fake sidecar returns a valid extraction");
        assert_eq!(result.entities.len(), 1);
        let entity = &result.entities[0];
        assert_eq!(
            (entity.name.as_str(), entity.entity_type.as_str()),
            ("Alice", "PERSON")
        );
        assert_eq!((entity.start_pos, entity.end_pos), (Some(0), Some(5)));
        assert_eq!(entity.confidence, 0.85);
        assert_eq!(entity.extractor.as_deref(), Some("spacy"));
        let requests = sidecar.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        assert!(
            requests[0].starts_with("POST /extract HTTP/1.1"),
            "{}",
            requests[0]
        );
        let body = requests[0]
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("request body");
        for expected in ["\"text\":\"Alice in Paris\"", "\"entity_types\":null"] {
            assert!(body.contains(expected), "{body}");
        }
    }

    #[test]
    fn invalid_extraction_endpoint_fails_fast_without_disclosure() {
        for endpoint in [
            "http://synthetic-user:synthetic-secret@127.0.0.1:18765/extract",
            "not-a-url",
            "file:///tmp/extract",
        ] {
            let cli = parse_cli(&spacy_args(endpoint));
            let error = extractor(&cli.extraction)
                .err()
                .expect("invalid extraction configuration must be rejected");
            let message = format!("{error:#}");
            assert!(message.contains("spaCy extraction"), "{message}");
            for secret in ["synthetic-secret", endpoint] {
                assert!(!message.contains(secret), "{message}");
            }
        }
    }

    #[test]
    fn missing_extraction_endpoint_names_the_option() {
        let arguments = [
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--extraction-provider",
            "spacy",
            "bridge",
        ];
        let cli = parse_cli(&arguments);
        let error = extractor(&cli.extraction)
            .err()
            .expect("invalid extraction configuration must be rejected");
        assert!(format!("{error:#}").contains("--extraction-endpoint"));
    }

    #[test]
    fn zero_extraction_timeout_is_rejected() {
        let cli = parse_cli(&[
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--extraction-provider",
            "spacy",
            "--extraction-endpoint",
            "http://127.0.0.1:18765/extract",
            "--extraction-timeout-secs",
            "0",
            "bridge",
        ]);
        let error = extractor(&cli.extraction)
            .err()
            .expect("zero timeout must be rejected");
        assert!(format!("{error:#}").contains("spaCy extraction"));
    }

    #[tokio::test]
    async fn unreachable_extraction_endpoint_fails_the_request() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        let endpoint = format!("http://{}/extract", listener.local_addr().unwrap());
        drop(listener);
        let cli = parse_cli(&spacy_args(&endpoint));
        let stage = extractor(&cli.extraction).unwrap().expect("adapter");
        let error = stage
            .extract("Alice in Paris", &Default::default())
            .await
            .unwrap_err();
        assert!(matches!(error, muninn_core::MemoryError::Provider(_)));
        assert_eq!(
            error.to_string(),
            "provider error: spaCy extraction transport failed"
        );
    }

    #[test]
    fn bridge_command_defaults_are_unchanged() {
        let cli = parse_cli(&[
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "bridge",
        ]);
        match cli.command {
            Command::Bridge {
                port,
                host,
                allow_destructive_clear,
            } => {
                assert_eq!(port, 3001);
                assert_eq!(host, "127.0.0.1");
                assert!(!allow_destructive_clear);
            }
            Command::Schema => panic!("bridge invocation must not select schema"),
        }
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
