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
