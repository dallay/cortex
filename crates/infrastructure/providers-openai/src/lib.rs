// providers-openai — OpenAI API provider adapter

pub mod endpoint_resolver;
pub mod provider;

// Re-exports
pub use provider::{parse_groq_rate_limit_headers, OpenAIProvider, OpenAIProviderConfig};
