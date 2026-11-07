//! The provider descriptor table — the single declarative source of what a
//! provider is.
//!
//! Provider identity, protocol, auth mechanism, base URL, wire paths,
//! canonical hosts, capability ceiling and model source are declared here and
//! nowhere else. Code that needs provider semantics consults the table; a
//! new provider is a table entry, not a new match arm.

use crate::model_capabilities::CapabilityState;

/// Upper bound of what a provider's protocol can express, declared as data.
/// Absence of evidence stays `CapabilityState::Unknown`; Tier-1 declarations
/// arrive with the unified catalog.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub tools: CapabilityState,
    pub vision: CapabilityState,
    pub reasoning: CapabilityState,
    pub streaming: CapabilityState,
    pub context_tokens: Option<u64>,
}

/// The wire protocol a provider speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// OpenAI Chat Completions (`POST {base}/chat/completions`).
    OpenAiChat,
    /// Anthropic Messages (`POST {base}/v1/messages`).
    AnthropicMessages,
    /// Gemini GenerateContent (`POST {base}/v1beta/models/{model}:generateContent`).
    GeminiGenerateContent,
    /// Ollama native (`POST {base}/api/chat`).
    OllamaNative,
}

/// Concrete adapter implementation used by the composition root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAdapter {
    OpenAi,
    OpenAiCompatible,
    AzureOpenAi,
    Anthropic,
    Ollama,
    OllamaCloud,
    Gemini,
    Groq,
}

/// How a connection's model list is established.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelListing {
    /// Models come from the model catalog, keyed by descriptor id.
    Catalog,
    /// The connection declares its supported models explicitly.
    SupportedModels,
    /// The connection exposes a single deployment as its default model.
    Deployment,
}

/// How a connection's base URL is validated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointValidation {
    /// No endpoint of the provider's own; only the cloud-metadata guard applies.
    None,
    /// Generic OpenAI wire endpoint: user-supplied base URL, endpoint policy,
    /// and the protocol's single API format.
    OpenAiEndpointPolicy,
    /// Azure OpenAI resource endpoint (HTTPS, `*.openai.azure.com` or
    /// `*.services.ai.azure.com`, no query, no fragment).
    AzureHost,
}

/// OAuth 2.0 wire configuration, declared as data. `{tenant}` in an endpoint
/// template is substituted with the connection's tenant segment.
#[derive(Debug, Clone, Copy)]
pub struct OAuthConfig {
    pub scopes: &'static str,
    pub authorization_endpoint: &'static str,
    pub token_endpoint: &'static str,
    pub device_code_endpoint: Option<&'static str>,
    pub userinfo_endpoint: &'static str,
    /// Extra query parameters appended to the authorization URL.
    pub extra_authorize_params: &'static [(&'static str, &'static str)],
}

impl OAuthConfig {
    /// Substitute the `{tenant}` placeholder in an endpoint template.
    pub fn resolve_endpoint(&self, template: &str, tenant: &str) -> String {
        template.replace("{tenant}", tenant)
    }
}

/// The authentication mechanism a provider expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    /// Static API key supplied by the user.
    ApiKey,
    /// OAuth 2.0 flow with refresh tokens.
    OAuth,
    /// A bearer token imported from elsewhere (no refresh flow).
    ImportedToken,
}

/// Provider-relative request paths, carried as data.
///
/// `{model}` in `chat` is a placeholder substituted by the protocol adapter.
#[derive(Debug, Clone, Copy)]
pub struct WirePaths {
    pub chat: &'static str,
    pub models: &'static str,
    pub health: &'static str,
}

/// Where a descriptor's model list comes from.
#[derive(Debug, Clone, Copy)]
pub enum ModelSource {
    /// Model ids declared inline with the descriptor.
    Static(&'static [&'static str]),
    /// The bundled `models-catalog` JSON, keyed by descriptor id.
    Catalog,
    /// Model discovery against the provider at runtime.
    Dynamic,
}

