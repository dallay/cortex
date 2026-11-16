//! Configuration settings mirroring `neo4j_agent_memory.config.settings`.
//!
//! Equivalent to the Pydantic models in `config/settings.py` (1,071 lines).
//! Rust adaptation: no `BaseSettings` / env-var loading — callers construct
//! structs directly or deserialize from JSON/YAML.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::MemoryError;

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// Supported embedding providers (`config.settings.EmbeddingProvider`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingProvider {
    #[default]
    Openai,
    Anthropic,
    Ollama,
    SentenceTransformers,
    VertexAi,
    Bedrock,
    Custom,
}

impl EmbeddingProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
            Self::SentenceTransformers => "sentence_transformers",
            Self::VertexAi => "vertex_ai",
            Self::Bedrock => "bedrock",
            Self::Custom => "custom",
        }
    }
}

impl fmt::Display for EmbeddingProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EmbeddingProvider {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::Openai),
            "anthropic" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            "sentence_transformers" => Ok(Self::SentenceTransformers),
            "vertex_ai" => Ok(Self::VertexAi),
            "bedrock" => Ok(Self::Bedrock),
            "custom" => Ok(Self::Custom),
            other => Err(MemoryError::invalid(format!(
                "unknown embedding provider '{other}'"
            ))),
        }
    }
}

/// Supported LLM providers for extraction (`config.settings.LLMProvider`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LLMProvider {
    #[default]
    Openai,
    Anthropic,
    Ollama,
    Custom,
}

impl LLMProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Openai => "openai",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
            Self::Custom => "custom",
        }
    }
}

impl fmt::Display for LLMProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LLMProvider {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "openai" => Ok(Self::Openai),
            "anthropic" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            other => Err(MemoryError::invalid(format!(
                "unknown LLM provider '{other}'"
            ))),
        }
    }
}

/// Supported entity extractor types (`config.settings.ExtractorType`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractorType {
    #[default]
    Llm,
    Gliner,
    Spacy,
    Pipeline,
    None,
}

impl ExtractorType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Llm => "llm",
            Self::Gliner => "gliner",
            Self::Spacy => "spacy",
            Self::Pipeline => "pipeline",
            Self::None => "none",
        }
    }
}

impl fmt::Display for ExtractorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ExtractorType {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "llm" => Ok(Self::Llm),
            "gliner" => Ok(Self::Gliner),
            "spacy" => Ok(Self::Spacy),
            "pipeline" => Ok(Self::Pipeline),
            "none" => Ok(Self::None),
            other => Err(MemoryError::invalid(format!(
                "unknown extractor type '{other}'"
            ))),
        }
    }
}

/// Strategies for merging extraction results (`config.settings.MergeStrategy`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    #[default]
    Union,
    Intersection,
    Confidence,
    Cascade,
}

impl MergeStrategy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Union => "union",
            Self::Intersection => "intersection",
            Self::Confidence => "confidence",
            Self::Cascade => "cascade",
        }
    }
}

impl fmt::Display for MergeStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MergeStrategy {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "union" => Ok(Self::Union),
            "intersection" => Ok(Self::Intersection),
            "confidence" => Ok(Self::Confidence),
            "cascade" => Ok(Self::Cascade),
            other => Err(MemoryError::invalid(format!(
                "unknown merge strategy '{other}'"
            ))),
        }
    }
}

/// Available schema models for entity types (`config.settings.SchemaModel`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaModel {
    #[default]
    Poleo,
    Legacy,
    Custom,
}

impl SchemaModel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Poleo => "poleo",
            Self::Legacy => "legacy",
            Self::Custom => "custom",
        }
    }
}

impl fmt::Display for SchemaModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SchemaModel {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "poleo" => Ok(Self::Poleo),
            "legacy" => Ok(Self::Legacy),
            "custom" => Ok(Self::Custom),
            other => Err(MemoryError::invalid(format!(
                "unknown schema model '{other}'"
            ))),
        }
    }
}

/// Supported entity resolution strategies (`config.settings.ResolverStrategy`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolverStrategy {
    #[default]
    Exact,
    Fuzzy,
    Semantic,
    Composite,
    None,
}

impl ResolverStrategy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Fuzzy => "fuzzy",
            Self::Semantic => "semantic",
            Self::Composite => "composite",
            Self::None => "none",
        }
    }
}

impl fmt::Display for ResolverStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ResolverStrategy {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "exact" => Ok(Self::Exact),
            "fuzzy" => Ok(Self::Fuzzy),
            "semantic" => Ok(Self::Semantic),
            "composite" => Ok(Self::Composite),
            "none" => Ok(Self::None),
            other => Err(MemoryError::invalid(format!(
                "unknown resolver strategy '{other}'"
            ))),
        }
    }
}

/// Supported geocoding providers (`config.settings.GeocodingProvider`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GeocodingProvider {
    #[default]
    Nominatim,
    Google,
}

impl GeocodingProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Nominatim => "nominatim",
            Self::Google => "google",
        }
    }
}

impl fmt::Display for GeocodingProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for GeocodingProvider {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "nominatim" => Ok(Self::Nominatim),
            "google" => Ok(Self::Google),
            other => Err(MemoryError::invalid(format!(
                "unknown geocoding provider '{other}'"
            ))),
        }
    }
}

/// Supported enrichment providers (`config.settings.EnrichmentProvider`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrichmentProvider {
    #[default]
    Wikimedia,
    Diffbot,
    None,
}

impl EnrichmentProvider {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wikimedia => "wikimedia",
            Self::Diffbot => "diffbot",
            Self::None => "none",
        }
    }
}

impl fmt::Display for EnrichmentProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EnrichmentProvider {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "wikimedia" => Ok(Self::Wikimedia),
            "diffbot" => Ok(Self::Diffbot),
            "none" => Ok(Self::None),
            other => Err(MemoryError::invalid(format!(
                "unknown enrichment provider '{other}'"
            ))),
        }
    }
}

