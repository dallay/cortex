// sanitize.rs — String sanitization and truncation utilities
//
// These functions prevent sensitive data leakage in error messages
// and truncate responses at character boundaries (not byte boundaries)
// to handle UTF-8 multi-byte characters correctly.

/// Truncate a string to at most `max` characters, safe across UTF-8 multi-byte boundaries.
///
/// If the string exceeds `max_chars`, it is truncated and an ellipsis indicator
/// is appended. The truncation happens at a character boundary, not a byte boundary,
/// so multi-byte UTF-8 characters are preserved correctly.
///
/// # Arguments
/// * `s` — The string to truncate
/// * `max_chars` — Maximum number of characters to keep (excluding ellipsis)
///
/// # Returns
/// * The truncated string with "… (truncated)" appended if truncation occurred
///
/// # Examples
/// ```
/// use providers_core::char_safe_truncate;
/// assert!(char_safe_truncate("hello world", 5).ends_with("… (truncated)"));
/// assert!(char_safe_truncate("hi", 10).ends_with("hi"));
/// ```
pub fn char_safe_truncate(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}… (truncated)")
    } else {
        truncated
    }
}

/// Sanitize and truncate a body string to prevent sensitive data leakage.
///
/// This function is used to sanitize error response bodies before logging
/// or including them in error messages.
///
/// It attempts to parse the body as JSON and redact any fields with
/// potentially sensitive names (api_key, authorization, token, secret, etc.).
/// Falls back to plain truncation if JSON parsing fails.
///
/// # Arguments
/// * `body` — The raw response body string
///
/// # Returns
/// * A sanitized string safe for logging/error messages
pub fn sanitize_body(body: &str) -> String {
    const MAX_LENGTH: usize = 200;
    const SENSITIVE_KEYS: &[&str] = &[
        "api_key",
        "authorization",
        "token",
        "access_token",
        "secret",
        "headers",
    ];

    // Try to parse as JSON and redact sensitive fields
    if let Ok(mut json) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(obj) = json.as_object_mut() {
            let keys_to_redact: Vec<String> = obj
                .keys()
                .filter(|k| {
                    let lower = k.to_lowercase();
                    SENSITIVE_KEYS.iter().any(|s| lower.contains(s))
                })
                .cloned()
                .collect();
            for key in keys_to_redact {
                obj.insert(key, serde_json::Value::String("(redacted)".to_string()));
            }
        }
        let sanitized = serde_json::to_string(&json).unwrap_or_else(|_| body.to_string());
        char_safe_truncate(&sanitized, MAX_LENGTH)
    } else {
        // Fall back to plain text truncation
        char_safe_truncate(body, MAX_LENGTH)
    }
}

/// Sanitize a URL by removing query parameters that may contain sensitive data.
///
/// Useful for logging URLs without exposing API keys in query strings.
///
/// # Arguments
/// * `url` — The URL to sanitize
///
/// # Returns
/// A sanitized URL with query parameters removed
pub fn sanitize_url(url: &str) -> String {
    if let Some(pos) = url.find('?') {
        format!("{}?[redacted]", &url[..pos])
    } else {
        url.to_string()
    }
}

/// Check if a string contains any sensitive patterns (API keys, tokens, etc.).
///
/// Returns true if the string looks like it contains sensitive data.
pub fn contains_sensitive_data(s: &str) -> bool {
    const SENSITIVE_PATTERNS: &[&str] = &[
        "sk-",       // OpenAI API key
        "sk-ant-",   // Anthropic API key
        "Bearer ",   // Authorization header
        "api_key",   // API key field name
        "\"token\"", // token field name in JSON
        "token=",    // OAuth/token URL param
        "password=", // Password URL param
        "secret=",   // Secret URL param
    ];

    let lower = s.to_lowercase();
    SENSITIVE_PATTERNS
        .iter()
        .any(|pattern| lower.contains(pattern))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char_safe_truncate_short() {
        let s = "hello";
        assert_eq!(char_safe_truncate(s, 10), "hello");
    }

    #[test]
    fn test_char_safe_truncate_exact() {
        let s = "hello";
        assert_eq!(char_safe_truncate(s, 5), "hello");
    }

    #[test]
    fn test_char_safe_truncate_long() {
        let s = "hello world";
        let result = char_safe_truncate(s, 5);
        assert!(result.contains("… (truncated)"));
        assert!(result.starts_with("hello"));
    }

    #[test]
    fn test_char_safe_truncate_multibyte() {
        let s = "こんにちは世界"; // Japanese: 7 characters
        let result = char_safe_truncate(s, 3);
        assert!(result.contains("… (truncated)"));
        // Should not split a Japanese character - result should be 3 chars + ellipsis
        let chars: Vec<char> = result.chars().collect();
        // 3 Japanese chars + "… (truncated)" suffix
        assert!(chars.len() > 3);
    }

    #[test]
    fn test_char_safe_truncate_empty() {
        assert_eq!(char_safe_truncate("", 10), "");
    }

    #[test]
    fn test_char_safe_truncate_zero() {
        let result = char_safe_truncate("hello", 0);
        assert!(result.contains("… (truncated)"));
    }

    #[test]
    fn test_sanitize_body_json() {
        let json = r#"{"api_key": "sk-12345", "content": "hello"}"#;
        let result = sanitize_body(json);
        assert!(result.contains("(redacted)"));
        assert!(result.contains("hello"));
        assert!(!result.contains("sk-12345"));
    }

    #[test]
    fn test_sanitize_body_json_case_insensitive() {
        let json = r#"{"AUTHORIZATION": "Bearer secret", "data": "test"}"#;
        let result = sanitize_body(json);
        assert!(result.contains("(redacted)"));
    }

    #[test]
    fn test_sanitize_body_non_json() {
        let text = "This is a plain text error message that is quite long.";
        let result = sanitize_body(text);
        assert!(result.len() <= 220); // MAX_LENGTH + ellipsis
    }

    #[test]
    fn test_sanitize_body_empty() {
        assert_eq!(sanitize_body(""), "");
    }

    #[test]
    fn test_sanitize_body_nested_json() {
        // Note: sanitize_body only handles top-level keys, not nested ones
        // This is consistent with the original provider implementations
        let json =
            r#"{"error": {"message": "bad request", "token": "secret123"}, "api_key": "sk-12345"}"#;
        let result = sanitize_body(json);
        // api_key at top level should be redacted
        assert!(result.contains("(redacted)"));
        // Nested error.message should be preserved
        assert!(result.contains("bad request"));
        // Nested token should NOT be redacted (only top-level keys are checked)
        assert!(result.contains("secret123"));
    }

    #[test]
    fn test_sanitize_url() {
        assert_eq!(
            sanitize_url("https://api.example.com/v1/chat?api_key=sk-123"),
            "https://api.example.com/v1/chat?[redacted]"
        );
    }

    #[test]
    fn test_sanitize_url_no_params() {
        assert_eq!(
            sanitize_url("https://api.example.com/v1/chat"),
            "https://api.example.com/v1/chat"
        );
    }

    #[test]
    fn test_contains_sensitive_data_true() {
        assert!(contains_sensitive_data("Authorization: Bearer sk-12345"));
        assert!(contains_sensitive_data("api_key=sk-abc"));
        assert!(contains_sensitive_data("{\"token\": \"secret\"}"));
    }

    #[test]
    fn test_contains_sensitive_data_false() {
        assert!(!contains_sensitive_data("hello world"));
        assert!(!contains_sensitive_data("normal request"));
    }
}
