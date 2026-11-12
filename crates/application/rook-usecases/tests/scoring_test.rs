use chrono::Utc;
use rook_usecases::scoring::*;
use rook_usecases::TargetRef;
use shared_kernel::ProviderId;
use std::collections::HashMap;

fn make_snapshot(values: Vec<(TargetRef, RawEvidence<f64>, RawEvidence<f64>)>) -> SignalsSnapshot {
    let mut candidates = HashMap::new();
    for (target, p50, p95) in values {
        candidates.insert(
            target,
            CandidateSignals {
                latency_p50_ms: p50,
                latency_p95_ms: p95,
                pricing: None,
                priority: RawEvidence::Unknown(UnknownReason::Missing),
                quota: None,
                circuit: None,
            },
        );
    }
    SignalsSnapshot {
        captured_at: Utc::now(),
        candidates,
    }
}

fn target(id: &str) -> TargetRef {
    TargetRef(ProviderId::new(id.to_string()))
}

#[test]
fn inverse_min_max_normalization() {
    // [10, 20, 30] → [1.0, 0.5, 0.0]
    let snapshot = make_snapshot(vec![
        (
            target("a"),
            RawEvidence::Known(10.0),
            RawEvidence::Known(10.0),
        ),
        (
            target("b"),
            RawEvidence::Known(20.0),
            RawEvidence::Known(20.0),
        ),
        (
            target("c"),
            RawEvidence::Known(30.0),
            RawEvidence::Known(30.0),
        ),
    ]);
    let components = build_score_components(&snapshot);

    let a = components.get(&target("a")).unwrap();
    let b = components.get(&target("b")).unwrap();
    let c = components.get(&target("c")).unwrap();

    assert!((a.latency_p50.normalized.unwrap() - 1.0).abs() < f64::EPSILON);
    assert!((b.latency_p50.normalized.unwrap() - 0.5).abs() < f64::EPSILON);
    assert!((c.latency_p50.normalized.unwrap() - 0.0).abs() < f64::EPSILON);
}

#[test]
fn equal_values_normalize_to_half() {
    // [20, 20, 20] → [0.5, 0.5, 0.5]
    let snapshot = make_snapshot(vec![
        (
            target("a"),
            RawEvidence::Known(20.0),
            RawEvidence::Known(20.0),
        ),
        (
            target("b"),
            RawEvidence::Known(20.0),
            RawEvidence::Known(20.0),
        ),
        (
            target("c"),
            RawEvidence::Known(20.0),
            RawEvidence::Known(20.0),
        ),
    ]);
    let components = build_score_components(&snapshot);

    for comp in components.values() {
        assert!(
            (comp.latency_p50.normalized.unwrap() - 0.5).abs() < f64::EPSILON,
            "equal values should normalize to 0.5"
        );
    }
}

#[test]
fn single_known_value_normalizes_to_half() {
    let snapshot = make_snapshot(vec![(
        target("a"),
        RawEvidence::Known(15.0),
        RawEvidence::Known(15.0),
    )]);
    let components = build_score_components(&snapshot);

    let a = components.get(&target("a")).unwrap();
    assert!(
        (a.latency_p50.normalized.unwrap() - 0.5).abs() < f64::EPSILON,
        "single known value should normalize to 0.5"
    );
}

#[test]
fn no_known_values_stay_unknown() {
    let snapshot = make_snapshot(vec![
        (
            target("a"),
            RawEvidence::Unknown(UnknownReason::Missing),
            RawEvidence::Unknown(UnknownReason::Missing),
        ),
        (
            target("b"),
            RawEvidence::Unknown(UnknownReason::Missing),
            RawEvidence::Unknown(UnknownReason::Missing),
        ),
    ]);
    let components = build_score_components(&snapshot);

    for comp in components.values() {
        assert!(
            comp.latency_p50.normalized.is_none(),
            "no known values should remain unknown"
        );
    }
}

#[test]
fn mixed_known_unknown_unknowns_dont_affect_bounds() {
    // Known: [10, 30], Unknown should not affect bounds
    let snapshot = make_snapshot(vec![
        (
            target("a"),
            RawEvidence::Known(10.0),
            RawEvidence::Known(10.0),
        ),
        (
            target("b"),
            RawEvidence::Unknown(UnknownReason::Missing),
            RawEvidence::Unknown(UnknownReason::Missing),
        ),
        (
            target("c"),
            RawEvidence::Known(30.0),
            RawEvidence::Known(30.0),
        ),
    ]);
    let components = build_score_components(&snapshot);

    let a = components.get(&target("a")).unwrap();
    let b = components.get(&target("b")).unwrap();
    let c = components.get(&target("c")).unwrap();

    // a=10 should normalize to 1.0 (best), c=30 to 0.0 (worst)
    assert!((a.latency_p50.normalized.unwrap() - 1.0).abs() < f64::EPSILON);
    assert!(b.latency_p50.normalized.is_none(), "unknown stays unknown");
    assert!((c.latency_p50.normalized.unwrap() - 0.0).abs() < f64::EPSILON);
}

#[test]
fn known_zero_vs_unknown() {
    // Known(0) enters bounds, unknown doesn't
    let snapshot = make_snapshot(vec![
        (
            target("a"),
            RawEvidence::Known(0.0),
            RawEvidence::Known(0.0),
        ),
        (
            target("b"),
            RawEvidence::Unknown(UnknownReason::Missing),
            RawEvidence::Unknown(UnknownReason::Missing),
        ),
        (
            target("c"),
            RawEvidence::Known(20.0),
            RawEvidence::Known(20.0),
        ),
    ]);
    let components = build_score_components(&snapshot);

    let a = components.get(&target("a")).unwrap();
    let b = components.get(&target("b")).unwrap();
    let c = components.get(&target("c")).unwrap();

    // a=0 should normalize to 1.0 (best), c=20 to 0.0 (worst)
    assert!((a.latency_p50.normalized.unwrap() - 1.0).abs() < f64::EPSILON);
    assert!(b.latency_p50.normalized.is_none(), "unknown stays unknown");
    assert!((c.latency_p50.normalized.unwrap() - 0.0).abs() < f64::EPSILON);
}

