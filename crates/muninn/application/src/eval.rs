//! Product evaluation API (`BOLT-08-07`, `SDD-5B`, `F-13`).
//!
//! Equivalent to `neo4j_agent_memory.memory.eval.EvalMemory`.
//! Strictly separate from H5 retrieval acceptance (`muninn-h5-acceptance-protocol`)
//! and Huginn end-to-end utility benchmarking (`muninn-memory-evaluation-benchmarking`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use muninn_core::extraction::{EntityExtractor, ExtractOptions};
use muninn_core::MemoryResult;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::long_term::{LongTermMemory, SearchEntitiesOptions};

/// A single retrieval-evaluation case (`Recall@k`).
#[derive(Debug, Clone)]
pub struct RetrievalCase {
    pub query: String,
    pub expected_entity_ids: BTreeSet<Uuid>,
    pub k: i64,
}

/// A single preference-fidelity case (`active_only = true`).
#[derive(Debug, Clone)]
pub struct PreferenceCase {
    pub user_identifier: String,
    pub expected_active_pref_ids: BTreeSet<Uuid>,
}

/// Gold entity descriptor for extraction quality scoring.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExpectedEntity {
    pub name: String,
    pub entity_type: String,
}

/// Gold relation triple for extraction quality scoring.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExpectedRelation {
    pub source: String,
    pub relation_type: String,
    pub target: String,
}

/// A single extraction-quality case.
#[derive(Debug, Clone)]
pub struct ExtractionCase {
    pub text: String,
    pub expected_entities: Vec<ExpectedEntity>,
    pub expected_relations: Vec<ExpectedRelation>,
}

/// A single resolution-quality case (predicted vs. gold clusters scored with B-cubed F1).
#[derive(Debug, Clone)]
pub struct ResolutionCase {
    pub predicted_clusters: BTreeMap<String, String>,
    pub gold_clusters: BTreeMap<String, String>,
}

/// Dimension score summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionScore {
    pub cases: usize,
    pub score: f64,
}

/// Input bundle for [`EvalMemory::run`].
#[derive(Debug, Clone, Default)]
pub struct EvalRequest {
    pub retrieval: Vec<RetrievalCase>,
    pub preferences: Vec<PreferenceCase>,
    pub extraction: Vec<ExtractionCase>,
    pub resolution: Vec<ResolutionCase>,
}

/// Product evaluation report (`client.eval.run`).
///
/// Mirrors upstream `EvalReport`: optional dimensions without a configured
/// component are recorded in `skipped`. Release-gate validators MUST call
/// [`Self::has_skipped_required`] so skipped mandatory dimensions never count as `PASS`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EvalReport {
    pub retrieval: Option<DimensionScore>,
    pub preferences: Option<DimensionScore>,
    pub extraction: Option<DimensionScore>,
    pub resolution: Option<DimensionScore>,
    pub skipped: Vec<String>,
}

impl EvalReport {
    /// Returns `true` if any required dimension name was skipped or left uncomputed.
    pub fn has_skipped_required(&self, required: &[&str]) -> bool {
        required.iter().any(|name| {
            self.skipped.iter().any(|s| s == name)
                || match *name {
                    "retrieval" => self.retrieval.is_none(),
                    "preferences" => self.preferences.is_none(),
                    "extraction" => self.extraction.is_none(),
                    "resolution" => self.resolution.is_none(),
                    _ => true,
                }
        })
    }
}

/// Compute `Recall@k` for a ranked list of predicted entity IDs against expected IDs.
pub fn recall_at_k(predicted: &[Uuid], expected: &BTreeSet<Uuid>, k: usize) -> f64 {
    if expected.is_empty() {
        return 1.0;
    }
    let hits = predicted
        .iter()
        .take(k)
        .filter(|id| expected.contains(id))
        .collect::<BTreeSet<_>>()
        .len();
    hits as f64 / expected.len() as f64
}

/// Compute exact-match F1 between two sets (case-insensitive normalized keys).
pub fn set_f1(predicted: &BTreeSet<String>, expected: &BTreeSet<String>) -> f64 {
    if predicted.is_empty() && expected.is_empty() {
        return 1.0;
    }
    if predicted.is_empty() || expected.is_empty() {
        return 0.0;
    }
    let tp = predicted.intersection(expected).count() as f64;
    if tp == 0.0 {
        return 0.0;
    }
    let precision = tp / predicted.len() as f64;
    let recall = tp / expected.len() as f64;
    2.0 * precision * recall / (precision + recall)
}