/// A declared provider.
#[derive(Debug, Clone, Copy)]
pub struct ProviderDescriptor {
    /// Human-readable label used by management clients.
    pub label: &'static str,
    /// Canonical persisted and serialized identity (kebab-case).
    pub id: &'static str,
    /// Legacy spellings accepted on read so pre-migration rows never fail.
    pub aliases: &'static [&'static str],
    pub protocol: Protocol,
    pub adapter: ProviderAdapter,
    pub auth: AuthKind,
    /// Explicit authentication kinds accepted by connections to this provider.
    pub accepted_auth_kinds: &'static [AuthKind],
    /// Whether managed connections accept imported OAuth credentials.
    pub accepts_oauth_credentials: bool,
    /// Base URL used when the user does not override it.
    /// Empty means the provider has no truthful default: the user must
    /// supply one (`requires_base_url` is true).
    pub default_base_url: &'static str,
    /// True when a connection must supply its own base URL.
    pub requires_base_url: bool,
    /// How the provider's base URL is validated.
    pub endpoint_validation: EndpointValidation,
    /// How a connection's model list is established.
    pub model_listing: ModelListing,
    /// OAuth wire configuration; `None` for non-OAuth providers.
    pub oauth: Option<OAuthConfig>,
    pub paths: WirePaths,
    /// Base URLs the provider officially documents; used by the canonical
    /// URL quota gate.
    pub canonical_base_urls: &'static [&'static str],
    /// Upper bound of capabilities the protocol can express.
    pub capability_ceiling: Capabilities,
    pub models: ModelSource,
}

impl ProviderDescriptor {
    /// Whether `value` names this descriptor canonically or by alias.
    pub fn matches(&self, value: &str) -> bool {
        self.id.eq_ignore_ascii_case(value)
            || self.aliases.iter().any(|a| a.eq_ignore_ascii_case(value))
    }

    /// The identity serialized to clients and persisted to disk.
    pub fn as_str(&self) -> &'static str {
        self.id
    }
}

/// Every declared provider. The fixed-size enumerations that used to live in
/// `all_provider_kinds`, `format_for` and the dashboard union were fan-out
/// copies of this table; iterate this instead.
pub const PROVIDER_DESCRIPTORS: &[ProviderDescriptor] = &[
    openai(),
    openai_compatible(),
    anthropic(),
    ollama(),
    ollama_cloud(),
    gemini(),
    azure_openai(),
    groq(),
    xai(),
    mistral(),
    deepseek(),
    together(),
    fireworks(),
    cerebras(),
    openrouter(),
    nan(),
];

const fn no_ceiling() -> Capabilities {
    Capabilities {
        tools: CapabilityState::Unknown,
        vision: CapabilityState::Unknown,
        reasoning: CapabilityState::Unknown,
        streaming: CapabilityState::Unknown,
        context_tokens: None,
    }
}

const fn openai_paths() -> WirePaths {
    WirePaths {
        chat: "/chat/completions",
        models: "/models",
        health: "/models",
    }
}

const fn ollama_paths() -> WirePaths {
    WirePaths {
        chat: "/api/chat",
        models: "/api/tags",
        health: "/api/tags",
    }
}

const fn openai() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: true,
        id: "openai",
        label: "OpenAI",
        aliases: &[],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::OpenAi,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "https://api.openai.com/v1",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: openai_paths(),
        canonical_base_urls: &["https://api.openai.com/v1", "https://api.openai.com"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Catalog,
    }
}

const fn openai_compatible() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: false,
        id: "openai-compatible",
        label: "OpenAI Compatible",
        aliases: &[],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::OpenAiCompatible,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "",
        requires_base_url: true,
        endpoint_validation: EndpointValidation::OpenAiEndpointPolicy,
        model_listing: ModelListing::SupportedModels,
        oauth: None,
        paths: openai_paths(),
        canonical_base_urls: &[],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Dynamic,
    }
}

