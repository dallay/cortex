// Static, in-memory implementation of `ModelCatalogPort`.
//
// The catalog is the source of truth for "what models an API key can be
// restricted to" in the dashboard. Today the catalog is hardcoded per
// `ProviderKind`. When the backend gains the ability to introspect real
// provider capabilities (e.g. fetch `/v1/models` from each active provider
// kind and union the results), this is the only file that should need to
// change.

use async_trait::async_trait;
use rook_core::{ModelCatalogEntry, ModelCatalogPort, ProviderKind};

/// A static catalog mapping provider kinds to known model ids.
///
/// This is intentionally hardcoded. It is **not** a real-time view of
/// provider capabilities — it is the set of models that the dashboard
/// exposes in the API key restriction UI. Add new models here when the
/// proxy learns to serve them.
pub struct StaticModelCatalog;

impl StaticModelCatalog {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StaticModelCatalog {
    fn default() -> Self {
        Self::new()
    }
}

fn catalog() -> Vec<ModelCatalogEntry> {
    let openai = ["gpt-4o", "gpt-4-turbo", "gpt-4", "o1-preview", "o1-mini"];
    let anthropic = [
        "claude-3-5-sonnet-latest",
        "claude-3-opus-20240229",
        "claude-3-haiku-20240307",
    ];
    let ollama = ["llama3.2", "mistral", "qwen2.5"];
    let gemini = ["gemini-1.5-pro", "gemini-1.5-flash"];
    let groq = ["llama-3.1-70b-versatile", "mixtral-8x7b-32768"];

    let mut out = Vec::new();
    for m in openai {
        out.push(ModelCatalogEntry {
            model_id: m.to_string(),
            provider_kind: ProviderKind::OpenAI,
        });
    }
    for m in anthropic {
        out.push(ModelCatalogEntry {
            model_id: m.to_string(),
            provider_kind: ProviderKind::Anthropic,
        });
    }
    for m in ollama {
        out.push(ModelCatalogEntry {
            model_id: m.to_string(),
            provider_kind: ProviderKind::Ollama,
        });
    }
    for m in gemini {
        out.push(ModelCatalogEntry {
            model_id: m.to_string(),
            provider_kind: ProviderKind::Gemini,
        });
    }
    for m in groq {
        out.push(ModelCatalogEntry {
            model_id: m.to_string(),
            provider_kind: ProviderKind::Groq,
        });
    }
    out
}

#[async_trait]
impl ModelCatalogPort for StaticModelCatalog {
    async fn list(&self) -> Vec<ModelCatalogEntry> {
        catalog()
    }
}