/// Storage backend selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Bolt,
    Nams,
}

impl Backend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bolt => "bolt",
            Self::Nams => "nams",
        }
    }
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Backend {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "bolt" => Ok(Self::Bolt),
            "nams" => Ok(Self::Nams),
            other => Err(MemoryError::invalid(format!("unknown backend '{other}'"))),
        }
    }
}

/// Write mode for buffered writes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    #[default]
    Sync,
    Buffered,
}

impl WriteMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Buffered => "buffered",
        }
    }
}

impl fmt::Display for WriteMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for WriteMode {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "sync" => Ok(Self::Sync),
            "buffered" => Ok(Self::Buffered),
            other => Err(MemoryError::invalid(format!(
                "unknown write mode '{other}'"
            ))),
        }
    }
}

/// Ontology validation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationMode {
    Permissive,
    Strict,
}

impl ValidationMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Permissive => "permissive",
            Self::Strict => "strict",
        }
    }
}

impl fmt::Display for ValidationMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ValidationMode {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "permissive" => Ok(Self::Permissive),
            "strict" => Ok(Self::Strict),
            other => Err(MemoryError::invalid(format!(
                "unknown validation mode '{other}'"
            ))),
        }
    }
}

/// Resolution scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionScope {
    #[default]
    Global,
    User,
}

impl ResolutionScope {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::User => "user",
        }
    }
}

impl fmt::Display for ResolutionScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for ResolutionScope {
    type Err = MemoryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_ascii_lowercase().as_str() {
            "global" => Ok(Self::Global),
            "user" => Ok(Self::User),
            other => Err(MemoryError::invalid(format!(
                "unknown resolution scope '{other}'"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// Config structs
// ---------------------------------------------------------------------------

/// Neo4j connection configuration (`config.settings.Neo4jConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Neo4jConfig {
    /// Neo4j connection URI.
    #[serde(default = "default_neo4j_uri")]
    pub uri: String,
    /// Neo4j username.
    #[serde(default = "default_neo4j_username")]
    pub username: String,
    /// Neo4j password.
    pub password: String,
    /// Neo4j database name.
    #[serde(default = "default_neo4j_database")]
    pub database: String,
    /// Maximum connection pool size.
    #[serde(default = "default_neo4j_max_pool_size")]
    pub max_connection_pool_size: u32,
    /// Connection timeout in seconds.
    #[serde(default = "default_neo4j_connection_timeout")]
    pub connection_timeout: f64,
    /// Maximum transaction retry time in seconds.
    #[serde(default = "default_neo4j_max_retry_time")]
    pub max_transaction_retry_time: f64,
    /// Maximum lifetime of a pooled connection in seconds.
    #[serde(default = "default_neo4j_max_lifetime")]
    pub max_connection_lifetime: u64,
    /// Seconds a connection can be idle before liveness check.
    #[serde(default = "default_neo4j_liveness_timeout")]
    pub liveness_check_timeout: u64,
    /// Enable TCP keep-alive on connections.
    #[serde(default = "default_neo4j_keep_alive")]
    pub keep_alive: bool,
}

impl Default for Neo4jConfig {
    fn default() -> Self {
        Self {
            uri: default_neo4j_uri(),
            username: default_neo4j_username(),
            password: String::new(),
            database: default_neo4j_database(),
            max_connection_pool_size: default_neo4j_max_pool_size(),
            connection_timeout: default_neo4j_connection_timeout(),
            max_transaction_retry_time: default_neo4j_max_retry_time(),
            max_connection_lifetime: default_neo4j_max_lifetime(),
            liveness_check_timeout: default_neo4j_liveness_timeout(),
            keep_alive: default_neo4j_keep_alive(),
        }
    }
}

fn default_neo4j_uri() -> String {
    "bolt://localhost:7687".to_owned()
}
fn default_neo4j_username() -> String {
    "neo4j".to_owned()
}
fn default_neo4j_database() -> String {
    "neo4j".to_owned()
}
fn default_neo4j_max_pool_size() -> u32 {
    50
}
fn default_neo4j_connection_timeout() -> f64 {
    30.0
}
fn default_neo4j_max_retry_time() -> f64 {
    30.0
}
fn default_neo4j_max_lifetime() -> u64 {
    300
}
fn default_neo4j_liveness_timeout() -> u64 {
    60
}
fn default_neo4j_keep_alive() -> bool {
    true
}

/// Embedding provider configuration (`config.settings.EmbeddingConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// Embedding provider to use.
    #[serde(default)]
    pub provider: EmbeddingProvider,
    /// Embedding model name.
    #[serde(default = "default_embedding_model")]
    pub model: String,
    /// Embedding dimensions.
    #[serde(default = "default_embedding_dimensions")]
    pub dimensions: usize,
    /// API key for embedding provider.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Batch size for embeddings.
    #[serde(default = "default_embedding_batch_size")]
    pub batch_size: usize,
    /// Device for sentence transformers (cpu/cuda/mps).
    #[serde(default = "default_embedding_device")]
    pub device: String,
    /// GCP project ID for Vertex AI.
    #[serde(default)]
    pub project_id: Option<String>,
    /// GCP region for Vertex AI.
    #[serde(default = "default_vertex_location")]
    pub location: String,
    /// Vertex AI task type.
    #[serde(default = "default_vertex_task_type")]
    pub task_type: String,
    /// Vertex AI output dimensionality.
    #[serde(default)]
    pub output_dimensionality: Option<usize>,
    /// AWS region for Bedrock.
    #[serde(default)]
    pub aws_region: Option<String>,
    /// AWS credentials profile name.
    #[serde(default)]
    pub aws_profile: Option<String>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: EmbeddingProvider::Openai,
            model: default_embedding_model(),
            dimensions: default_embedding_dimensions(),
            api_key: None,
            batch_size: default_embedding_batch_size(),
            device: default_embedding_device(),
            project_id: None,
            location: default_vertex_location(),
            task_type: default_vertex_task_type(),
            output_dimensionality: None,
            aws_region: None,
            aws_profile: None,
        }
    }
}

impl EmbeddingConfig {
    /// Apply Vertex AI defaults when the provider is Vertex AI.
    pub fn apply_vertex_ai_defaults(&mut self) {
        if self.provider != EmbeddingProvider::VertexAi {
            return;
        }
        // Default model for Vertex AI
        if self.model == "text-embedding-3-small" {
            self.model = "gemini-embedding-001".to_owned();
        }
        // Default output dimensionality for Vertex AI
        if self.output_dimensionality.is_none() {
            self.output_dimensionality = Some(768);
        }
        // Default dimensions from output_dimensionality
        if self.dimensions == 1536 {
            self.dimensions = self.output_dimensionality.unwrap_or(768);
        }
    }
}

fn default_embedding_model() -> String {
    "text-embedding-3-small".to_owned()
}
fn default_embedding_dimensions() -> usize {
    1536
}
fn default_embedding_batch_size() -> usize {
    100
}
fn default_embedding_device() -> String {
    "cpu".to_owned()
}
fn default_vertex_location() -> String {
    "us-central1".to_owned()
}
fn default_vertex_task_type() -> String {
    "RETRIEVAL_DOCUMENT".to_owned()
}

/// LLM provider configuration for extraction (`config.settings.LLMConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LLMConfig {
    /// LLM provider to use.
    #[serde(default)]
    pub provider: LLMProvider,
    /// LLM model name.
    #[serde(default = "default_llm_model")]
    pub model: String,
    /// API key for LLM provider.
    #[serde(default)]
    pub api_key: Option<String>,
    /// LLM temperature.
    #[serde(default)]
    pub temperature: f64,
    /// Maximum tokens for LLM.
    #[serde(default = "default_llm_max_tokens")]
    pub max_tokens: usize,
}

