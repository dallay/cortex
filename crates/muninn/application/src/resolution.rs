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
                matched_entity_name: Some(hit.item.name.clone()),
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
