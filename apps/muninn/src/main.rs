//! Muninn command-line entry point and composition root.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use muninn_application::enrichment::{EnrichmentService, GeocodingService};
use muninn_application::{MemoryClient, MemoryOptions, MemoryPorts};
use muninn_bridge::BridgeState;
use muninn_core::enrichment::{
    ApiKey, EnrichmentConfig, EnrichmentProvider, EnrichmentProviderKind, EntityEnrichmentStore,
    Geocoder, GeocodingConfig, GeocodingProviderKind,
};
use muninn_core::extraction::EntityExtractor;
use muninn_core::ports::{Embedder, GraphMaintenance};
use muninn_neo4j::{
    Neo4jClient, Neo4jConfig, Neo4jCypherQuery, Neo4jLongTermStore, Neo4jMaintenance,
    Neo4jReasoningStore, Neo4jShortTermStore, SchemaManager,
};
use muninn_providers::{
    DiffbotEnrichment, HashEmbedder, NominatimGeocoder, OpenAiEmbedder, SpacyEntityExtractor,
    WikimediaEnrichment,
};
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
    #[command(flatten)]
    enrichment: EnrichmentArgs,
    #[command(flatten)]
    geocoding: GeocodingArgs,
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

/// Upstream `EnrichmentConfig` surface. Disabled by default: nothing is reachable
/// and no stage is built until `--enrichment-enabled` is set.
#[derive(Args)]
struct EnrichmentArgs {
    /// Opt in to entity enrichment; false contacts no provider.
    #[arg(long, env = "NAM_ENRICHMENT__ENABLED", default_value_t = false)]
    enrichment_enabled: bool,
    /// Provider priority order, comma separated; `none` selects nothing.
    #[arg(long, env = "NAM_ENRICHMENT__PROVIDERS", default_value = "wikimedia")]
    enrichment_providers: String,
    /// Wikimedia REST root override, e.g. `http://127.0.0.1:8765/api/rest_v1`.
    #[arg(long, env = "NAM_ENRICHMENT__WIKIMEDIA_BASE_URL")]
    enrichment_wikimedia_base_url: Option<String>,
    /// Diffbot knowledge-graph root override.
    #[arg(long, env = "NAM_ENRICHMENT__DIFFBOT_BASE_URL")]
    enrichment_diffbot_base_url: Option<String>,
    /// Required only when the `diffbot` provider is selected.
    #[arg(long, env = "NAM_ENRICHMENT__DIFFBOT_API_KEY", hide_env_values = true)]
    enrichment_diffbot_api_key: Option<String>,
    /// Per-request provider deadline in seconds; zero is rejected.
    #[arg(long, env = "NAM_ENRICHMENT__TIMEOUT_SECS", default_value_t = 10)]
    enrichment_timeout_secs: u64,
}

#[derive(Clone, Copy, ValueEnum)]
enum GeocodingProvider {
    /// OpenStreetMap Nominatim `GET /search`; the declared user agent is required.
    Nominatim,
    /// Configuration is validated; no adapter is implemented.
    Google,
}

/// Upstream `GeocodingConfig` surface. Disabled by default, like enrichment.
#[derive(Args)]
struct GeocodingArgs {
    /// Opt in to `LOCATION` geocoding; false contacts no provider.
    #[arg(long, env = "NAM_GEOCODING__ENABLED", default_value_t = false)]
    geocoding_enabled: bool,
    #[arg(
        long,
        env = "NAM_GEOCODING__PROVIDER",
        value_enum,
        default_value = "nominatim"
    )]
    geocoding_provider: GeocodingProvider,
    /// Nominatim root override, e.g. `http://127.0.0.1:8766`.
    #[arg(long, env = "NAM_GEOCODING__BASE_URL")]
    geocoding_base_url: Option<String>,
    /// Required only when the `google` provider is selected.
    #[arg(long, env = "NAM_GEOCODING__API_KEY", hide_env_values = true)]
    geocoding_api_key: Option<String>,
    /// Upstream applies this value as seconds between requests, not per second.
    #[arg(
        long,
        env = "NAM_GEOCODING__RATE_LIMIT_PER_SECOND",
        default_value_t = 1.0
    )]
    geocoding_rate_limit_per_second: f64,
    /// Per-request geocoder deadline in seconds; zero is rejected.
    #[arg(long, env = "NAM_GEOCODING__TIMEOUT_SECS", default_value_t = 10)]
    geocoding_timeout_secs: u64,
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

/// Closed provider vocabulary of `EnrichmentConfig.providers`; an unknown name is
/// rejected instead of adopted, and the message never echoes the input.
fn enrichment_providers(names: &str) -> Result<Vec<EnrichmentProviderKind>> {
    names
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            EnrichmentProviderKind::from_name(name).context(
                "enrichment configuration rejected: --enrichment-providers must name wikimedia, \
                 diffbot or none",
            )
        })
        .collect()
}