impl Default for LLMConfig {
    fn default() -> Self {
        Self {
            provider: LLMProvider::Openai,
            model: default_llm_model(),
            api_key: None,
            temperature: 0.0,
            max_tokens: default_llm_max_tokens(),
        }
    }
}

fn default_llm_model() -> String {
    "gpt-4o-mini".to_owned()
}
fn default_llm_max_tokens() -> usize {
    4096
}

/// Knowledge graph schema configuration (`config.settings.SchemaConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaConfig {
    /// Schema model to use.
    #[serde(default)]
    pub model: SchemaModel,
    /// Custom entity types.
    #[serde(default)]
    pub entity_types: Option<Vec<String>>,
    /// Whether to track entity subtypes.
    #[serde(default = "default_true")]
    pub enable_subtypes: bool,
    /// Whether to reject unknown entity types.
    #[serde(default)]
    pub strict_types: bool,
    /// Path to a schema definition file.
    #[serde(default)]
    pub custom_schema_path: Option<String>,
    /// Path to an ontology document.
    #[serde(default)]
    pub ontology_path: Option<String>,
    /// Whether to adopt the active stored ontology.
    #[serde(default = "default_true")]
    pub use_active_ontology: bool,
    /// Built-in ontology template name.
    #[serde(default = "default_ontology_template")]
    pub ontology_template: String,
    /// Ontology enforcement mode.
    #[serde(default)]
    pub validation_mode: Option<ValidationMode>,
    /// Whether to run one-shot v0.7 backfills.
    #[serde(default = "default_true")]
    pub backfill_relation_types: bool,
}

impl Default for SchemaConfig {
    fn default() -> Self {
        Self {
            model: SchemaModel::Poleo,
            entity_types: None,
            enable_subtypes: true,
            strict_types: false,
            custom_schema_path: None,
            ontology_path: None,
            use_active_ontology: true,
            ontology_template: default_ontology_template(),
            validation_mode: None,
            backfill_relation_types: true,
        }
    }
}

fn default_ontology_template() -> String {
    "poleo".to_owned()
}