#[test]
fn invalid_negative_values_are_unknown() {
    // Negative latency should be treated as unknown
    let snapshot = make_snapshot(vec![
        (
            target("a"),
            RawEvidence::Known(10.0),
            RawEvidence::Known(10.0),
        ),
        (
            target("b"),
            RawEvidence::Unknown(UnknownReason::Invalid),
            RawEvidence::Unknown(UnknownReason::Invalid),
        ),
    ]);
    let components = build_score_components(&snapshot);

    let a = components.get(&target("a")).unwrap();
    let b = components.get(&target("b")).unwrap();

    // a should normalize to 0.5 (single known value)
    assert!(
        (a.latency_p50.normalized.unwrap() - 0.5).abs() < f64::EPSILON,
        "single known value should normalize to 0.5"
    );
    assert!(b.latency_p50.normalized.is_none(), "invalid stays unknown");
}

#[test]
fn p50_p95_normalized_separately() {
    let snapshot = make_snapshot(vec![
        (
            target("a"),
            RawEvidence::Known(10.0),
            RawEvidence::Known(50.0),
        ),
        (
            target("b"),
            RawEvidence::Known(30.0),
            RawEvidence::Known(90.0),
        ),
    ]);
    let components = build_score_components(&snapshot);

    let a = components.get(&target("a")).unwrap();
    let b = components.get(&target("b")).unwrap();

    // p50: [10, 30] → a=1.0, b=0.0
    assert!((a.latency_p50.normalized.unwrap() - 1.0).abs() < f64::EPSILON);
    assert!((b.latency_p50.normalized.unwrap() - 0.0).abs() < f64::EPSILON);

    // p95: [50, 90] → a=1.0, b=0.0
    assert!((a.latency_p95.normalized.unwrap() - 1.0).abs() < f64::EPSILON);
    assert!((b.latency_p95.normalized.unwrap() - 0.0).abs() < f64::EPSILON);
}

#[test]
fn permuted_order_same_normalization() {
    // Same eligible set, permuted order → identical normalization results
    let snapshot1 = make_snapshot(vec![
        (
            target("a"),
            RawEvidence::Known(10.0),
            RawEvidence::Known(10.0),
        ),
        (
            target("b"),
            RawEvidence::Known(20.0),
            RawEvidence::Known(20.0),
        ),
        (
            target("c"),
            RawEvidence::Known(30.0),
            RawEvidence::Known(30.0),
        ),
    ]);
    let snapshot2 = make_snapshot(vec![
        (
            target("c"),
            RawEvidence::Known(30.0),
            RawEvidence::Known(30.0),
        ),
        (
            target("a"),
            RawEvidence::Known(10.0),
            RawEvidence::Known(10.0),
        ),
        (
            target("b"),
            RawEvidence::Known(20.0),
            RawEvidence::Known(20.0),
        ),
    ]);

    let comp1 = build_score_components(&snapshot1);
    let comp2 = build_score_components(&snapshot2);

    // Same target should have same normalization regardless of order
    let a1 = comp1.get(&target("a")).unwrap();
    let a2 = comp2.get(&target("a")).unwrap();
    assert!(
        (a1.latency_p50.normalized.unwrap() - a2.latency_p50.normalized.unwrap()).abs()
            < f64::EPSILON
    );
}

#[test]
fn distinct_tiny_bounds_normalize_to_extremes() {
    let snapshot = make_snapshot(vec![
        (
            target("a"),
            RawEvidence::Known(0.0),
            RawEvidence::Known(0.0),
        ),
        (
            target("b"),
            RawEvidence::Known(1e-16),
            RawEvidence::Known(1e-16),
        ),
    ]);
    let components = build_score_components(&snapshot);
    assert_eq!(components[&target("a")].latency_p50.normalized, Some(1.0));
    assert_eq!(components[&target("b")].latency_p50.normalized, Some(0.0));
    assert_eq!(
        build_selection_reason(&snapshot, &target("a"), SelectionMethod::Priority)
            .unwrap()
            .fallback,
        None
    );
    let single = make_snapshot(vec![(
        target("a"),
        RawEvidence::Known(0.0),
        RawEvidence::Known(0.0),
    )]);
    assert_eq!(
        build_selection_reason(&single, &target("a"), SelectionMethod::Priority)
            .unwrap()
            .fallback,
        Some(SelectionFallback::SingleCandidate)
    );
}

#[test]
fn pricing_evidence_distinguishes_nonfinite_from_negative() {
    for (value, expected) in [
        (f64::NAN, UnknownReason::NonFinite),
        (f64::INFINITY, UnknownReason::NonFinite),
        (f64::NEG_INFINITY, UnknownReason::NonFinite),
        (-1.0, UnknownReason::Invalid),
    ] {
        let target = target("a");
        let snapshot = build_signals_snapshot(
            &[&target],
            &rook_core::ModelId::new("model"),
            &None,
            &rook_usecases::PricingConfig::from_entry("a", "model", value, value),
            &HashMap::new(),
            &HashMap::new(),
        );
        let pricing = snapshot.candidates[&target].pricing.as_ref().unwrap();
        assert_eq!(pricing.input_per_million, RawEvidence::Unknown(expected));
        assert_eq!(pricing.output_per_million, RawEvidence::Unknown(expected));
    }
}
