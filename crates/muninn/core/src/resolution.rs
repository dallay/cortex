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