/// Entity extraction configuration (`config.settings.ExtractionConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionConfig {
    /// Type of entity extractor.
    #[serde(default)]
    pub extractor_type: ExtractorType,
    /// Enable spaCy in extraction pipeline.
    #[serde(default = "default_true")]
    pub enable_spacy: bool,
    /// Enable GLiNER2.5 in the extraction pipeline.
    #[serde(default = "default_true")]
    pub enable_gliner: bool,
    /// Enable LLM as fallback in pipeline.
    #[serde(default = "default_true")]
    pub enable_llm_fallback: bool,
    /// Strategy for merging results from multiple extractors.
    #[serde(default)]
    pub merge_strategy: MergeStrategy,
    /// Continue to next stage if current stage returns no results.
    #[serde(default = "default_true")]
    pub fallback_on_empty: bool,
    /// spaCy model name.
    #[serde(default = "default_spacy_model")]
    pub spacy_model: String,
    /// Default confidence score for spaCy extractions.
    #[serde(default = "default_spacy_confidence")]
    pub spacy_confidence: f64,
    /// GLiNER2.5 checkpoint.
    #[serde(default = "default_gliner_model")]
    pub gliner_model: String,
    /// GLiNER2.5 entity confidence threshold.
    #[serde(default = "default_gliner_threshold")]
    pub gliner_threshold: f64,
    /// GLiNER2.5 relation confidence floor.
    #[serde(default)]
    pub gliner_relation_threshold: Option<f64>,
    /// Device for the GLiNER2.5 model.
    #[serde(default = "default_gliner_device")]
    pub gliner_device: String,
    /// Domain schema for GLiNER2.5 extraction.
    #[serde(default)]
    pub gliner_schema: Option<String>,
    /// Longest input decoded in one pass.
    #[serde(default = "default_gliner_max_words")]
    pub gliner_max_words: usize,
    /// Word overlap between windows for long input.
    #[serde(default = "default_gliner_chunk_overlap")]
    pub gliner_chunk_overlap: usize,
    /// Span-overlap policy for the attribute pass.
    #[serde(default)]
    pub gliner_overlap_policy: Option<String>,
    /// Run the opt-in second pass for enum properties.
    #[serde(default)]
    pub gliner_extract_attributes: bool,
    /// Load GLiNER2.5 weights in fp16.
    #[serde(default)]
    pub gliner_quantize: bool,
    /// Run GLiNER2.5 weights through torch.compile.
    #[serde(default)]
    pub gliner_compile: bool,
    /// LLM model for extraction.
    #[serde(default = "default_llm_model")]
    pub llm_model: String,
    /// Entity types to extract.
    #[serde(default = "default_entity_types")]
    pub entity_types: Vec<String>,
    /// Whether to extract relations.
    #[serde(default = "default_true")]
    pub extract_relations: bool,
    /// Whether to extract preferences.
    #[serde(default = "default_true")]
    pub extract_preferences: bool,
    /// Confidence floor for the pipeline's merged result.
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f64,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            extractor_type: ExtractorType::Pipeline,
            enable_spacy: true,
            enable_gliner: true,
            enable_llm_fallback: true,
            merge_strategy: MergeStrategy::Confidence,
            fallback_on_empty: true,
            spacy_model: default_spacy_model(),
            spacy_confidence: default_spacy_confidence(),
            gliner_model: default_gliner_model(),
            gliner_threshold: default_gliner_threshold(),
            gliner_relation_threshold: None,
            gliner_device: default_gliner_device(),
            gliner_schema: None,
            gliner_max_words: default_gliner_max_words(),
            gliner_chunk_overlap: default_gliner_chunk_overlap(),
            gliner_overlap_policy: None,
            gliner_extract_attributes: false,
            gliner_quantize: false,
            gliner_compile: false,
            llm_model: default_llm_model(),
            entity_types: default_entity_types(),
            extract_relations: true,
            extract_preferences: true,
            confidence_threshold: default_confidence_threshold(),
        }
    }
}

impl ExtractionConfig {
    /// Check whether this config requires an LLM provider.
    pub fn needs_llm(&self) -> bool {
        self.extractor_type == ExtractorType::Llm
            || (self.extractor_type == ExtractorType::Pipeline && self.enable_llm_fallback)
    }
}

fn default_true() -> bool {
    true
}
fn default_spacy_model() -> String {
    "en_core_web_sm".to_owned()
}
fn default_spacy_confidence() -> f64 {
    0.85
}
fn default_gliner_model() -> String {
    "fastino/gliner2.5-base-v1".to_owned()
}
fn default_gliner_threshold() -> f64 {
    0.5
}
fn default_gliner_device() -> String {
    "cpu".to_owned()
}
fn default_gliner_max_words() -> usize {
    384
}
fn default_gliner_chunk_overlap() -> usize {
    64
}
fn default_entity_types() -> Vec<String> {
    vec![
        "PERSON".to_owned(),
        "ORGANIZATION".to_owned(),
        "LOCATION".to_owned(),
        "EVENT".to_owned(),
        "OBJECT".to_owned(),
    ]
}
fn default_confidence_threshold() -> f64 {
    0.5
}

/// Entity resolution configuration (`config.settings.ResolutionConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolutionConfig {
    /// Resolution strategy.
    #[serde(default)]
    pub strategy: ResolverStrategy,
    /// Exact match threshold.
    #[serde(default = "default_exact_threshold")]
    pub exact_threshold: f64,
    /// Fuzzy match threshold.
    #[serde(default = "default_fuzzy_threshold")]
    pub fuzzy_threshold: f64,
    /// Semantic match threshold.
    #[serde(default = "default_semantic_threshold")]
    pub semantic_threshold: f64,
    /// Fuzzy matching scorer.
    #[serde(default = "default_fuzzy_scorer")]
    pub fuzzy_scorer: String,
    /// Resolve extracted mentions against stored entities while storing a message.
    #[serde(default = "default_true")]
    pub resolve_on_ingest: bool,
    /// Score at or above which a mention merges onto the matched entity.
    #[serde(default = "default_auto_merge_threshold")]
    pub auto_merge_threshold: f64,
    /// Score at or above which a mention is stored with a pending SAME_AS edge.
    #[serde(default = "default_review_threshold")]
    pub review_threshold: f64,
    /// Maximum blocking candidates fetched per mention and per bucket.
    #[serde(default = "default_candidate_limit")]
    pub candidate_limit: usize,
    /// Use EntityTypeDef.aliases as blocking keys.
    #[serde(default = "default_true")]
    pub use_alias_gazetteer: bool,
    /// Also generate candidates from the entity vector index.
    #[serde(default = "default_true")]
    pub use_embedding_blocking: bool,
    /// Half-width of the mention context window compared during scoring.
    #[serde(default = "default_context_window_chars")]
    pub context_window_chars: usize,
    /// Resolution scope.
    #[serde(default)]
    pub scope: ResolutionScope,
}

impl Default for ResolutionConfig {
    fn default() -> Self {
        Self {
            strategy: ResolverStrategy::Composite,
            exact_threshold: default_exact_threshold(),
            fuzzy_threshold: default_fuzzy_threshold(),
            semantic_threshold: default_semantic_threshold(),
            fuzzy_scorer: default_fuzzy_scorer(),
            resolve_on_ingest: true,
            auto_merge_threshold: default_auto_merge_threshold(),
            review_threshold: default_review_threshold(),
            candidate_limit: default_candidate_limit(),
            use_alias_gazetteer: true,
            use_embedding_blocking: true,
            context_window_chars: default_context_window_chars(),
            scope: ResolutionScope::Global,
        }
    }
}

