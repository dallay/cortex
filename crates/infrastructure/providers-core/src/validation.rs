// validation.rs — Response validation utilities
// Note: serde::ser::Error is used via fully qualified syntax for custom error messages

/// Validate that a string is valid JSON that can be deserialized.
///
/// This is used to validate provider responses before attempting
/// full parsing with specific response types.
///
/// # Arguments
/// * `response` — The raw string response from a provider
///
/// # Returns
/// * `Ok(serde_json::Value)` — The parsed JSON value
/// * `Err(serde_json::Error)` — If the response is not valid JSON
///
/// # Examples
/// ```
/// use providers_core::validate_response;
/// let valid = r#"{"key": "value"}"#;
/// assert!(validate_response(valid).is_ok());
///
/// let invalid = "not json";
/// assert!(validate_response(invalid).is_err());
/// ```
pub fn validate_response(response: &str) -> Result<serde_json::Value, serde_json::Error> {
    serde_json::from_str(response)
}

/// Validate that a response is valid JSON and has a specific structure.
///
/// This is a convenience function for validating response types that
/// have a "type" or "error" field at the top level.
///
/// # Arguments
/// * `response` — The raw string response from a provider
/// * `expected_type` — The type field value to check for
///
/// # Returns
/// * `Ok(serde_json::Value)` — If valid JSON and type matches
/// * `Err(serde_json::Error)` — If not valid JSON or type doesn't match
pub fn validate_response_with_type(
    response: &str,
    expected_type: &str,
) -> Result<serde_json::Value, serde_json::Error> {
    let value: serde_json::Value = serde_json::from_str(response)?;

    // Check if the response has the expected type field
    if let Some(obj) = value.as_object() {
        if let Some(typ) = obj.get("type").and_then(|t| t.as_str()) {
            if typ != expected_type {
                return Err(serde::ser::Error::custom(format!(
                    "expected type '{}', got '{}'",
                    expected_type, typ
                )));
            }
        }
    }

    Ok(value)
}

/// Check if a JSON response indicates an error.
///
/// Returns true if the response contains an "error" field at the top level.
pub fn is_error_response(response: &str) -> bool {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(response) {
        value
            .as_object()
            .is_some_and(|obj| obj.contains_key("error"))
    } else {
        false
    }
}

/// Extract error message from a response if present.
///
/// Returns the error message if the response has an "error.message" or
/// "error" field, otherwise returns the original response.
pub fn extract_error_message(response: &str) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(response) {
        if let Some(obj) = value.as_object() {
            // Try "error.message" first (Anthropic format)
            if let Some(error_obj) = obj.get("error").and_then(|e| e.as_object()) {
                if let Some(msg) = error_obj.get("message").and_then(|m| m.as_str()) {
                    return msg.to_string();
                }
            }
            // Try "error" as a string (OpenAI format)
            if let Some(msg) = obj.get("error").and_then(|e| e.as_str()) {
                return msg.to_string();
            }
        }
    }
    response.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_response_valid() {
        let json = r#"{"key": "value", "number": 42}"#;
        let result = validate_response(json);
        assert!(result.is_ok());
        let value = result.unwrap();
        assert_eq!(value["key"], "value");
        assert_eq!(value["number"], 42);
    }

    #[test]
    fn test_validate_response_empty() {
        assert!(validate_response("").is_err());
    }

    #[test]
    fn test_validate_response_invalid() {
        assert!(validate_response("not json").is_err());
        assert!(validate_response("{invalid").is_err());
    }

    #[test]
    fn test_validate_response_array() {
        let json = "[1, 2, 3]";
        let result = validate_response(json);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_response_with_type_matching() {
        let json = r#"{"type": "message", "content": "hello"}"#;
        let result = validate_response_with_type(json, "message");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_response_with_type_mismatch() {
        let json = r#"{"type": "error", "content": "hello"}"#;
        let result = validate_response_with_type(json, "message");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_response_with_type_missing() {
        let json = r#"{"content": "hello"}"#;
        let result = validate_response_with_type(json, "message");
        // No type field means validation passes (type field is optional)
        assert!(result.is_ok());
    }

    #[test]
    fn test_is_error_response_true() {
        let json = r#"{"error": "something went wrong"}"#;
        assert!(is_error_response(json));
    }

    #[test]
    fn test_is_error_response_false() {
        let json = r#"{"content": "hello"}"#;
        assert!(!is_error_response(json));
    }

    #[test]
    fn test_is_error_response_invalid_json() {
        assert!(!is_error_response("not json"));
    }

    #[test]
    fn test_extract_error_message_anthropic_format() {
        let json = r#"{"error": {"type": "rate_limit", "message": "slow down"}}"#;
        assert_eq!(extract_error_message(json), "slow down");
    }

    #[test]
    fn test_extract_error_message_openai_format() {
        let json = r#"{"error": "bad request"}"#;
        assert_eq!(extract_error_message(json), "bad request");
    }

    #[test]
    fn test_extract_error_message_no_error() {
        let json = r#"{"content": "hello"}"#;
        assert_eq!(extract_error_message(json), json);
    }

    #[test]
    fn test_extract_error_message_invalid_json() {
        assert_eq!(extract_error_message("not json"), "not json");
    }
}
