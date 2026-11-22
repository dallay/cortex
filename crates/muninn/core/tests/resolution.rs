//! Contract tests for entity-resolution policy and result types.

use chrono::{DateTime, Utc};
use muninn_core::config::ResolutionConfig as SettingsResolutionConfig;
use muninn_core::model::{Entity, Metadata};
use muninn_core::resolution::{
    matches_surface, DeduplicationAction, DeduplicationResult,
    ResolutionConfig as DeduplicationConfig, FUZZY_SCORER,
};
use uuid::Uuid;

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
    let config = DeduplicationConfig::default();
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
            (|config: &mut DeduplicationConfig| config.auto_merge_threshold = 1.5)
                as fn(&mut DeduplicationConfig),
        ),
        ("review_threshold", |config: &mut DeduplicationConfig| {
            config.review_threshold = -0.1
        }),
        ("exact_threshold", |config: &mut DeduplicationConfig| {
            config.exact_threshold = 2.0
        }),
        ("fuzzy_threshold", |config: &mut DeduplicationConfig| {
            config.fuzzy_threshold = -0.5
        }),
        ("semantic_threshold", |config: &mut DeduplicationConfig| {
            config.semantic_threshold = 1.1
        }),
    ] {
        let mut config = DeduplicationConfig::default();
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
    let config = DeduplicationConfig {
        review_threshold: 0.95,
        auto_merge_threshold: 0.90,
        ..DeduplicationConfig::default()
    };
    let error = config.validate().expect_err("inverted thresholds");
    assert_eq!(
        error.to_string(),
        "invalid input: resolution.review_threshold must be <= resolution.auto_merge_threshold (got 0.95 > 0.9)"
    );
}

#[test]
fn validation_rejects_a_candidate_limit_below_one() {
    let config = DeduplicationConfig {
        candidate_limit: 0,
        ..DeduplicationConfig::default()
    };
    let error = config.validate().expect_err("zero candidate limit");
    assert_eq!(
        error.to_string(),
        "invalid input: candidate_limit must be at least 1"
    );
}

#[test]
fn action_merges_at_or_above_the_auto_merge_threshold() {
    let config = DeduplicationConfig::default();
    assert_eq!(config.action(1.0), DeduplicationAction::Merged);
    assert_eq!(config.action(0.90), DeduplicationAction::Merged);
}

#[test]
fn action_flags_between_the_thresholds() {
    let config = DeduplicationConfig::default();
    assert_eq!(config.action(0.89), DeduplicationAction::Flagged);
    assert_eq!(config.action(0.85), DeduplicationAction::Flagged);
}

#[test]
fn action_is_none_below_the_review_threshold() {
    let config = DeduplicationConfig::default();
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
fn configured_fuzzy_scorer_matches_the_pinned_resolution_scorer() {
    // This is the in-core scorer-selection contract: configuration and the
    // resolver's pinned scorer must not drift independently. The `resolve`
    // entry point and its scorer application live in muninn-application.
    let configured_scorer = SettingsResolutionConfig::default().fuzzy_scorer;
    assert_eq!(configured_scorer, FUZZY_SCORER);
    assert_eq!(FUZZY_SCORER, "token_sort_ratio");
}