const fn anthropic() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: true,
        id: "anthropic",
        label: "Anthropic",
        aliases: &[],
        protocol: Protocol::AnthropicMessages,
        adapter: ProviderAdapter::Anthropic,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "https://api.anthropic.com",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: WirePaths {
            chat: "/v1/messages",
            models: "/v1/models",
            health: "/v1/models",
        },
        canonical_base_urls: &["https://api.anthropic.com"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Catalog,
    }
}

const fn ollama() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: true,
        id: "ollama",
        label: "Ollama",
        aliases: &[],
        protocol: Protocol::OllamaNative,
        adapter: ProviderAdapter::Ollama,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "http://localhost:11434",
        requires_base_url: true,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: ollama_paths(),
        // Ollama carries no quota capture; no URL is canonical.
        canonical_base_urls: &[],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Catalog,
    }
}

const fn ollama_cloud() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: true,
        id: "ollama-cloud",
        label: "Ollama Cloud",
        // Pre-migration rows and serde output spelled it `ollamacloud`.
        aliases: &["ollamacloud"],
        protocol: Protocol::OllamaNative,
        adapter: ProviderAdapter::OllamaCloud,
        auth: AuthKind::ImportedToken,
        accepted_auth_kinds: &[AuthKind::ImportedToken],
        default_base_url: "https://ollama.com",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: ollama_paths(),
        canonical_base_urls: &["https://ollama.com"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Catalog,
    }
}

const fn gemini() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: true,
        id: "gemini",
        label: "Gemini",
        aliases: &[],
        protocol: Protocol::GeminiGenerateContent,
        adapter: ProviderAdapter::Gemini,
        auth: AuthKind::OAuth,
        accepted_auth_kinds: &[AuthKind::ApiKey, AuthKind::OAuth],
        default_base_url: "https://generativelanguage.googleapis.com",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: Some(OAuthConfig {
            scopes: "https://www.googleapis.com/auth/generative-language.retriever openid email",
            authorization_endpoint: "https://accounts.google.com/o/oauth2/v2/auth",
            token_endpoint: "https://oauth2.googleapis.com/token",
            device_code_endpoint: None,
            userinfo_endpoint: "https://openidconnect.googleapis.com/v1/userinfo",
            extra_authorize_params: &[("access_type", "offline"), ("prompt", "consent")],
        }),
        paths: WirePaths {
            chat: "/v1beta/models/{model}:generateContent",
            models: "/v1beta/models",
            health: "/v1beta/models",
        },
        // Gemini carries no quota capture; no URL is canonical.
        canonical_base_urls: &[],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Catalog,
    }
}

const fn azure_openai() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: true,
        id: "azure-openai",
        label: "Azure OpenAI",
        // Pre-migration rows and serde output spelled it `azureopenai`.
        aliases: &["azureopenai"],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::AzureOpenAi,
        auth: AuthKind::OAuth,
        accepted_auth_kinds: &[AuthKind::OAuth],
        default_base_url: "",
        requires_base_url: true,
        endpoint_validation: EndpointValidation::AzureHost,
        model_listing: ModelListing::Deployment,
        oauth: Some(OAuthConfig {
            scopes:
                "https://cognitiveservices.azure.com/.default offline_access openid profile email",
            authorization_endpoint:
                "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize",
            token_endpoint: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token",
            device_code_endpoint: Some(
                "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/devicecode",
            ),
            userinfo_endpoint: "https://graph.microsoft.com/oidc/userinfo",
            extra_authorize_params: &[],
        }),
        paths: openai_paths(),
        canonical_base_urls: &[],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Dynamic,
    }
}

const fn groq() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: true,
        id: "groq",
        label: "Groq",
        aliases: &[],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::Groq,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "https://api.groq.com/openai/v1",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: openai_paths(),
        canonical_base_urls: &["https://api.groq.com/openai/v1"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Catalog,
    }
}

const fn xai() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: false,
        id: "xai",
        label: "xAI",
        aliases: &[],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::OpenAi,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "https://api.x.ai/v1",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: openai_paths(),
        canonical_base_urls: &["https://api.x.ai/v1"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Static(&["grok-2", "grok-2-mini"]),
    }
}