/// Compute B-cubed F1 between predicted and gold mention->cluster maps.
/// All mentions present in either `predicted` or `gold` are evaluated.
/// Mentions present in only one map are treated as isolated singleton clusters in the other.
pub fn bcubed_f1(predicted: &BTreeMap<String, String>, gold: &BTreeMap<String, String>) -> f64 {
    let all_mentions: BTreeSet<&String> = gold.keys().chain(predicted.keys()).collect();
    if all_mentions.is_empty() {
        return 1.0;
    }
    let mentions: Vec<&String> = all_mentions.into_iter().collect();
    let mut precision_sum = 0.0;
    let mut recall_sum = 0.0;
    for (idx, m) in mentions.iter().enumerate() {
        // Fallback to synthetic unique cluster if mention is missing in either map
        let fallback = format!("__unmatched_{idx}");
        let p_cluster = predicted.get(*m).unwrap_or(&fallback);
        let g_cluster = gold.get(*m).unwrap_or(&fallback);
        let same_pred: Vec<_> = mentions
            .iter()
            .enumerate()
            .filter(|(i, o)| {
                let fb = format!("__unmatched_{i}");
                predicted.get(**o).unwrap_or(&fb) == p_cluster
            })
            .map(|(_, o)| o)
            .collect();
        let same_gold: Vec<_> = mentions
            .iter()
            .enumerate()
            .filter(|(i, o)| {
                let fb = format!("__unmatched_{i}");
                gold.get(**o).unwrap_or(&fb) == g_cluster
            })
            .map(|(_, o)| o)
            .collect();
        let both = same_pred
            .iter()
            .filter(|o| {
                let fb = format!("__unmatched_{idx}");
                gold.get(***o).unwrap_or(&fb) == g_cluster
            })
            .count() as f64;
        precision_sum += both / same_pred.len() as f64;
        recall_sum += both / same_gold.len() as f64;
    }
    let precision = precision_sum / mentions.len() as f64;
    let recall = recall_sum / mentions.len() as f64;
    if precision + recall == 0.0 {
        0.0
    } else {
        2.0 * precision * recall / (precision + recall)
    }
}

/// High-level product evaluator (`client.eval`).
pub struct EvalMemory<'a> {
    long_term: &'a LongTermMemory,
    extractor: Option<Arc<dyn EntityExtractor>>,
}

impl<'a> EvalMemory<'a> {
    pub fn new(long_term: &'a LongTermMemory, extractor: Option<Arc<dyn EntityExtractor>>) -> Self {
        Self {
            long_term,
            extractor,
        }
    }

