//! Single-entity resolution, separate from preference/fact duplicate folding.
use muninn_core::model::Entity;
use muninn_core::ports::Scored;
use muninn_core::resolution::{DeduplicationResult, ResolutionConfig};

/// Select one candidate. The persistence adapter owns blocking, not graph writes.
///
/// The pinned `find_duplicate` loop is reproduced exactly: candidates arrive
/// pre-thresholded at the flag boundary by the vector query, `best_score`
/// starts at zero with strict-improvement selection, fuzzy scoring combines
/// `(embedding + fuzz.ratio) / 2` over the name and canonical name only and
/// always consumes the candidate, and there is no exact-match shortcut.
pub fn resolve(
    name: &str,
    entity_type: &str,
    candidates: Vec<Scored<Entity>>,
    config: &ResolutionConfig,
) -> DeduplicationResult {
    let mut best: Option<DeduplicationResult> = None;
    let mut best_score = 0.0_f64;
    let limit = usize::try_from(config.candidate_limit).unwrap_or(usize::MAX);
    for hit in candidates
        .into_iter()
        .filter(|hit| !config.match_same_type_only || hit.item.entity_type == entity_type)
        .take(limit)
    {
        let score = hit.score.clamp(0.0, 1.0);
        // The pinned vector query pre-filters candidates at flag_threshold;
        // reproduce that boundary here because test doubles inject raw scores.
        if score < config.review_threshold {
            continue;
        }
        let mut fuzzy_handled = false;
        if config.use_fuzzy_matching {
            let canonical = hit.item.canonical_name.as_deref().unwrap_or(&hit.item.name);
            let fuzzy = ratio(name, &hit.item.name).max(ratio(name, canonical));
            if fuzzy >= config.fuzzy_threshold {
                fuzzy_handled = true;
                let combined = (score + fuzzy) / 2.0;
                if combined > best_score {
                    best_score = combined;
                    best = Some(DeduplicationResult {
                        is_duplicate: true,
                        action: config.action(combined),
                        matched_entity_id: Some(hit.item.id),
                        matched_entity_name: Some(hit.item.name.clone()),
                        similarity_score: combined,
                        match_type: Some("both".into()),
                    });
                }
            }
        }
        if !fuzzy_handled && score > best_score {
            best_score = score;
            best = Some(DeduplicationResult {
                is_duplicate: true,
                action: config.action(score),
                matched_entity_id: Some(hit.item.id),
                matched_entity_name: Some(hit.item.name),
                similarity_score: score,
                match_type: Some("embedding".into()),
            });
        }
    }
    best.unwrap_or_default()
}

/// RapidFuzz `fuzz.ratio`: normalized indel similarity over the lowercased raw
/// strings. Linear-space LCS avoids a quadratic allocation; no model dependency.
fn ratio(left: &str, right: &str) -> f64 {
    let left: Vec<char> = left.to_lowercase().chars().collect();
    let right: Vec<char> = right.to_lowercase().chars().collect();
    indel_similarity(&left, &right)
}

fn indel_similarity(left: &[char], right: &[char]) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let mut row = vec![0_usize; right.len() + 1];
    for a in left {
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

#[cfg(test)]
mod tests {
    use super::*;
    use muninn_core::model::Metadata;
    use muninn_core::resolution::DeduplicationAction;
    use uuid::Uuid;

    fn sample_entity(id: Uuid, name: &str, entity_type: &str) -> Entity {
        Entity {
            id,
            name: name.to_string(),
            entity_type: entity_type.to_string(),
            subtype: None,
            description: None,
            aliases: vec![],
            canonical_name: None,
            metadata: Metadata::new(),
            created_at: chrono::Utc::now(),
            updated_at: None,
            confidence: 1.0,
            embedding: None,
        }
    }

    #[test]
    fn test_ratio_empty_and_exact() {
        assert_eq!(ratio("", ""), 0.0);
        assert_eq!(ratio("a", ""), 0.0);
        assert_eq!(ratio("", "b"), 0.0);
        assert_eq!(ratio("apple", "apple"), 1.0);
        assert_eq!(ratio("Apple", "apple"), 1.0);
        assert!(ratio("apple", "apples") > 0.8);
    }

    #[test]
    fn test_resolve_empty_candidates() {
        let config = ResolutionConfig::default();
        let res = resolve("Ada", "PERSON", vec![], &config);
        assert!(!res.is_duplicate);
        assert_eq!(res.action, DeduplicationAction::None);
    }

    #[test]
    fn test_resolve_filters_below_review_threshold() {
        let config = ResolutionConfig {
            review_threshold: 0.8,
            ..Default::default()
        };
        let entity = sample_entity(Uuid::new_v4(), "Ada", "PERSON");
        let candidates = vec![Scored {
            item: entity,
            score: 0.75,
        }];
        let res = resolve("Ada", "PERSON", candidates, &config);
        assert!(!res.is_duplicate);
    }

    #[test]
    fn test_resolve_type_mismatch_filtering() {
        let config = ResolutionConfig {
            match_same_type_only: true,
            review_threshold: 0.5,
            ..Default::default()
        };
        let entity = sample_entity(Uuid::new_v4(), "Ada", "ORGANIZATION");
        let candidates = vec![Scored {
            item: entity,
            score: 0.9,
        }];
        let res = resolve("Ada", "PERSON", candidates, &config);
        assert!(!res.is_duplicate);
    }

    #[test]
    fn test_resolve_fuzzy_match_both() {
        let config = ResolutionConfig {
            use_fuzzy_matching: true,
            fuzzy_threshold: 0.8,
            review_threshold: 0.5,
            ..Default::default()
        };
        let mut entity = sample_entity(Uuid::new_v4(), "Ada Lovelace", "PERSON");
        entity.canonical_name = Some("Lady Ada".into());
        let candidates = vec![Scored {
            item: entity.clone(),
            score: 0.85,
        }];
        let res = resolve("Ada Lovelace", "PERSON", candidates, &config);
        assert!(res.is_duplicate);
        assert_eq!(res.match_type.as_deref(), Some("both"));
        assert_eq!(res.matched_entity_id, Some(entity.id));
    }

    #[test]
    fn test_resolve_embedding_only_when_fuzzy_fails() {
        let config = ResolutionConfig {
            use_fuzzy_matching: true,
            fuzzy_threshold: 0.95,
            review_threshold: 0.5,
            ..Default::default()
        };
        let entity = sample_entity(Uuid::new_v4(), "Completely Different Name", "PERSON");
        let candidates = vec![Scored {
            item: entity.clone(),
            score: 0.85,
        }];
        let res = resolve("Ada", "PERSON", candidates, &config);
        assert!(res.is_duplicate);
        assert_eq!(res.match_type.as_deref(), Some("embedding"));
        assert_eq!(res.matched_entity_id, Some(entity.id));
    }
}