const fn mistral() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: false,
        id: "mistral",
        label: "Mistral",
        aliases: &[],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::OpenAi,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "https://api.mistral.ai/v1",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: openai_paths(),
        canonical_base_urls: &["https://api.mistral.ai/v1"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Static(&[
            "mistral-large-latest",
            "mistral-small-latest",
            "codestral-latest",
        ]),
    }
}

const fn deepseek() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: false,
        id: "deepseek",
        label: "DeepSeek",
        aliases: &[],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::OpenAi,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "https://api.deepseek.com/v1",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: openai_paths(),
        canonical_base_urls: &["https://api.deepseek.com/v1"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Static(&["deepseek-chat", "deepseek-reasoner"]),
    }
}

const fn together() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: false,
        id: "together",
        label: "Together AI",
        aliases: &[],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::OpenAi,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "https://api.together.xyz/v1",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: openai_paths(),
        canonical_base_urls: &["https://api.together.xyz/v1"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Static(&["meta-llama/Llama-3.3-70B-Instruct-Turbo"]),
    }
}

const fn fireworks() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: false,
        id: "fireworks",
        label: "Fireworks AI",
        aliases: &[],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::OpenAi,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "https://api.fireworks.ai/inference/v1",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: openai_paths(),
        canonical_base_urls: &["https://api.fireworks.ai/inference/v1"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Static(&["accounts/fireworks/models/llama-v3p3-70b-instruct"]),
    }
}

const fn cerebras() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: false,
        id: "cerebras",
        label: "Cerebras",
        aliases: &[],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::OpenAi,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "https://api.cerebras.ai/v1",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: openai_paths(),
        canonical_base_urls: &["https://api.cerebras.ai/v1"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Static(&["llama3.1-8b", "llama-3.3-70b"]),
    }
}

const fn openrouter() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: false,
        id: "openrouter",
        label: "OpenRouter",
        aliases: &[],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::OpenAi,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "https://openrouter.ai/api/v1",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: openai_paths(),
        canonical_base_urls: &["https://openrouter.ai/api/v1"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Static(&["openai/gpt-4o", "anthropic/claude-sonnet-4"]),
    }
}

const fn nan() -> ProviderDescriptor {
    ProviderDescriptor {
        accepts_oauth_credentials: false,
        id: "nan",
        label: "NAN",
        aliases: &[],
        protocol: Protocol::OpenAiChat,
        adapter: ProviderAdapter::OpenAi,
        auth: AuthKind::ApiKey,
        accepted_auth_kinds: &[AuthKind::ApiKey],
        default_base_url: "https://api.nan.builders/v1",
        requires_base_url: false,
        endpoint_validation: EndpointValidation::None,
        model_listing: ModelListing::Catalog,
        oauth: None,
        paths: openai_paths(),
        canonical_base_urls: &["https://api.nan.builders/v1"],
        capability_ceiling: no_ceiling(),
        models: ModelSource::Static(&[
            "deepseek-v4-flash",
            "glm5.3-flash",
            "qwen3.8-flash",
            "gemma4",
        ]),
    }
}

/// Resolve a canonical or legacy provider id to its descriptor.
pub fn descriptor_for(value: &str) -> Option<&'static ProviderDescriptor> {
    PROVIDER_DESCRIPTORS.iter().find(|d| d.matches(value))
}

/// Resolve by exact canonical id.
pub fn descriptor_by_id(id: &str) -> Option<&'static ProviderDescriptor> {
    PROVIDER_DESCRIPTORS.iter().find(|d| d.id == id)
}

/// The canonical ids in table order — the single provider set every
/// enumeration site (quota view, `/v1/models`, dashboard catalog) must
/// derive from.
pub fn provider_ids() -> impl Iterator<Item = &'static str> {
    PROVIDER_DESCRIPTORS.iter().map(|d| d.id)
}