impl ResolutionConfig {
    /// Validate that review_threshold <= auto_merge_threshold.
    pub fn validate(&self) -> Result<(), MemoryError> {
        if self.review_threshold > self.auto_merge_threshold {
            return Err(MemoryError::invalid(format!(
                "resolution.review_threshold ({}) must be <= resolution.auto_merge_threshold ({})",
                self.review_threshold, self.auto_merge_threshold
            )));
        }
        Ok(())
    }
}

fn default_exact_threshold() -> f64 {
    1.0
}
fn default_fuzzy_threshold() -> f64 {
    0.85
}
fn default_semantic_threshold() -> f64 {
    0.8
}
fn default_fuzzy_scorer() -> String {
    "token_sort_ratio".to_owned()
}
fn default_auto_merge_threshold() -> f64 {
    0.90
}
fn default_review_threshold() -> f64 {
    0.85
}
fn default_candidate_limit() -> usize {
    12
}
fn default_context_window_chars() -> usize {
    90
}

/// Memory behavior configuration (`config.settings.MemoryConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryBehaviorConfig {
    /// Default conversation message limit.
    #[serde(default = "default_conversation_limit")]
    pub default_conversation_limit: usize,
    /// Enable message embeddings.
    #[serde(default = "default_true")]
    pub message_embedding_enabled: bool,
    /// Preference confidence threshold.
    #[serde(default = "default_preference_confidence")]
    pub preference_confidence_threshold: f64,
    /// Enable fact deduplication.
    #[serde(default = "default_true")]
    pub fact_deduplication_enabled: bool,
    /// Enable reasoning trace embeddings.
    #[serde(default = "default_true")]
    pub trace_embedding_enabled: bool,
    /// Enable tool usage statistics.
    #[serde(default = "default_true")]
    pub tool_stats_enabled: bool,
    /// Multi-tenant mode.
    #[serde(default)]
    pub multi_tenant: bool,
    /// Write mode.
    #[serde(default)]
    pub write_mode: WriteMode,
    /// Maximum number of buffered writes in flight.
    #[serde(default = "default_max_pending")]
    pub max_pending: usize,
    /// Conversation TTL in days.
    #[serde(default)]
    pub conversation_ttl_days: Option<usize>,
    /// Read-audit preference.
    #[serde(default)]
    pub audit_read: bool,
}

impl Default for MemoryBehaviorConfig {
    fn default() -> Self {
        Self {
            default_conversation_limit: default_conversation_limit(),
            message_embedding_enabled: true,
            preference_confidence_threshold: default_preference_confidence(),
            fact_deduplication_enabled: true,
            trace_embedding_enabled: true,
            tool_stats_enabled: true,
            multi_tenant: false,
            write_mode: WriteMode::Sync,
            max_pending: default_max_pending(),
            conversation_ttl_days: None,
            audit_read: false,
        }
    }
}

fn default_conversation_limit() -> usize {
    50
}
fn default_preference_confidence() -> f64 {
    0.7
}
fn default_max_pending() -> usize {
    200
}

/// Search configuration (`config.settings.SearchConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchConfig {
    /// Default search limit.
    #[serde(default = "default_search_limit")]
    pub default_limit: usize,
    /// Default similarity threshold.
    #[serde(default = "default_search_threshold")]
    pub default_threshold: f64,
    /// Enable hybrid search.
    #[serde(default = "default_true")]
    pub hybrid_search_enabled: bool,
    /// Graph traversal depth for search.
    #[serde(default = "default_graph_depth")]
    pub graph_depth: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            default_limit: default_search_limit(),
            default_threshold: default_search_threshold(),
            hybrid_search_enabled: true,
            graph_depth: default_graph_depth(),
        }
    }
}

fn default_search_limit() -> usize {
    10
}
fn default_search_threshold() -> f64 {
    0.7
}
fn default_graph_depth() -> usize {
    2
}

/// Geocoding configuration (`config.settings.GeocodingConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeocodingConfig {
    /// Enable automatic geocoding of Location entities.
    #[serde(default)]
    pub enabled: bool,
    /// Geocoding provider to use.
    #[serde(default)]
    pub provider: GeocodingProvider,
    /// API key for geocoding provider.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Cache geocoding results.
    #[serde(default = "default_true")]
    pub cache_results: bool,
    /// Rate limit for geocoding requests.
    #[serde(default = "default_geocoding_rate_limit")]
    pub rate_limit_per_second: f64,
    /// User agent string for Nominatim requests.
    #[serde(default = "default_geocoding_user_agent")]
    pub user_agent: String,
}

impl Default for GeocodingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: GeocodingProvider::Nominatim,
            api_key: None,
            cache_results: true,
            rate_limit_per_second: default_geocoding_rate_limit(),
            user_agent: default_geocoding_user_agent(),
        }
    }
}

fn default_geocoding_rate_limit() -> f64 {
    1.0
}
fn default_geocoding_user_agent() -> String {
    "neo4j-agent-memory".to_owned()
}