/// One adapter per configured provider, in the configured priority order.
/// Building an adapter contacts nothing; the caller supplies the property writer.
fn enrichment_provider(
    kind: EnrichmentProviderKind,
    args: &EnrichmentArgs,
    config: &EnrichmentConfig,
) -> Result<Arc<dyn EnrichmentProvider>> {
    let rate = config.rate_limit(kind).unwrap_or(1.0);
    let timeout = Duration::from_secs(args.enrichment_timeout_secs);
    const REJECT: &str = "enrichment configuration rejected: use an http(s) provider endpoint \
                          without credentials and a nonzero timeout";
    Ok(match kind {
        EnrichmentProviderKind::Wikimedia => {
            let adapter = args
                .enrichment_wikimedia_base_url
                .as_deref()
                .map_or_else(
                    || {
                        WikimediaEnrichment::for_language(
                            &config.language,
                            &config.user_agent,
                            rate,
                            timeout,
                        )
                    },
                    |base| WikimediaEnrichment::new(base, &config.user_agent, rate, timeout),
                )
                .context(REJECT)?;
            Arc::new(adapter)
        }
        EnrichmentProviderKind::Diffbot => {
            let base = args
                .enrichment_diffbot_base_url
                .as_deref()
                .unwrap_or(DiffbotEnrichment::DEFAULT_BASE);
            let key = config.diffbot_api_key.as_deref().context(
                "enrichment configuration rejected: enrichment.diffbot_api_key is \
                          required when the diffbot provider is selected",
            )?;
            Arc::new(DiffbotEnrichment::new(base, key, rate, timeout).context(REJECT)?)
        }
        EnrichmentProviderKind::None => bail!(
            "enrichment configuration rejected: 'none' selects no enrichment provider adapter"
        ),
    })
}

/// Validated enrichment configuration and the adapters it selects. The default
/// path returns `None`: no stage is built and no endpoint is contacted.
type EnrichmentStage = (EnrichmentConfig, Vec<Arc<dyn EnrichmentProvider>>);

fn enrichment(args: &EnrichmentArgs) -> Result<Option<EnrichmentStage>> {
    let mut config = EnrichmentConfig {
        enabled: args.enrichment_enabled,
        providers: enrichment_providers(&args.enrichment_providers)?,
        ..EnrichmentConfig::default()
    };
    if let Some(key) = args.enrichment_diffbot_api_key.as_deref() {
        config.diffbot_api_key = ApiKey::new(key);
    }
    config
        .validate()
        .context("enrichment configuration rejected")?;
    if !config.is_active() {
        return Ok(None);
    }
    let mut adapters: Vec<Arc<dyn EnrichmentProvider>> = Vec::new();
    for kind in config.providers.clone() {
        if kind != EnrichmentProviderKind::None {
            adapters.push(enrichment_provider(kind, args, &config)?);
        }
    }
    Ok(Some((config, adapters)))
}

