// format_registry.rs — maps provider kind strings to `ApiFormat` variants.
//
// This is a skeleton for Phase 1. Phase 2 will extend it with format-specific
// serializers/deserializers so the routing layer can pick the right wire format
// without hard-coding provider names in route handlers.

/// The set of supported API wire formats.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiFormat {
    OpenAI,
    Anthropic,
}

/// Registry that resolves a provider kind string to an `ApiFormat`.
///
/// Constructed once at startup and shared via `Arc`.
#[derive(Debug, Default, Clone)]
pub struct FormatRegistry;

impl FormatRegistry {
    /// Create a new (empty / default) registry.
    pub fn new() -> Self {
        Self
    }

    /// Look up the `ApiFormat` for the given provider kind.
    ///
    /// Returns `None` for unknown kinds so callers can decide the fallback policy.
    pub fn format_for(&self, kind: &str) -> Option<ApiFormat> {
        match kind {
            "openai" => Some(ApiFormat::OpenAI),
            "anthropic" => Some(ApiFormat::Anthropic),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_for_openai_returns_openai_variant() {
        let registry = FormatRegistry::new();
        assert_eq!(registry.format_for("openai"), Some(ApiFormat::OpenAI));
    }

    #[test]
    fn format_for_anthropic_returns_anthropic_variant() {
        let registry = FormatRegistry::new();
        assert_eq!(registry.format_for("anthropic"), Some(ApiFormat::Anthropic));
    }

    #[test]
    fn format_for_unknown_returns_none() {
        let registry = FormatRegistry::new();
        assert_eq!(registry.format_for("unknown"), None);
        assert_eq!(registry.format_for(""), None);
        assert_eq!(registry.format_for("gemini"), None);
    }
}