/// Entity enrichment configuration (`config.settings.EnrichmentConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnrichmentConfig {
    /// Enable automatic entity enrichment.
    #[serde(default)]
    pub enabled: bool,
    /// Enrichment providers to use (in priority order).
    #[serde(default = "default_enrichment_providers")]
    pub providers: Vec<EnrichmentProvider>,
    /// API key for Diffbot Knowledge Graph.
    #[serde(default)]
    pub diffbot_api_key: Option<String>,
    /// Seconds between Wikimedia API requests.
    #[serde(default = "default_wikimedia_rate_limit")]
    pub wikimedia_rate_limit: f64,
    /// Seconds between Diffbot API requests.
    #[serde(default = "default_diffbot_rate_limit")]
    pub diffbot_rate_limit: f64,
    /// Cache enrichment results.
    #[serde(default = "default_true")]
    pub cache_results: bool,
    /// Hours to cache enrichment results.
    #[serde(default = "default_cache_ttl_hours")]
    pub cache_ttl_hours: usize,
    /// Run enrichment in background (non-blocking).
    #[serde(default = "default_true")]
    pub background_enabled: bool,
    /// Maximum enrichment queue size.
    #[serde(default = "default_enrichment_queue_size")]
    pub queue_max_size: usize,
    /// Maximum retry attempts for failed enrichments.
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
    /// Delay between retry attempts in seconds.
    #[serde(default = "default_retry_delay_seconds")]
    pub retry_delay_seconds: f64,
    /// Entity types to enrich.
    #[serde(default = "default_enrichment_entity_types")]
    pub entity_types: Vec<String>,
    /// Minimum entity confidence to trigger enrichment.
    #[serde(default = "default_enrichment_min_confidence")]
    pub min_confidence: f64,
    /// Preferred language for enrichment data.
    #[serde(default = "default_enrichment_language")]
    pub language: String,
    /// User-Agent for API requests.
    #[serde(default = "default_enrichment_user_agent")]
    pub user_agent: String,
}

impl Default for EnrichmentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            providers: default_enrichment_providers(),
            diffbot_api_key: None,
            wikimedia_rate_limit: default_wikimedia_rate_limit(),
            diffbot_rate_limit: default_diffbot_rate_limit(),
            cache_results: true,
            cache_ttl_hours: default_cache_ttl_hours(),
            background_enabled: true,
            queue_max_size: default_enrichment_queue_size(),
            max_retries: default_max_retries(),
            retry_delay_seconds: default_retry_delay_seconds(),
            entity_types: default_enrichment_entity_types(),
            min_confidence: default_enrichment_min_confidence(),
            language: default_enrichment_language(),
            user_agent: default_enrichment_user_agent(),
        }
    }
}

fn default_enrichment_providers() -> Vec<EnrichmentProvider> {
    vec![EnrichmentProvider::Wikimedia]
}
fn default_wikimedia_rate_limit() -> f64 {
    0.5
}
fn default_diffbot_rate_limit() -> f64 {
    0.2
}
fn default_cache_ttl_hours() -> usize {
    24 * 7 // 1 week
}
fn default_enrichment_queue_size() -> usize {
    1000
}
fn default_max_retries() -> usize {
    3
}
fn default_retry_delay_seconds() -> f64 {
    60.0
}
fn default_enrichment_entity_types() -> Vec<String> {
    vec![
        "PERSON".to_owned(),
        "ORGANIZATION".to_owned(),
        "LOCATION".to_owned(),
        "EVENT".to_owned(),
    ]
}
fn default_enrichment_min_confidence() -> f64 {
    0.7
}
fn default_enrichment_language() -> String {
    "en".to_owned()
}
fn default_enrichment_user_agent() -> String {
    "neo4j-agent-memory/1.0".to_owned()
}

/// Configuration for the hosted NAMS backend (`config.settings.NamsConfig`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamsConfig {
    /// NAMS endpoint base URL.
    #[serde(default = "default_nams_endpoint")]
    pub endpoint: String,
    /// NAMS API key.
    #[serde(default)]
    pub api_key: Option<String>,
    /// NAMS workspace id.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// HTTP request timeout in seconds.
    #[serde(default = "default_nams_timeout")]
    pub timeout: f64,
    /// Max retry attempts.
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
    /// Base exponential backoff between retries.
    #[serde(default = "default_nams_backoff")]
    pub retry_backoff_seconds: f64,
    /// Extra HTTP headers.
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,
    /// Validate credentials on connect.
    #[serde(default = "default_true")]
    pub validate_on_connect: bool,
}

impl Default for NamsConfig {
    fn default() -> Self {
        Self {
            endpoint: default_nams_endpoint(),
            api_key: None,
            workspace_id: None,
            timeout: default_nams_timeout(),
            max_retries: default_max_retries(),
            retry_backoff_seconds: default_nams_backoff(),
            headers: std::collections::HashMap::new(),
            validate_on_connect: true,
        }
    }
}

fn default_nams_endpoint() -> String {
    "https://memory.neo4jlabs.com/v1".to_owned()
}
fn default_nams_timeout() -> f64 {
    30.0
}
fn default_nams_backoff() -> f64 {
    0.5
}

// ---------------------------------------------------------------------------
// Main aggregator
// ---------------------------------------------------------------------------

/// Main configuration class mirroring `neo4j_agent_memory.config.MemorySettings`.
///
/// Aggregates all sub-configs into a single top-level struct. Callers construct
/// this directly or deserialize from JSON/YAML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemorySettings {
    /// Neo4j connection configuration.
    #[serde(default)]
    pub neo4j: Neo4jConfig,
    /// Embedding provider configuration.
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    /// LLM provider configuration (optional — None means no LLM).
    #[serde(default)]
    pub llm: Option<LLMConfig>,
    /// Schema configuration.
    #[serde(default)]
    pub schema_config: SchemaConfig,
    /// Extraction configuration.
    #[serde(default)]
    pub extraction: ExtractionConfig,
    /// Resolution configuration.
    #[serde(default)]
    pub resolution: ResolutionConfig,
    /// Memory behavior configuration.
    #[serde(default)]
    pub memory: MemoryBehaviorConfig,
    /// Search configuration.
    #[serde(default)]
    pub search: SearchConfig,
    /// Geocoding configuration.
    #[serde(default)]
    pub geocoding: GeocodingConfig,
    /// Enrichment configuration.
    #[serde(default)]
    pub enrichment: EnrichmentConfig,
    /// Storage backend.
    #[serde(default)]
    pub backend: Option<Backend>,
    /// NAMS configuration.
    #[serde(default)]
    pub nams: NamsConfig,
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            neo4j: Neo4jConfig::default(),
            embedding: EmbeddingConfig::default(),
            llm: None,
            schema_config: SchemaConfig::default(),
            extraction: ExtractionConfig::default(),
            resolution: ResolutionConfig::default(),
            memory: MemoryBehaviorConfig::default(),
            search: SearchConfig::default(),
            geocoding: GeocodingConfig::default(),
            enrichment: EnrichmentConfig::default(),
            backend: None,
            nams: NamsConfig::default(),
        }
    }
}

