//! Built-in model aliases — seeded at startup when table is empty

/// Built-in aliases: (alias, canonical, provider_id)
/// All aliases are provider-scoped (provider_id is always Some)
pub const DEFAULT_ALIASES: &[(&str, &str, Option<&str>)] = &[
    // OpenAI
    ("gpt-4o-latest", "gpt-4o-2024-05-13", Some("openai")),
    ("gpt-4o", "gpt-4o-2024-05-13", Some("openai")),
    ("gpt-4-turbo", "gpt-4-turbo-2024-04-09", Some("openai")),
    ("gpt-4", "gpt-4-0613", Some("openai")),
    ("gpt-3.5-turbo", "gpt-3.5-turbo-0125", Some("openai")),
    ("o1", "o1-2024-12-17", Some("openai")),
    ("o1-mini", "o1-mini-2024-09-12", Some("openai")),
    ("o3-mini", "o3-mini-2025-01-31", Some("openai")),
    // Anthropic
    ("claude-opus", "claude-3-opus-20240229", Some("anthropic")),
    (
        "claude-sonnet",
        "claude-3-5-sonnet-20241022",
        Some("anthropic"),
    ),
    (
        "claude-haiku",
        "claude-3-5-haiku-20241022",
        Some("anthropic"),
    ),
    ("claude-3-opus", "claude-3-opus-20240229", Some("anthropic")),
    (
        "claude-3-sonnet",
        "claude-3-sonnet-20240229",
        Some("anthropic"),
    ),
    (
        "claude-3-haiku",
        "claude-3-haiku-20240307",
        Some("anthropic"),
    ),
    // Google Gemini
    ("gemini-pro", "gemini-1.5-pro-latest", Some("gemini")),
    ("gemini-flash", "gemini-1.5-flash-latest", Some("gemini")),
    ("gemini-2.0-flash", "gemini-2.0-flash-exp", Some("gemini")),
    ("gemini-1.5-pro", "gemini-1.5-pro-latest", Some("gemini")),
    (
        "gemini-1.5-flash",
        "gemini-1.5-flash-latest",
        Some("gemini"),
    ),
    ("gemini-exp", "gemini-exp-1206", Some("gemini")),
    // Mistral
    ("mistral-large", "mistral-large-2411", Some("mistral")),
    ("mistral-small", "mistral-small-2501", Some("mistral")),
    ("ministral-8b", "ministral-8b-2410", Some("mistral")),
    // Groq
    (
        "groq-llama-3.1-70b",
        "llama-3.1-70b-versatile",
        Some("groq"),
    ),
    ("groq-llama-3.1-8b", "llama-3.1-8b-instant", Some("groq")),
    (
        "groq-llama-3.3-70b",
        "llama-3.3-70b-versatile",
        Some("groq"),
    ),
];
