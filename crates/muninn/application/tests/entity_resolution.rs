//! Integration coverage for single-entity resolution.
//!
//! The fuzzy scorer is private to the resolver, so its score is observed
//! through the resolver result: the reported `similarity_score` is the
//! combined `(embedding + fuzzy) / 2` value of the winning candidate.

use muninn_application::resolution::resolve;
use muninn_core::model::{Entity, Metadata};
use muninn_core::ports::Scored;
use muninn_core::resolution::{DeduplicationAction, ResolutionConfig};
use uuid::Uuid;

/// Tolerance for the combined score, which averages two rounded doubles.
const EPSILON: f64 = 1e-9;

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

fn scored(entity: Entity, score: f64) -> Vec<Scored<Entity>> {
    vec![Scored {
        item: entity,
        score,
    }]
}

#[test]
fn resolve_without_candidates_reports_no_duplicate() {
    let config = ResolutionConfig::default();
    let result = resolve("Ada", "PERSON", vec![], &config);
    assert!(!result.is_duplicate);
    assert_eq!(result.action, DeduplicationAction::None);
    assert_eq!(result.similarity_score, 0.0);
    assert_eq!(result.matched_entity_id, None);
}

#[test]
fn resolve_ignores_candidates_below_the_review_threshold() {
    let config = ResolutionConfig {
        review_threshold: 0.8,
        ..Default::default()
    };
    let entity = sample_entity(Uuid::new_v4(), "Ada", "PERSON");
    let result = resolve("Ada", "PERSON", scored(entity, 0.75), &config);
    assert!(!result.is_duplicate);
    assert_eq!(result.action, DeduplicationAction::None);
}

#[test]
fn resolve_ignores_candidates_of_another_entity_type() {
    let config = ResolutionConfig {
        match_same_type_only: true,
        review_threshold: 0.5,
        ..Default::default()
    };
    let entity = sample_entity(Uuid::new_v4(), "Ada", "ORGANIZATION");
    let result = resolve("Ada", "PERSON", scored(entity, 0.9), &config);
    assert!(!result.is_duplicate);
}

#[test]
fn resolve_combines_the_embedding_and_fuzzy_scores() {
    let config = ResolutionConfig {
        use_fuzzy_matching: true,
        fuzzy_threshold: 0.8,
        review_threshold: 0.5,
        ..Default::default()
    };
    let mut entity = sample_entity(Uuid::new_v4(), "Ada Lovelace", "PERSON");
    entity.canonical_name = Some("Lady Ada".into());
    let id = entity.id;
    let result = resolve("Ada Lovelace", "PERSON", scored(entity, 0.85), &config);
    assert!(result.is_duplicate);
    assert_eq!(result.match_type.as_deref(), Some("both"));
    assert_eq!(result.matched_entity_id, Some(id));
    // The fuzzy score is 1.0 for an identical name, so the combined score is
    // `(0.85 + 1.0) / 2`.
    assert!((result.similarity_score - 0.925).abs() < EPSILON);
}

#[test]
fn resolve_falls_back_to_the_embedding_score_when_the_fuzzy_score_is_too_low() {
    let config = ResolutionConfig {
        use_fuzzy_matching: true,
        fuzzy_threshold: 0.95,
        review_threshold: 0.5,
        ..Default::default()
    };
    let entity = sample_entity(Uuid::new_v4(), "Completely Different Name", "PERSON");
    let id = entity.id;
    let result = resolve("Ada", "PERSON", scored(entity, 0.85), &config);
    assert!(result.is_duplicate);
    assert_eq!(result.match_type.as_deref(), Some("embedding"));
    assert_eq!(result.matched_entity_id, Some(id));
    assert_eq!(result.similarity_score, 0.85);
}

#[test]
fn resolve_reports_a_full_fuzzy_score_for_exact_and_case_insensitive_names() {
    let config = ResolutionConfig {
        use_fuzzy_matching: true,
        fuzzy_threshold: 0.5,
        review_threshold: 0.5,
        ..Default::default()
    };

    let exact = sample_entity(Uuid::new_v4(), "apple", "PERSON");
    let result = resolve("apple", "PERSON", scored(exact, 0.75), &config);
    assert_eq!(result.match_type.as_deref(), Some("both"));
    assert!((result.similarity_score - 0.875).abs() < EPSILON);

    let insensitive = sample_entity(Uuid::new_v4(), "apple", "PERSON");
    let result = resolve("Apple", "PERSON", scored(insensitive, 0.75), &config);
    assert_eq!(result.match_type.as_deref(), Some("both"));
    assert!((result.similarity_score - 0.875).abs() < EPSILON);

    // The canonical name is scored as well, so it can carry the fuzzy match.
    let mut canonical = sample_entity(Uuid::new_v4(), "Unrelated Label", "PERSON");
    canonical.canonical_name = Some("apple".into());
    let result = resolve("apple", "PERSON", scored(canonical, 0.75), &config);
    assert_eq!(result.match_type.as_deref(), Some("both"));
    assert!((result.similarity_score - 0.875).abs() < EPSILON);
}

#[test]
fn resolve_reports_a_zero_fuzzy_score_for_empty_names() {
    let config = ResolutionConfig {
        use_fuzzy_matching: true,
        // A zero threshold keeps the fuzzy branch reachable for an empty name.
        fuzzy_threshold: 0.0,
        review_threshold: 0.5,
        ..Default::default()
    };

    let both_empty = sample_entity(Uuid::new_v4(), "", "PERSON");
    let result = resolve("", "PERSON", scored(both_empty, 0.75), &config);
    assert_eq!(result.match_type.as_deref(), Some("both"));
    assert!((result.similarity_score - 0.375).abs() < EPSILON);

    let empty_left = sample_entity(Uuid::new_v4(), "apple", "PERSON");
    let result = resolve("", "PERSON", scored(empty_left, 0.75), &config);
    assert_eq!(result.match_type.as_deref(), Some("both"));
    assert!((result.similarity_score - 0.375).abs() < EPSILON);

    let empty_right = sample_entity(Uuid::new_v4(), "", "PERSON");
    let result = resolve("a", "PERSON", scored(empty_right, 0.75), &config);
    assert_eq!(result.match_type.as_deref(), Some("both"));
    assert!((result.similarity_score - 0.375).abs() < EPSILON);
}

#[test]
fn resolve_reports_the_partial_fuzzy_score_of_a_near_match() {
    let config = ResolutionConfig {
        use_fuzzy_matching: true,
        fuzzy_threshold: 0.5,
        review_threshold: 0.5,
        ..Default::default()
    };
    // "apple" and "apples" share five of eleven characters, so the fuzzy
    // score is `2 * 5 / 11` and the combined score is `(0.75 + 10/11) / 2`.
    let entity = sample_entity(Uuid::new_v4(), "apples", "PERSON");
    let result = resolve("apple", "PERSON", scored(entity, 0.75), &config);
    assert_eq!(result.match_type.as_deref(), Some("both"));
    assert!((result.similarity_score - 0.829_545_454_545_454_5).abs() < EPSILON);
    // The fuzzy score lifts the candidate above the raw embedding score.
    assert!(result.similarity_score > 0.75);
}