/// Validated geocoding configuration and the adapter it selects. The default path
/// returns `None` and contacts nothing.
fn geocoding(args: &GeocodingArgs) -> Result<Option<(GeocodingConfig, Arc<dyn Geocoder>)>> {
    let mut config = GeocodingConfig {
        enabled: args.geocoding_enabled,
        provider: match args.geocoding_provider {
            GeocodingProvider::Nominatim => GeocodingProviderKind::Nominatim,
            GeocodingProvider::Google => GeocodingProviderKind::Google,
        },
        rate_limit_per_second: args.geocoding_rate_limit_per_second,
        ..GeocodingConfig::default()
    };
    if let Some(key) = args.geocoding_api_key.as_deref() {
        config.api_key = ApiKey::new(key);
    }
    config
        .validate()
        .context("geocoding configuration rejected")?;
    if !config.is_active() {
        return Ok(None);
    }
    match config.provider {
        GeocodingProviderKind::Nominatim => {
            let base = args
                .geocoding_base_url
                .as_deref()
                .unwrap_or(NominatimGeocoder::DEFAULT_BASE);
            let adapter = NominatimGeocoder::new(
                base,
                &config.user_agent,
                config.rate_limit_per_second,
                Duration::from_secs(args.geocoding_timeout_secs),
            )
            .context(
                "geocoding configuration rejected: use an http(s) endpoint without credentials \
                 and a nonzero timeout",
            )?;
            Ok(Some((config, Arc::new(adapter))))
        }
        GeocodingProviderKind::Google => bail!(
            "geocoding configuration rejected: the google provider is validated but not \
             implemented; configure nominatim"
        ),
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
    let enrichment = enrichment(&cli.enrichment)?;
    let geocoding = geocoding(&cli.geocoding)?;
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
            if let Some((config, providers)) = enrichment {
                tracing::info!("entity enrichment stage is configured");
                let writer: Arc<dyn EntityEnrichmentStore> =
                    Arc::new(Neo4jLongTermStore::new(client.clone()));
                let service = EnrichmentService::new(config, providers, Some(writer))
                    .context("enrichment configuration rejected")?;
                memory = memory.with_enrichment(Arc::new(service));
            }
            if let Some((config, geocoder)) = geocoding {
                tracing::info!("entity geocoding stage is configured");
                let writer: Arc<dyn EntityEnrichmentStore> =
                    Arc::new(Neo4jLongTermStore::new(client.clone()));
                let service = GeocodingService::new(config, geocoder, Some(writer))
                    .context("geocoding configuration rejected")?;
                memory = memory.with_geocoding(Arc::new(service));
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
    fn cli_accepts_the_documented_enrichment_and_geocoding_configuration() {
        Cli::try_parse_from([
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--enrichment-enabled",
            "--enrichment-providers",
            "wikimedia",
            "--enrichment-wikimedia-base-url",
            "http://127.0.0.1:18765/api/rest_v1",
            "--geocoding-enabled",
            "--geocoding-provider",
            "nominatim",
            "--geocoding-base-url",
            "http://127.0.0.1:18766/search",
            "--geocoding-rate-limit-per-second",
            "0.5",
            "bridge",
        ])
        .expect("the documented enrichment and geocoding configuration must be accepted");
    }

    #[test]
    fn help_documents_the_enrichment_and_geocoding_options() {
        let help = Cli::command().render_help().to_string();
        for text in [
            "--enrichment-enabled",
            "--enrichment-providers",
            "--enrichment-wikimedia-base-url",
            "--enrichment-diffbot-base-url",
            "--enrichment-diffbot-api-key",
            "--enrichment-timeout-secs",
            "--geocoding-enabled",
            "--geocoding-provider",
            "--geocoding-base-url",
            "--geocoding-api-key",
            "--geocoding-rate-limit-per-second",
            "--geocoding-timeout-secs",
        ] {
            assert!(help.contains(text), "{text} missing from help:\n{help}");
        }
        assert!(
            help.contains("wikimedia"),
            "wikimedia value missing:\n{help}"
        );
        assert!(
            help.contains("nominatim"),
            "nominatim value missing:\n{help}"
        );
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

    /// Records enrichment writes so the binary-level test never touches a store.
    #[derive(Default)]
    struct RecordingWriter {
        enrichments: std::sync::Mutex<Vec<uuid::Uuid>>,
    }

    #[async_trait::async_trait]
    impl EntityEnrichmentStore for RecordingWriter {
        async fn write_enrichment(
            &self,
            entity_id: uuid::Uuid,
            _: &muninn_core::enrichment::EnrichmentAttributes,
        ) -> muninn_core::MemoryResult<bool> {
            self.enrichments.lock().unwrap().push(entity_id);
            Ok(true)
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

    #[tokio::test]
    async fn configured_wikimedia_enrichment_reaches_the_provider_contract() {
        let body = r#"{"type":"standard","title":"Ada Lovelace","extract":"mathematician","description":null,"wikibase_item":"Q7259","thumbnail":null,"content_urls":null}"#;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback bind");
        let address = listener.local_addr().unwrap();
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
        let base = format!("http://{address}/api/rest_v1");
        let cli = parse_cli(&[
            "muninn",
            "--neo4j-password",
            "synthetic-test-password",
            "--enrichment-enabled",
            "--enrichment-providers",
            "wikimedia",
            "--enrichment-wikimedia-base-url",
            base.as_str(),
            "bridge",
        ]);
        let Some((config, providers)) = enrichment(&cli.enrichment)
            .unwrap()
            .filter(|(config, _)| config.is_active())
        else {
            panic!("a configured provider must build the enrichment stage");
        };
        let service = EnrichmentService::new(
            config,
            providers,
            Some(Arc::new(RecordingWriter::default())),
        )
        .expect("the recorded configuration is valid");
        let entity = muninn_core::model::Entity {
            id: uuid::Uuid::new_v4(),
            name: "Ada Lovelace".to_owned(),
            canonical_name: Some("Ada Lovelace".to_owned()),
            entity_type: "PERSON".to_owned(),
            subtype: None,
            description: None,
            confidence: 1.0,
            aliases: Vec::new(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: None,
            metadata: muninn_core::model::Metadata::new(),
        };
        let outcome = service.enrich(&entity).await.expect("the provider answers");
        let attributes = outcome
            .attributes
            .expect("a found summary carries the documented attributes");
        assert_eq!(
            attributes.enriched_summary.as_deref(),
            Some("mathematician")
        );
        assert_eq!(attributes.enriched_description, None);
        assert_eq!(attributes.wikidata_id.as_deref(), Some("Q7259"));
        assert_eq!(attributes.enrichment_provider, "wikimedia");
        let requests = received.lock().unwrap().clone();
        assert_eq!(requests.len(), 1, "{requests:?}");
        assert!(
            requests[0].starts_with("GET /api/rest_v1/page/summary/Ada_Lovelace HTTP/1.1"),
            "{}",
            requests[0]
        );
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
