//! Version policy for the product schema, independent of embedding identity.

use muninn_core::{MemoryError, MemoryResult};
use serde_json::Value;

pub const SCHEMA_VERSION: i64 = 1;

pub fn validate_marker(marker: &Value) -> MemoryResult<()> {
    let version = marker.get("version").and_then(Value::as_i64);
    if !matches!(version, Some(0..=SCHEMA_VERSION))
        || marker.get("state").and_then(Value::as_str) != Some("ready")
    {
        return Err(MemoryError::storage(
            "incompatible Muninn schema version or migration state; restore a compatible backup or use a compatible binary; no automatic repair was performed",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_incomplete_and_malformed_markers() {
        for marker in [
            json!({"version": 1, "state": "migrating"}),
            json!({"version": -1, "state": "ready"}),
            json!({"version": "1", "state": "ready"}),
            json!({"version": 1}),
            json!({"state": "ready"}),
        ] {
            assert!(validate_marker(&marker).is_err(), "{marker}");
        }
    }

    #[test]
    fn accepts_initial_and_current_versions() {
        for version in [0, 1] {
            assert!(validate_marker(&json!({"version": version, "state": "ready"})).is_ok());
        }
    }

    #[test]
    fn rejects_future_versions_before_migration() {
        assert!(validate_marker(&json!({"version": 2, "state": "ready"})).is_err());
    }
}