    /// Run all requested evaluation dimensions (`client.eval.run`).
    pub async fn run(&self, request: &EvalRequest) -> MemoryResult<EvalReport> {
        let mut report = EvalReport::default();

        if !request.retrieval.is_empty() {
            let mut total = 0.0;
            for case in &request.retrieval {
                if case.k == 0 {
                    total += recall_at_k(&[], &case.expected_entity_ids, 0);
                    continue;
                }
                let hits = self
                    .long_term
                    .search_entities(
                        &case.query,
                        &SearchEntitiesOptions {
                            limit: case.k,
                            threshold: 0.0,
                            ..Default::default()
                        },
                    )
                    .await?;
                let ids: Vec<Uuid> = hits.into_iter().map(|e| e.id).collect();
                total += recall_at_k(&ids, &case.expected_entity_ids, case.k as usize);
            }
            report.retrieval = Some(DimensionScore {
                cases: request.retrieval.len(),
                score: total / request.retrieval.len() as f64,
            });
        }

        if !request.preferences.is_empty() {
            let mut exact_matches = 0usize;
            for case in &request.preferences {
                let active = self
                    .long_term
                    .get_preferences_for(&case.user_identifier, true)
                    .await?;
                let actual: BTreeSet<Uuid> = active.into_iter().map(|p| p.id).collect();
                if actual == case.expected_active_pref_ids {
                    exact_matches += 1;
                }
            }
            report.preferences = Some(DimensionScore {
                cases: request.preferences.len(),
                score: exact_matches as f64 / request.preferences.len() as f64,
            });
        }

        if !request.extraction.is_empty() {
            if let Some(extractor) = &self.extractor {
                let mut total = 0.0;
                for case in &request.extraction {
                    let result = extractor
                        .extract(
                            &case.text,
                            &ExtractOptions {
                                extract_relations: Some(!case.expected_relations.is_empty()),
                                extract_preferences: Some(false),
                                ..Default::default()
                            },
                        )
                        .await?;
                    if !case.expected_relations.is_empty() {
                        let pred: BTreeSet<String> = result
                            .relations
                            .iter()
                            .map(|r| {
                                format!(
                                    "{}|{}|{}",
                                    r.source.to_lowercase(),
                                    r.relation_type.to_uppercase(),
                                    r.target.to_lowercase()
                                )
                            })
                            .collect();
                        let gold: BTreeSet<String> = case
                            .expected_relations
                            .iter()
                            .map(|r| {
                                format!(
                                    "{}|{}|{}",
                                    r.source.to_lowercase(),
                                    r.relation_type.to_uppercase(),
                                    r.target.to_lowercase()
                                )
                            })
                            .collect();
                        total += set_f1(&pred, &gold);
                    } else {
                        let pred: BTreeSet<String> = result
                            .entities
                            .iter()
                            .map(|e| {
                                format!(
                                    "{}|{}",
                                    e.name.to_lowercase(),
                                    e.entity_type.to_uppercase()
                                )
                            })
                            .collect();
                        let gold: BTreeSet<String> = case
                            .expected_entities
                            .iter()
                            .map(|e| {
                                format!(
                                    "{}|{}",
                                    e.name.to_lowercase(),
                                    e.entity_type.to_uppercase()
                                )
                            })
                            .collect();
                        total += set_f1(&pred, &gold);
                    }
                }
                report.extraction = Some(DimensionScore {
                    cases: request.extraction.len(),
                    score: total / request.extraction.len() as f64,
                });
            } else {
                report.skipped.push("extraction".to_owned());
            }
        }

        if !request.resolution.is_empty() {
            let mut total = 0.0;
            for case in &request.resolution {
                total += bcubed_f1(&case.predicted_clusters, &case.gold_clusters);
            }
            report.resolution = Some(DimensionScore {
                cases: request.resolution.len(),
                score: total / request.resolution.len() as f64,
            });
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recall_set_f1_and_bcubed_metrics_are_deterministic() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let expected = BTreeSet::from([a, b]);
        assert!((recall_at_k(&[a, c, b], &expected, 2) - 0.5).abs() < 1e-9);
        assert!((recall_at_k(&[a, c, b], &expected, 3) - 1.0).abs() < 1e-9);

        let pred = BTreeSet::from(["alice|PERSON".to_owned(), "neo4j|ORG".to_owned()]);
        let gold = BTreeSet::from(["alice|PERSON".to_owned()]);
        assert!((set_f1(&pred, &gold) - (2.0 / 3.0)).abs() < 1e-9);

        let pred_c = BTreeMap::from([
            ("m1".into(), "c1".into()),
            ("m2".into(), "c1".into()),
            ("m3".into(), "c2".into()),
        ]);
        let gold_c = BTreeMap::from([
            ("m1".into(), "g1".into()),
            ("m2".into(), "g1".into()),
            ("m3".into(), "g2".into()),
        ]);
        assert!((bcubed_f1(&pred_c, &gold_c) - 1.0).abs() < 1e-9);

        // Unmatched mentions penalty: missing predicted mention must not return perfect 1.0
        let pred_incomplete = BTreeMap::from([("m1".into(), "c1".into())]);
        let gold_two = BTreeMap::from([("m1".into(), "g1".into()), ("m2".into(), "g1".into())]);
        assert!(bcubed_f1(&pred_incomplete, &gold_two) < 0.99);
    }

    #[test]
    fn skipped_required_dimension_is_detected_for_release_gate() {
        let report = EvalReport {
            retrieval: Some(DimensionScore {
                cases: 1,
                score: 1.0,
            }),
            preferences: None,
            extraction: None,
            resolution: None,
            skipped: vec!["extraction".to_owned()],
        };
        assert!(!report.has_skipped_required(&["retrieval"]));
        assert!(report.has_skipped_required(&["retrieval", "extraction"]));
        assert!(report.has_skipped_required(&["preferences"]));
    }
}
