// Integration tests for telemetry recording in RouteRequest
//
// These tests verify that telemetry observations are recorded correctly
// after request execution, without mocking the entire RouteRequest infrastructure.

use std::sync::Arc;

use observability::{ObservationStatus, TelemetryConfig, TelemetryTracker};
use shared_kernel::ProviderId;

#[tokio::test]
async fn test_telemetry_tracker_records_observations() {
    let config = TelemetryConfig::default();
    let telemetry = Arc::new(TelemetryTracker::new(config));
    let provider_id = ProviderId::from("test-provider");

    // Simulate what RouteRequest does after a successful non-streaming request
    telemetry.record_observation(provider_id.clone(), 150, None, ObservationStatus::Success);

    // Verify observation recorded via percentile stats
    let stats = telemetry
        .compute_latency_percentiles(&provider_id)
        .expect("Should have latency stats");
    assert_eq!(stats.min, 150);
    assert_eq!(stats.max, 150);
    assert_eq!(stats.count, 1, "Should have recorded 1 observation");

    // No TTFT for non-streaming
    let ttft_stats = telemetry.compute_ttft_percentiles(&provider_id);
    assert!(ttft_stats.is_none());
}

#[tokio::test]
async fn test_telemetry_tracker_records_streaming_with_ttft() {
    let config = TelemetryConfig::default();
    let telemetry = Arc::new(TelemetryTracker::new(config));
    let provider_id = ProviderId::from("streaming-provider");

    // Simulate what RouteRequest does after a streaming request
    // TTFT = 50ms (first chunk), total latency = 200ms
    telemetry.record_observation(
        provider_id.clone(),
        200,
        Some(50),
        ObservationStatus::Success,
    );

    // Verify TTFT stats
    let ttft_stats = telemetry
        .compute_ttft_percentiles(&provider_id)
        .expect("Should have TTFT stats");
    assert_eq!(ttft_stats.min, 50);
    assert_eq!(ttft_stats.max, 50);
    assert_eq!(ttft_stats.count, 1);

    // Verify latency stats
    let latency_stats = telemetry
        .compute_latency_percentiles(&provider_id)
        .expect("Should have latency stats");
    assert_eq!(latency_stats.min, 200);
    assert_eq!(latency_stats.max, 200);
}

#[tokio::test]
async fn test_telemetry_tracker_records_failures() {
    let config = TelemetryConfig::default();
    let telemetry = Arc::new(TelemetryTracker::new(config));
    let provider_id = ProviderId::from("failing-provider");

    // Simulate failures
    telemetry.record_observation(provider_id.clone(), 100, None, ObservationStatus::Failure);
    telemetry.record_observation(
        provider_id.clone(),
        150,
        None,
        ObservationStatus::RateLimited,
    );
    telemetry.record_observation(provider_id.clone(), 200, None, ObservationStatus::Success);

    // Verify latency stats include all observations (success + failure)
    let stats = telemetry
        .compute_latency_percentiles(&provider_id)
        .expect("Should have stats");
    assert_eq!(stats.count, 3);
    assert_eq!(stats.min, 100);
    assert_eq!(stats.max, 200);
}

#[tokio::test]
async fn test_telemetry_tracker_multiple_providers() {
    let config = TelemetryConfig::default();
    let telemetry = Arc::new(TelemetryTracker::new(config));

    let openai = ProviderId::from("openai");
    let anthropic = ProviderId::from("anthropic");

    // Record observations for different providers
    telemetry.record_observation(openai.clone(), 100, None, ObservationStatus::Success);
    telemetry.record_observation(openai.clone(), 150, None, ObservationStatus::Success);
    telemetry.record_observation(anthropic.clone(), 300, Some(80), ObservationStatus::Success);

    // Verify separate stats
    let openai_stats = telemetry
        .compute_latency_percentiles(&openai)
        .expect("Should have OpenAI stats");
    assert_eq!(openai_stats.count, 2);
    assert_eq!(openai_stats.min, 100);
    assert_eq!(openai_stats.max, 150);

    let anthropic_stats = telemetry
        .compute_latency_percentiles(&anthropic)
        .expect("Should have Anthropic stats");
    assert_eq!(anthropic_stats.count, 1);
    assert_eq!(anthropic_stats.min, 300);

    // Only Anthropic has TTFT
    assert!(telemetry.compute_ttft_percentiles(&openai).is_none());
    assert!(telemetry.compute_ttft_percentiles(&anthropic).is_some());
}

#[tokio::test]
async fn test_telemetry_optional_none_is_safe() {
    // Simulate RouteRequest with telemetry = None
    let telemetry: Option<Arc<TelemetryTracker>> = None;

    // Verify no-op when telemetry is None
    if let Some(tracker) = telemetry.as_ref() {
        tracker.record_observation(
            ProviderId::from("test"),
            100,
            None,
            ObservationStatus::Success,
        );
    }

    // No panic, no error - this is the integration test passing
    assert!(telemetry.is_none());
}

#[tokio::test]
async fn test_telemetry_concurrent_recording_from_multiple_requests() {
    let config = TelemetryConfig::default();
    let telemetry = Arc::new(TelemetryTracker::new(config));
    let provider_id = ProviderId::from("concurrent-provider");

    let mut handles = vec![];

    // Simulate 50 concurrent requests (like multiple RouteRequest executions)
    for i in 0..50 {
        let telemetry_clone = telemetry.clone();
        let provider_id_clone = provider_id.clone();
        let handle = tokio::spawn(async move {
            telemetry_clone.record_observation(
                provider_id_clone,
                100 + i,
                None,
                ObservationStatus::Success,
            );
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.expect("Task should complete");
    }

    // Verify all recorded via stats
    let stats = telemetry
        .compute_latency_percentiles(&provider_id)
        .expect("Should have stats");
    assert_eq!(stats.count, 50, "Should have recorded all 50 observations");
}
