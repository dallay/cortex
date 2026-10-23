// route_selection — pure routing classification for virtual combos.
//
// Single policy for ALL transports: given the requested `model` string and the
// raw `X-Rook-Combo` header values, decide which routing intention the request
// expresses. Pure function — no repository access, no Axum types.
//
// Transports extract strings, call `classify_route`, map `RouteError` to the
// wire format, and forward the `RouteSelection` (or its stashed form) to
// `RouteRequest`, which performs lookup + authorization + execution exactly once.

use rook_core::ComboAlias;
use shared_kernel::{ComboId, ModelId};

/// Routing intention expressed by (model, combo-header) inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteSelection {
    /// Normal model routing (may still hit `ModelAlias` resolution downstream).
    Model,
    /// Virtual combo: exact `rook/*` alias, header must be absent.
    VirtualCombo { alias: String },
    /// Legacy combo: physical model + `X-Rook-Combo` UUID header.
    LegacyCombo { combo_id: ComboId },
}

/// Classification failures. Transport maps these to HTTP 400 without touching
/// any repository. `404 model_not_found` is a RESOLUTION result, never raised here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// `rook/*` model AND combo header present — refuse to guess.
    Ambiguous,
    /// Header present but unusable: empty, malformed UUID, or duplicated.
    InvalidHeader(String),
    /// `rook/` prefix present but alias shape invalid (`rook/`, uppercase, …).
    InvalidAlias(String),
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ambiguous => write!(
                f,
                "model selects a virtual combo and X-Rook-Combo is also present; send only one"
            ),
            Self::InvalidHeader(msg) => write!(f, "invalid X-Rook-Combo header: {msg}"),
            Self::InvalidAlias(msg) => write!(f, "invalid virtual model: {msg}"),
        }
    }
}

impl std::error::Error for RouteError {}

/// Classify routing intention. `header_values` are ALL `X-Rook-Combo` header
/// values in order (via `get_all`); empty vec means absent.
///
/// | model | header | result |
/// |---|---|---|
/// | `rook/coding` | absent | `VirtualCombo` |
/// | `rook/coding` | present | `Ambiguous` |
/// | `gpt-4.1` | absent | `Model` |
/// | `gpt-4.1` | present(valid UUID) | `LegacyCombo` |
/// | `rook/unknown` | absent | `VirtualCombo` (existence is resolution, not classification) |
/// | `rook/` | absent | `InvalidAlias` |
pub fn classify_route(model: &str, header_values: &[String]) -> Result<RouteSelection, RouteError> {
    let is_virtual = model.starts_with("rook/");

    if header_values.len() > 1 {
        return Err(RouteError::InvalidHeader(
            "multiple X-Rook-Combo headers present; send exactly one".to_string(),
        ));
    }

    match header_values.first() {
        None => {
            if is_virtual {
                ComboAlias::validate_alias_format(model).map_err(RouteError::InvalidAlias)?;
                Ok(RouteSelection::VirtualCombo {
                    alias: model.to_string(),
                })
            } else {
                Ok(RouteSelection::Model)
            }
        }
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(RouteError::InvalidHeader(
                    "header is empty; omit it or send a valid combo UUID".to_string(),
                ));
            }
            let combo_id = ComboId::parse_str(trimmed).map_err(|_| {
                RouteError::InvalidHeader(format!("must be a valid UUID, got: {trimmed}"))
            })?;
            if is_virtual {
                return Err(RouteError::Ambiguous);
            }
            Ok(RouteSelection::LegacyCombo { combo_id })
        }
    }
}

/// Extract all values of a header from an Axum `HeaderMap` without depending on
/// Axum here: callers pass `headers.get_all(name).iter().map(...)`.
/// Kept as documentation; the actual extraction is two lines in transport.
pub fn header_values_note() -> &'static str {
    "use headers.get_all(\"x-rook-combo\").iter().filter_map(|v| v.to_str().ok().map(str::to_string)).collect::<Vec<_>>()"
}

pub fn is_virtual_model(model: &ModelId) -> bool {
    model.as_str().starts_with("rook/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(v: &str) -> Vec<String> {
        vec![v.to_string()]
    }

    #[test]
    fn virtual_without_header_selects_combo() {
        assert_eq!(
            classify_route("rook/coding", &[]),
            Ok(RouteSelection::VirtualCombo {
                alias: "rook/coding".to_string()
            })
        );
    }

    #[test]
    fn virtual_with_header_is_ambiguous() {
        let id = ComboId::new().to_string();
        assert_eq!(
            classify_route("rook/coding", &h(&id)),
            Err(RouteError::Ambiguous)
        );
    }

    #[test]
    fn physical_without_header_is_model() {
        assert_eq!(classify_route("gpt-4.1", &[]), Ok(RouteSelection::Model));
    }

    #[test]
    fn physical_with_header_is_legacy() {
        let id = ComboId::new();
        assert_eq!(
            classify_route("gpt-4.1", &h(&id.to_string())),
            Ok(RouteSelection::LegacyCombo { combo_id: id })
        );
    }

    #[test]
    fn unknown_virtual_is_still_virtual_classification() {
        // Existence is resolution (IO), not classification (pure).
        assert_eq!(
            classify_route("rook/unknown", &[]),
            Ok(RouteSelection::VirtualCombo {
                alias: "rook/unknown".to_string()
            })
        );
    }

    #[test]
    fn bare_prefix_is_invalid_alias() {
        assert!(matches!(
            classify_route("rook/", &[]),
            Err(RouteError::InvalidAlias(_))
        ));
    }

    #[test]
    fn uppercase_virtual_is_invalid_alias() {
        assert!(matches!(
            classify_route("rook/Coding", &[]),
            Err(RouteError::InvalidAlias(_))
        ));
    }

    #[test]
    fn empty_header_is_invalid() {
        assert!(matches!(
            classify_route("gpt-4.1", &h("")),
            Err(RouteError::InvalidHeader(_))
        ));
        assert!(matches!(
            classify_route("gpt-4.1", &h("   ")),
            Err(RouteError::InvalidHeader(_))
        ));
    }

    #[test]
    fn malformed_header_is_invalid() {
        assert!(matches!(
            classify_route("gpt-4.1", &h("not-a-uuid")),
            Err(RouteError::InvalidHeader(_))
        ));
    }

    #[test]
    fn duplicate_headers_are_invalid() {
        let id = ComboId::new().to_string();
        assert!(matches!(
            classify_route("gpt-4.1", &[id.clone(), id]),
            Err(RouteError::InvalidHeader(_))
        ));
    }
}
