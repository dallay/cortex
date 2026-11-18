//! Opt-in entity-resolution policy and the upstream deduplication result shape.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{MemoryError, MemoryResult};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResolutionConfig {
    pub auto_merge_threshold: f64,
    pub review_threshold: f64,
    pub exact_threshold: f64,
    pub fuzzy_threshold: f64,
    pub semantic_threshold: f64,
    pub candidate_limit: i64,
    pub match_same_type_only: bool,
    /// Pinned `DeduplicationConfig.use_fuzzy_matching`. The authoritative
    /// oracle deployment lacks the optional rapidfuzz extra, so its ImportError
    /// path disables fuzzy scoring entirely.
    pub use_fuzzy_matching: bool,
    pub use_embedding_blocking: bool,
    pub resolve_on_ingest: bool,
    pub context_window_chars: usize,
}

impl Default for ResolutionConfig {
    fn default() -> Self {
        Self {
            auto_merge_threshold: 0.90,
            review_threshold: 0.85,
            exact_threshold: 1.0,
            fuzzy_threshold: 0.85,
            semantic_threshold: 0.8,
            candidate_limit: 12,
            match_same_type_only: true,
            use_fuzzy_matching: true,
            use_embedding_blocking: true,
            resolve_on_ingest: true,
            context_window_chars: 90,
        }
    }
}

impl ResolutionConfig {
    /// Validate before calling either a provider or persistence port.
    pub fn validate(&self) -> MemoryResult<()> {
        for (name, value) in [
            ("auto_merge_threshold", self.auto_merge_threshold),
            ("review_threshold", self.review_threshold),
            ("exact_threshold", self.exact_threshold),
            ("fuzzy_threshold", self.fuzzy_threshold),
            ("semantic_threshold", self.semantic_threshold),
        ] {
            if !(0.0..=1.0).contains(&value) {
                return Err(MemoryError::invalid(format!(
                    "{name} must be between 0 and 1"
                )));
            }
        }
        if self.review_threshold > self.auto_merge_threshold {
            return Err(MemoryError::invalid(format!(
                "resolution.review_threshold must be <= resolution.auto_merge_threshold (got {} > {})",
                self.review_threshold, self.auto_merge_threshold
            )));
        }
        if self.candidate_limit < 1 {
            return Err(MemoryError::invalid("candidate_limit must be at least 1"));
        }
        Ok(())
    }

    pub fn action(&self, score: f64) -> DeduplicationAction {
        if score >= self.auto_merge_threshold {
            DeduplicationAction::Merged
        } else if score >= self.review_threshold {
            DeduplicationAction::Flagged
        } else {
            DeduplicationAction::None
        }
    }
}

