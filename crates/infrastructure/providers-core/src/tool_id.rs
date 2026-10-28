//! Presence-aware tool IDs at provider wire boundaries.

use serde::de::{IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::fmt;

/// Distinguishes omission, null, strings, and invalid JSON types without retaining
/// arbitrary invalid payloads. The empty string is an invalid canonical sentinel,
/// never a generated or coerced ID.
#[derive(Debug, Default)]
pub enum ToolId {
    #[default]
    Missing,
    Null,
    String(String),
    Invalid,
}

impl ToolId {
    pub fn is_invalid_type(&self) -> bool {
        matches!(self, Self::Invalid)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(id) => Some(id),
            _ => None,
        }
    }

    /// Preserve omissions for generation, and explicit invalid values for rejection.
    pub fn canonical(&self) -> Option<String> {
        match self {
            Self::Missing => None,
            Self::String(id) => Some(id.clone()),
            Self::Null | Self::Invalid => Some(String::new()),
        }
    }

    /// OpenAI delta nulls are neutral continuation fields, not identity changes.
    pub fn delta(&self) -> Option<String> {
        match self {
            Self::Null => None,
            _ => self.canonical(),
        }
    }
}

impl<'de> Deserialize<'de> for ToolId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IdVisitor;
        impl<'de> Visitor<'de> for IdVisitor {
            type Value = ToolId;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a tool-call ID")
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<ToolId, E> {
                Ok(ToolId::String(value.into()))
            }

            fn visit_string<E: serde::de::Error>(self, value: String) -> Result<ToolId, E> {
                Ok(ToolId::String(value))
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<ToolId, E> {
                Ok(ToolId::Null)
            }

            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<ToolId, E> {
                Ok(ToolId::Invalid)
            }

            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<ToolId, E> {
                Ok(ToolId::Invalid)
            }

            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<ToolId, E> {
                Ok(ToolId::Invalid)
            }

            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<ToolId, E> {
                Ok(ToolId::Invalid)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<ToolId, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(ToolId::Invalid)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ToolId, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(ToolId::Invalid)
            }
        }
        deserializer.deserialize_any(IdVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Wrapper {
        #[serde(default)]
        id: ToolId,
    }

    #[test]
    fn test_tool_id_variants_methods() {
        let missing = ToolId::Missing;
        assert!(!missing.is_invalid_type());
        assert_eq!(missing.as_str(), None);
        assert_eq!(missing.canonical(), None);
        assert_eq!(missing.delta(), None);

        let null = ToolId::Null;
        assert!(!null.is_invalid_type());
        assert_eq!(null.as_str(), None);
        assert_eq!(null.canonical(), Some("".to_string()));
        assert_eq!(null.delta(), None);

        let valid_str = ToolId::String("call_123".to_string());
        assert!(!valid_str.is_invalid_type());
        assert_eq!(valid_str.as_str(), Some("call_123"));
        assert_eq!(valid_str.canonical(), Some("call_123".to_string()));
        assert_eq!(valid_str.delta(), Some("call_123".to_string()));

        let invalid = ToolId::Invalid;
        assert!(invalid.is_invalid_type());
        assert_eq!(invalid.as_str(), None);
        assert_eq!(invalid.canonical(), Some("".to_string()));
        assert_eq!(invalid.delta(), Some("".to_string()));
    }

    #[test]
    fn test_tool_id_deserialization() {
        let w: Wrapper = serde_json::from_str("{}").unwrap();
        assert!(matches!(w.id, ToolId::Missing));

        let w: Wrapper = serde_json::from_str(r#"{"id": null}"#).unwrap();
        assert!(matches!(w.id, ToolId::Null));

        let w: Wrapper = serde_json::from_str(r#"{"id": "call_abc"}"#).unwrap();
        assert!(matches!(w.id, ToolId::String(ref s) if s == "call_abc"));

        for json in [
            r#"{"id": true}"#,
            r#"{"id": 123}"#,
            r#"{"id": -456}"#,
            r#"{"id": 3.14}"#,
            r#"{"id": [1, 2]}"#,
            r#"{"id": {"a": 1}}"#,
        ] {
            let w: Wrapper = serde_json::from_str(json).unwrap();
            assert!(matches!(w.id, ToolId::Invalid), "failed for {json}");
        }
    }
}
