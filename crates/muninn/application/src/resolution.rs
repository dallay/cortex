//! Single-entity resolution, separate from preference/fact duplicate folding.
use muninn_core::model::Entity;
use muninn_core::ports::Scored;
use muninn_core::resolution::{DeduplicationAction, DeduplicationResult, ResolutionConfig};

/// Select one candidate. The persistence adapter owns blocking, not graph writes.
pub fn resolve(
    name: &str,
    entity_type: &str,
    candidates: Vec<Scored<Entity>>,
    config: &ResolutionConfig,
) -> DeduplicationResult {
    let mut best = DeduplicationResult::default();
    let limit = usize::try_from(config.candidate_limit).unwrap_or(usize::MAX);
    for hit in candidates
        .into_iter()
        .filter(|hit| !config.match_same_type_only || hit.item.entity_type == entity_type)
        .take(limit)
    {
        let surfaces = std::iter::once(hit.item.name.as_str())
            .chain(hit.item.canonical_name.as_deref())
            .chain(hit.item.aliases.iter().map(String::as_str));
        let exact = muninn_core::resolution::matches_surface(&hit.item, name);
        let mut fuzzy: f64 = 0.0;
        for surface in surfaces {
            fuzzy = fuzzy.max(token_sort_ratio(name, surface));
        }
        let semantic = if config.use_embedding_blocking
            && hit.score.is_finite()
            && hit.score >= config.semantic_threshold
        {
            hit.score.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (score, kind) = if exact {
            (1.0, "exact")
        } else if fuzzy >= config.fuzzy_threshold {
            if semantic > 0.0 {
                ((semantic + fuzzy) / 2.0, "both")
            } else {
                (fuzzy, "fuzzy")
            }
        } else {
            (semantic, "embedding")
        };
        let action = config.action(score);
        if action != DeduplicationAction::None
            && (!best.is_duplicate
                || score > best.similarity_score
                || (score == best.similarity_score
                    && kind == "exact"
                    && best.match_type.as_deref() != Some("exact")))
        {
            best = DeduplicationResult {
                is_duplicate: true,
                action,
                matched_entity_id: Some(hit.item.id),
                matched_entity_name: Some(hit.item.name),
                similarity_score: score,
                match_type: Some(kind.into()),
            };
        }
    }
    best
}

/// RapidFuzz token_sort_ratio composition and normalized indel similarity.
/// Linear-space LCS avoids a quadratic allocation; no optional model dependency.
fn token_sort_ratio(left: &str, right: &str) -> f64 {
    fn sorted(value: &str) -> Vec<char> {
        let folded = value.to_lowercase();
        let mut tokens: Vec<_> = folded.split_whitespace().collect();
        tokens.sort_unstable();
        tokens.join(" ").chars().collect()
    }
    let left = sorted(left);
    let right = sorted(right);
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let mut row = vec![0_usize; right.len() + 1];
    for a in &left {
        let mut diagonal = 0;
        for (index, b) in right.iter().enumerate() {
            let previous = row[index + 1];
            row[index + 1] = if a == b {
                diagonal + 1
            } else {
                row[index + 1].max(row[index])
            };
            diagonal = previous;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    {
        2.0 * row[right.len()] as f64 / (left.len() + right.len()) as f64
    }
}