/// The fuzzy scorer is fixed, rather than accepting unsupported scorer names.
pub const FUZZY_SCORER: &str = "token_sort_ratio";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeduplicationAction {
    #[default]
    None,
    Merged,
    Flagged,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct DeduplicationResult {
    pub is_duplicate: bool,
    pub action: DeduplicationAction,
    pub matched_entity_id: Option<Uuid>,
    pub matched_entity_name: Option<String>,
    pub similarity_score: f64,
    pub match_type: Option<String>,
}

/// Exact surface evidence shared by candidate ordering and scored resolution.
pub fn matches_surface(entity: &crate::model::Entity, name: &str) -> bool {
    std::iter::once(entity.name.as_str())
        .chain(entity.canonical_name.as_deref())
        .chain(entity.aliases.iter().map(String::as_str))
        .any(|surface| name.trim().eq_ignore_ascii_case(surface.trim()))
}

/// Bounded candidate generation; no conversation traversal or owner reinterpretation.
pub struct ResolutionQuery<'a> {
    pub name: &'a str,
    pub entity_type: Option<&'a str>,
    pub embedding: Option<&'a [f64]>,
    pub limit: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Entity, Metadata};
    use chrono::{DateTime, Utc};

    fn entity(name: &str) -> Entity {
        Entity {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            canonical_name: None,
            entity_type: "PERSON".to_owned(),
            subtype: None,
            description: None,
            confidence: 1.0,
            aliases: Vec::new(),
            embedding: None,
            created_at: DateTime::<Utc>::from_timestamp(1_700_000_000, 0)
                .expect("timestamp is representable"),
            updated_at: None,
            metadata: Metadata::new(),
        }
    }

    #[test]
    fn default_config_uses_the_documented_thresholds() {
        let config = ResolutionConfig::default();
        assert!((config.auto_merge_threshold - 0.90).abs() < f64::EPSILON);
        assert!((config.review_threshold - 0.85).abs() < f64::EPSILON);
        assert!((config.exact_threshold - 1.0).abs() < f64::EPSILON);
        assert!((config.fuzzy_threshold - 0.85).abs() < f64::EPSILON);
        assert!((config.semantic_threshold - 0.8).abs() < f64::EPSILON);
        assert_eq!(config.candidate_limit, 12);
        assert!(config.match_same_type_only);
        assert!(config.use_fuzzy_matching);
        assert!(config.use_embedding_blocking);
        assert!(config.resolve_on_ingest);
        assert_eq!(config.context_window_chars, 90);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validation_rejects_thresholds_outside_the_unit_interval() {
        for (name, mutate) in [
            (
                "auto_merge_threshold",
                (|config: &mut ResolutionConfig| config.auto_merge_threshold = 1.5)
                    as fn(&mut ResolutionConfig),
            ),
            ("review_threshold", |config: &mut ResolutionConfig| {
                config.review_threshold = -0.1
            }),
            ("exact_threshold", |config: &mut ResolutionConfig| {
                config.exact_threshold = 2.0
            }),
            ("fuzzy_threshold", |config: &mut ResolutionConfig| {
                config.fuzzy_threshold = -0.5
            }),
            ("semantic_threshold", |config: &mut ResolutionConfig| {
                config.semantic_threshold = 1.1
            }),
        ] {
            let mut config = ResolutionConfig::default();
            mutate(&mut config);
            let error = config.validate().expect_err("out-of-range threshold");
            assert_eq!(
                error.to_string(),
                format!("invalid input: {name} must be between 0 and 1")
            );
        }
    }

    #[test]
    fn validation_rejects_review_threshold_above_auto_merge() {
        let config = ResolutionConfig {
            review_threshold: 0.95,
            auto_merge_threshold: 0.90,
            ..ResolutionConfig::default()
        };
        let error = config.validate().expect_err("inverted thresholds");
        assert_eq!(
            error.to_string(),
            "invalid input: resolution.review_threshold must be <= resolution.auto_merge_threshold (got 0.95 > 0.9)"
        );
    }

    #[test]
    fn validation_rejects_a_candidate_limit_below_one() {
        let config = ResolutionConfig {
            candidate_limit: 0,
            ..ResolutionConfig::default()
        };
        let error = config.validate().expect_err("zero candidate limit");
        assert_eq!(
            error.to_string(),
            "invalid input: candidate_limit must be at least 1"
        );
    }

    #[test]
    fn action_merges_at_or_above_the_auto_merge_threshold() {
        let config = ResolutionConfig::default();
        assert_eq!(config.action(1.0), DeduplicationAction::Merged);
        assert_eq!(config.action(0.90), DeduplicationAction::Merged);
    }

    #[test]
    fn action_flags_between_the_thresholds() {
        let config = ResolutionConfig::default();
        assert_eq!(config.action(0.89), DeduplicationAction::Flagged);
        assert_eq!(config.action(0.85), DeduplicationAction::Flagged);
    }

    #[test]
    fn action_is_none_below_the_review_threshold() {
        let config = ResolutionConfig::default();
        assert_eq!(config.action(0.84), DeduplicationAction::None);
        assert_eq!(config.action(0.0), DeduplicationAction::None);
    }

    #[test]
    fn deduplication_action_defaults_to_none() {
        assert_eq!(DeduplicationAction::default(), DeduplicationAction::None);
    }

    #[test]
    fn deduplication_result_defaults_are_empty() {
        let result = DeduplicationResult::default();
        assert!(!result.is_duplicate);
        assert_eq!(result.action, DeduplicationAction::None);
        assert_eq!(result.matched_entity_id, None);
        assert_eq!(result.matched_entity_name, None);
        assert!((result.similarity_score - 0.0).abs() < f64::EPSILON);
        assert_eq!(result.match_type, None);
    }

    #[test]
    fn matches_surface_compares_name_canonical_name_and_aliases() {
        let mut subject = entity("Ada Lovelace");
        assert!(matches_surface(&subject, "Ada Lovelace"));
        assert!(matches_surface(&subject, "  ada lovelace  "));
        assert!(!matches_surface(&subject, "Ada"));

        subject.canonical_name = Some("Ada King".to_owned());
        assert!(matches_surface(&subject, "ada king"));

        subject.aliases.push("Countess of Kept".to_owned());
        assert!(matches_surface(&subject, "countess of kept"));
        assert!(!matches_surface(&subject, "Grace Hopper"));
    }

    #[test]
    fn fuzzy_scorer_is_pinned() {
        assert_eq!(FUZZY_SCORER, "token_sort_ratio");
    }
}