impl MemorySettings {
    /// Validate the entire settings tree.
    ///
    /// Mirrors the Pydantic model validators:
    /// - `_validate_llm_consistency`
    /// - `_resolve_backend`
    /// - `ResolutionConfig._check_resolution_bands`
    pub fn validate(&mut self) -> Result<(), MemoryError> {
        // ResolutionConfig bands check
        self.resolution.validate()?;

        // LLM consistency check
        let needs_llm = self.extraction.needs_llm();
        if needs_llm && self.llm.is_none() {
            // Auto-provision a default LLM config if extraction needs it
            // and the user didn't explicitly set llm=None.
            // (In Rust, we can't distinguish "user didn't set" from "user set None"
            // without a special sentinel, so we auto-provision.)
            self.llm = Some(LLMConfig::default());
        }

        // Resolve backend
        if self.backend.is_none() {
            self.backend = if self.nams.api_key.is_some() {
                Some(Backend::Nams)
            } else {
                Some(Backend::Bolt)
            };
        }

        // Apply Vertex AI defaults if applicable
        if self.embedding.provider == EmbeddingProvider::VertexAi {
            self.embedding.apply_vertex_ai_defaults();
        }

        Ok(())
    }

    /// Create settings from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, MemoryError> {
        serde_json::from_str(json)
            .map_err(|e| MemoryError::invalid(format!("invalid settings JSON: {e}")))
    }

    /// Create settings from a dictionary-like serde_json::Value.
    pub fn from_value(value: serde_json::Value) -> Result<Self, MemoryError> {
        serde_json::from_value(value)
            .map_err(|e| MemoryError::invalid(format!("invalid settings value: {e}")))
    }

    /// Return a minimal config suitable for local testing.
    pub fn minimal() -> Self {
        Self::default()
    }

    /// Return a config with all defaults aligned to the oráculo.
    pub fn default_agent() -> Self {
        Self::default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neo4j_config_defaults() {
        let cfg = Neo4jConfig::default();
        assert_eq!(cfg.uri, "bolt://localhost:7687");
        assert_eq!(cfg.username, "neo4j");
        assert_eq!(cfg.password, "");
        assert_eq!(cfg.database, "neo4j");
        assert_eq!(cfg.max_connection_pool_size, 50);
        assert_eq!(cfg.connection_timeout, 30.0);
        assert!(cfg.keep_alive);
    }

    #[test]
    fn embedding_config_defaults() {
        let cfg = EmbeddingConfig::default();
        assert_eq!(cfg.provider, EmbeddingProvider::Openai);
        assert_eq!(cfg.model, "text-embedding-3-small");
        assert_eq!(cfg.dimensions, 1536);
        assert_eq!(cfg.batch_size, 100);
    }

    #[test]
    fn embedding_config_vertex_ai_defaults() {
        let mut cfg = EmbeddingConfig {
            provider: EmbeddingProvider::VertexAi,
            ..Default::default()
        };
        cfg.apply_vertex_ai_defaults();
        assert_eq!(cfg.model, "gemini-embedding-001");
        assert_eq!(cfg.output_dimensionality, Some(768));
        assert_eq!(cfg.dimensions, 768);
    }

    #[test]
    fn llm_config_defaults() {
        let cfg = LLMConfig::default();
        assert_eq!(cfg.provider, LLMProvider::Openai);
        assert_eq!(cfg.model, "gpt-4o-mini");
        assert_eq!(cfg.temperature, 0.0);
        assert_eq!(cfg.max_tokens, 4096);
    }

    #[test]
    fn schema_config_defaults() {
        let cfg = SchemaConfig::default();
        assert_eq!(cfg.model, SchemaModel::Poleo);
        assert!(cfg.enable_subtypes);
        assert!(!cfg.strict_types);
        assert_eq!(cfg.ontology_template, "poleo");
        assert!(cfg.backfill_relation_types);
    }

    #[test]
    fn extraction_config_defaults() {
        let cfg = ExtractionConfig::default();
        assert_eq!(cfg.extractor_type, ExtractorType::Pipeline);
        assert!(cfg.enable_spacy);
        assert!(cfg.enable_gliner);
        assert!(cfg.enable_llm_fallback);
        assert_eq!(cfg.merge_strategy, MergeStrategy::Confidence);
        assert_eq!(cfg.spacy_model, "en_core_web_sm");
        assert_eq!(cfg.gliner_model, "fastino/gliner2.5-base-v1");
        assert!(cfg.needs_llm());
    }

    #[test]
    fn extraction_config_no_llm_needed() {
        let cfg = ExtractionConfig {
            extractor_type: ExtractorType::Spacy,
            ..Default::default()
        };
        assert!(!cfg.needs_llm());
    }

    #[test]
    fn resolution_config_defaults() {
        let cfg = ResolutionConfig::default();
        assert_eq!(cfg.strategy, ResolverStrategy::Composite);
        assert_eq!(cfg.exact_threshold, 1.0);
        assert_eq!(cfg.fuzzy_threshold, 0.85);
        assert_eq!(cfg.semantic_threshold, 0.8);
        assert_eq!(cfg.auto_merge_threshold, 0.90);
        assert_eq!(cfg.review_threshold, 0.85);
        assert_eq!(cfg.scope, ResolutionScope::Global);
    }

    #[test]
    fn resolution_config_validate_passes() {
        let cfg = ResolutionConfig::default();
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn resolution_config_validate_fails_when_review_gt_merge() {
        let cfg = ResolutionConfig {
            review_threshold: 0.95,
            auto_merge_threshold: 0.90,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("review_threshold"));
    }

    #[test]
    fn memory_behavior_config_defaults() {
        let cfg = MemoryBehaviorConfig::default();
        assert_eq!(cfg.default_conversation_limit, 50);
        assert!(cfg.message_embedding_enabled);
        assert!(cfg.fact_deduplication_enabled);
        assert!(!cfg.multi_tenant);
        assert_eq!(cfg.write_mode, WriteMode::Sync);
        assert_eq!(cfg.max_pending, 200);
    }

    #[test]
    fn search_config_defaults() {
        let cfg = SearchConfig::default();
        assert_eq!(cfg.default_limit, 10);
        assert_eq!(cfg.default_threshold, 0.7);
        assert!(cfg.hybrid_search_enabled);
        assert_eq!(cfg.graph_depth, 2);
    }

    #[test]
    fn geocoding_config_defaults() {
        let cfg = GeocodingConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.provider, GeocodingProvider::Nominatim);
        assert!(cfg.cache_results);
    }

    #[test]
    fn enrichment_config_defaults() {
        let cfg = EnrichmentConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.providers, vec![EnrichmentProvider::Wikimedia]);
        assert!(cfg.cache_results);
        assert!(cfg.background_enabled);
        assert_eq!(cfg.queue_max_size, 1000);
        assert_eq!(cfg.max_retries, 3);
    }

    #[test]
    fn nams_config_defaults() {
        let cfg = NamsConfig::default();
        assert_eq!(cfg.endpoint, "https://memory.neo4jlabs.com/v1");
        assert!(cfg.api_key.is_none());
        assert_eq!(cfg.timeout, 30.0);
        assert!(cfg.validate_on_connect);
    }

    #[test]
    fn memory_settings_defaults() {
        let settings = MemorySettings::default();
        assert_eq!(settings.backend, None);
        assert!(settings.llm.is_none());
        assert_eq!(settings.neo4j.uri, "bolt://localhost:7687");
    }

    #[test]
    fn memory_settings_validate_auto_provisions_llm() {
        let mut settings = MemorySettings::default();
        // Default extraction is Pipeline with enable_llm_fallback=true
        settings.validate().unwrap();
        assert!(settings.llm.is_some());
    }

    #[test]
    fn memory_settings_validate_resolves_backend() {
        let mut settings = MemorySettings {
            nams: NamsConfig {
                api_key: Some("nams_test".to_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        settings.validate().unwrap();
        assert_eq!(settings.backend, Some(Backend::Nams));
    }

    #[test]
    fn memory_settings_validate_resolves_bolt_backend() {
        let mut settings = MemorySettings::default();
        settings.validate().unwrap();
        assert_eq!(settings.backend, Some(Backend::Bolt));
    }

    #[test]
    fn enum_parsing() {
        assert_eq!(
            "openai".parse::<EmbeddingProvider>().unwrap(),
            EmbeddingProvider::Openai
        );
        assert_eq!(
            "OPENAI".parse::<EmbeddingProvider>().unwrap(),
            EmbeddingProvider::Openai
        );
        assert!("unknown".parse::<EmbeddingProvider>().is_err());

        assert_eq!(
            "composite".parse::<ResolverStrategy>().unwrap(),
            ResolverStrategy::Composite
        );
        assert_eq!(
            "pipeline".parse::<ExtractorType>().unwrap(),
            ExtractorType::Pipeline
        );
        assert_eq!(
            "confidence".parse::<MergeStrategy>().unwrap(),
            MergeStrategy::Confidence
        );
        assert_eq!("poleo".parse::<SchemaModel>().unwrap(), SchemaModel::Poleo);
    }

    #[test]
    fn enum_display() {
        assert_eq!(EmbeddingProvider::Openai.to_string(), "openai");
        assert_eq!(LLMProvider::Anthropic.to_string(), "anthropic");
        assert_eq!(ExtractorType::Pipeline.to_string(), "pipeline");
        assert_eq!(MergeStrategy::Confidence.to_string(), "confidence");
        assert_eq!(SchemaModel::Poleo.to_string(), "poleo");
        assert_eq!(ResolverStrategy::Composite.to_string(), "composite");
        assert_eq!(GeocodingProvider::Nominatim.to_string(), "nominatim");
        assert_eq!(EnrichmentProvider::Wikimedia.to_string(), "wikimedia");
        assert_eq!(Backend::Bolt.to_string(), "bolt");
        assert_eq!(WriteMode::Buffered.to_string(), "buffered");
        assert_eq!(ValidationMode::Strict.to_string(), "strict");
        assert_eq!(ResolutionScope::User.to_string(), "user");
    }

    #[test]
    fn settings_from_json() {
        let json = r#"{
            "neo4j": { "uri": "bolt://remote:7687", "password": "secret" },
            "embedding": { "provider": "vertex_ai" },
            "backend": "bolt"
        }"#;
        let settings = MemorySettings::from_json(json).unwrap();
        assert_eq!(settings.neo4j.uri, "bolt://remote:7687");
        assert_eq!(settings.embedding.provider, EmbeddingProvider::VertexAi);
        assert_eq!(settings.backend, Some(Backend::Bolt));
    }

    #[test]
    fn settings_serialization_roundtrip() {
        let settings = MemorySettings::default();
        let json = serde_json::to_string(&settings).unwrap();
        let deserialized = MemorySettings::from_json(&json).unwrap();
        assert_eq!(settings, deserialized);
    }
}
