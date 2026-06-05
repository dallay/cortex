// Telemetry tracker for request-level latency and TTFT metrics
//
// Design:
// - In-memory DashMap storage for lock-free concurrent writes
// - VecDeque per provider for efficient FIFO sliding window
// - Clone-before-sort for percentile computation (no write blocking)
// - Dual retention: count limit (1000) + time limit (1 hour)

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::Serialize;
use shared_kernel::ProviderId;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

/// Configuration for telemetry tracker
#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    /// Max observations per provider (default: 1000)
    pub max_observations: usize,
    /// Max age for observations (default: 1 hour)
    pub max_age: Duration,
    /// Background cleanup interval (default: 60 seconds)
    pub cleanup_interval: Duration,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            max_observations: 1000,
            max_age: Duration::from_secs(3600),
            cleanup_interval: Duration::from_secs(60),
        }
    }
}

/// Request outcome status for telemetry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ObservationStatus {
    Success,
    Failure,
    RateLimited,
}

/// Single latency observation
#[derive(Debug, Clone)]
pub struct LatencyObservation {
    pub latency_ms: u64,
    pub ttft_ms: Option<u64>,
    pub timestamp: DateTime<Utc>,
    pub status: ObservationStatus,
}

/// Aggregated metrics per provider
#[derive(Debug, Clone)]
struct ProviderMetrics {
    observations: VecDeque<LatencyObservation>,
    total_requests: u64,
    success_count: u64,
    failure_count: u64,
    rate_limited_count: u64,
}

impl ProviderMetrics {
    fn new(max_observations: usize) -> Self {
        Self {
            observations: VecDeque::with_capacity(max_observations),
            total_requests: 0,
            success_count: 0,
            failure_count: 0,
            rate_limited_count: 0,
        }
    }

    /// Add observation and apply count-based retention (eager pruning)
    fn add_observation(&mut self, observation: LatencyObservation, max_observations: usize) {
        self.total_requests += 1;
        match observation.status {
            ObservationStatus::Success => self.success_count += 1,
            ObservationStatus::Failure => self.failure_count += 1,
            ObservationStatus::RateLimited => self.rate_limited_count += 1,
        }

        self.observations.push_back(observation);

        // Enforce count limit (eager)
        if self.observations.len() > max_observations {
            self.observations.pop_front();
        }
    }

    /// Remove observations older than max_age
    fn enforce_retention(&mut self, max_age: Duration) {
        let cutoff = Utc::now() - chrono::Duration::from_std(max_age).unwrap();
        self.observations.retain(|obs| obs.timestamp > cutoff);
    }
}

/// Computed percentile statistics
#[derive(Debug, Clone, Serialize)]
pub struct PercentileStats {
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub avg: u64,
    pub min: u64,
    pub max: u64,
    pub count: usize,
}

/// Provider summary for HTTP API responses
#[derive(Debug, Clone, Serialize)]
pub struct ProviderSummary {
    pub provider_id: ProviderId,
    pub total_requests: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub rate_limited_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_stats: Option<PercentileStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttft_stats: Option<PercentileStats>,
}

/// Telemetry tracker for request-level metrics
pub struct TelemetryTracker {
    metrics: Arc<DashMap<ProviderId, ProviderMetrics>>,
    config: TelemetryConfig,
}

impl TelemetryTracker {
    /// Create a new telemetry tracker with default configuration
    pub fn new(config: TelemetryConfig) -> Self {
        Self {
            metrics: Arc::new(DashMap::new()),
            config,
        }
    }

    /// Record a request observation (non-blocking)
    pub fn record_observation(
        &self,
        provider_id: ProviderId,
        latency_ms: u64,
        ttft_ms: Option<u64>,
        status: ObservationStatus,
    ) {
        let observation = LatencyObservation {
            latency_ms,
            ttft_ms,
            timestamp: Utc::now(),
            status,
        };

        self.metrics
            .entry(provider_id)
            .or_insert_with(|| ProviderMetrics::new(self.config.max_observations))
            .add_observation(observation, self.config.max_observations);
    }

    /// Compute percentiles from sorted values (helper)
    fn calculate_percentiles(sorted: &[u64]) -> PercentileStats {
        let count = sorted.len();

        // Floor-based index calculation as per spec
        let p50_idx = ((count as f64) * 0.50).floor() as usize;
        let p95_idx = ((count as f64) * 0.95).floor() as usize;
        let p99_idx = ((count as f64) * 0.99).floor() as usize;

        let sum: u64 = sorted.iter().sum();
        let avg = sum / count as u64;

        PercentileStats {
            p50: sorted[p50_idx.min(count - 1)],
            p95: sorted[p95_idx.min(count - 1)],
            p99: sorted[p99_idx.min(count - 1)],
            avg,
            min: *sorted.first().unwrap(),
            max: *sorted.last().unwrap(),
            count,
        }
    }

    /// Compute latency percentiles for a provider
    pub fn compute_latency_percentiles(&self, provider_id: &ProviderId) -> Option<PercentileStats> {
        let entry = self.metrics.get(provider_id)?;
        let observations = &entry.observations;

        if observations.is_empty() {
            return None;
        }

        // Clone to vec and sort (no lock holding during sort)
        let mut latencies: Vec<u64> = observations.iter().map(|o| o.latency_ms).collect();
        latencies.sort_unstable();

        Some(Self::calculate_percentiles(&latencies))
    }

    /// Compute TTFT percentiles for a provider (streaming only)
    pub fn compute_ttft_percentiles(&self, provider_id: &ProviderId) -> Option<PercentileStats> {
        let entry = self.metrics.get(provider_id)?;
        let observations = &entry.observations;

        // Filter to only streaming requests with TTFT
        let mut ttfts: Vec<u64> = observations.iter().filter_map(|o| o.ttft_ms).collect();

        if ttfts.is_empty() {
            return None;
        }

        ttfts.sort_unstable();
        Some(Self::calculate_percentiles(&ttfts))
    }

    /// Get observation count for a provider (for testing)
    #[cfg(test)]
    pub fn observation_count(&self, provider_id: &ProviderId) -> usize {
        self.metrics
            .get(provider_id)
            .map(|entry| entry.observations.len())
            .unwrap_or(0)
    }

    /// Prune observations older than max_age (called by background task)
    pub fn cleanup_old_observations(&self) {
        for mut entry in self.metrics.iter_mut() {
            entry.enforce_retention(self.config.max_age);
        }
    }

    /// Spawn background cleanup task (call once at startup)
    pub fn start_cleanup_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let interval = self.config.cleanup_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                self.cleanup_old_observations();
            }
        })
    }

    /// Get summary for a single provider
    pub fn get_provider_summary(&self, provider_id: &ProviderId) -> Option<ProviderSummary> {
        let entry = self.metrics.get(provider_id)?;
        let latency_stats = self.compute_latency_percentiles(provider_id);
        let ttft_stats = self.compute_ttft_percentiles(provider_id);

        Some(ProviderSummary {
            provider_id: provider_id.clone(),
            total_requests: entry.total_requests,
            success_count: entry.success_count,
            failure_count: entry.failure_count,
            rate_limited_count: entry.rate_limited_count,
            latency_stats,
            ttft_stats,
        })
    }

    /// Get summaries for all providers
    pub fn get_all_summaries(&self) -> Vec<ProviderSummary> {
        self.metrics
            .iter()
            .filter_map(|entry| self.get_provider_summary(entry.key()))
            .collect()
    }

    /// Get the configuration (for reading max_age_seconds)
    pub fn config(&self) -> &TelemetryConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile_computation_empty() {
        let config = TelemetryConfig::default();
        let tracker = TelemetryTracker::new(config);
        let provider_id = ProviderId::from("openai");

        let result = tracker.compute_latency_percentiles(&provider_id);
        assert!(result.is_none(), "Empty observations should return None");
    }

    #[test]
    fn test_percentile_computation_single() {
        let config = TelemetryConfig::default();
        let tracker = TelemetryTracker::new(config);
        let provider_id = ProviderId::from("openai");

        tracker.record_observation(provider_id.clone(), 100, None, ObservationStatus::Success);

        let stats = tracker
            .compute_latency_percentiles(&provider_id)
            .expect("Should have stats");
        assert_eq!(stats.p50, 100);
        assert_eq!(stats.p95, 100);
        assert_eq!(stats.p99, 100);
        assert_eq!(stats.avg, 100);
        assert_eq!(stats.min, 100);
        assert_eq!(stats.max, 100);
        assert_eq!(stats.count, 1);
    }

    #[test]
    fn test_percentile_computation_ten_observations() {
        let config = TelemetryConfig::default();
        let tracker = TelemetryTracker::new(config);
        let provider_id = ProviderId::from("openai");

        // Record observations: [100, 150, 200, 250, 300, 350, 400, 450, 500, 1000]
        let latencies = vec![100, 150, 200, 250, 300, 350, 400, 450, 500, 1000];
        for latency in latencies {
            tracker.record_observation(
                provider_id.clone(),
                latency,
                None,
                ObservationStatus::Success,
            );
        }

        let stats = tracker
            .compute_latency_percentiles(&provider_id)
            .expect("Should have stats");

        // Floor-based indices: p50 = floor(10 * 0.50) = 5 → latencies[5] = 350
        // p95 = floor(10 * 0.95) = 9 → latencies[9] = 1000
        // p99 = floor(10 * 0.99) = 9 → latencies[9] = 1000
        assert_eq!(stats.p50, 350);
        assert_eq!(stats.p95, 1000);
        assert_eq!(stats.p99, 1000);
        assert_eq!(stats.avg, 370); // (100+150+...+1000) / 10
        assert_eq!(stats.min, 100);
        assert_eq!(stats.max, 1000);
        assert_eq!(stats.count, 10);
    }

    #[test]
    fn test_percentile_computation_thousand_observations() {
        let config = TelemetryConfig::default();
        let tracker = TelemetryTracker::new(config);
        let provider_id = ProviderId::from("openai");

        // Record 1000 observations: 1, 2, 3, ..., 1000
        for i in 1..=1000 {
            tracker.record_observation(
                provider_id.clone(),
                i as u64,
                None,
                ObservationStatus::Success,
            );
        }

        let stats = tracker
            .compute_latency_percentiles(&provider_id)
            .expect("Should have stats");

        // Floor-based: p50 = floor(1000 * 0.50) = 500 → value 501
        // p95 = floor(1000 * 0.95) = 950 → value 951
        // p99 = floor(1000 * 0.99) = 990 → value 991
        assert_eq!(stats.p50, 501);
        assert_eq!(stats.p95, 951);
        assert_eq!(stats.p99, 991);
        assert_eq!(stats.min, 1);
        assert_eq!(stats.max, 1000);
        assert_eq!(stats.count, 1000);
    }

    #[test]
    fn test_retention_policy_count_limit() {
        let config = TelemetryConfig {
            max_observations: 1000,
            ..Default::default()
        };
        let tracker = TelemetryTracker::new(config);
        let provider_id = ProviderId::from("openai");

        // Record 1500 observations
        for i in 1..=1500 {
            tracker.record_observation(
                provider_id.clone(),
                i as u64,
                None,
                ObservationStatus::Success,
            );
        }

        // Should only retain last 1000
        let count = tracker.observation_count(&provider_id);
        assert_eq!(
            count, 1000,
            "Should retain max 1000 observations, got {}",
            count
        );

        // Verify oldest observations removed (should start from 501)
        let stats = tracker
            .compute_latency_percentiles(&provider_id)
            .expect("Should have stats");
        assert_eq!(stats.min, 501, "Oldest observation should be 501");
        assert_eq!(stats.max, 1500, "Newest observation should be 1500");
    }

    #[test]
    fn test_retention_policy_time_limit() {
        let config = TelemetryConfig {
            max_age: Duration::from_secs(3600), // 1 hour
            ..Default::default()
        };
        let max_observations = config.max_observations;
        let tracker = TelemetryTracker::new(config);
        let provider_id = ProviderId::from("openai");

        // Record observations with fake old timestamps
        let now = Utc::now();
        let old_timestamp = now - chrono::Duration::seconds(7200); // 2 hours ago
        let recent_timestamp = now - chrono::Duration::seconds(1800); // 30 min ago

        // Manually insert observations with custom timestamps
        let old_obs = LatencyObservation {
            latency_ms: 100,
            ttft_ms: None,
            timestamp: old_timestamp,
            status: ObservationStatus::Success,
        };
        let recent_obs = LatencyObservation {
            latency_ms: 200,
            ttft_ms: None,
            timestamp: recent_timestamp,
            status: ObservationStatus::Success,
        };

        // Insert via metrics directly (bypass record_observation timestamp)
        tracker
            .metrics
            .entry(provider_id.clone())
            .or_insert_with(|| ProviderMetrics::new(max_observations))
            .observations
            .push_back(old_obs);
        tracker
            .metrics
            .get_mut(&provider_id)
            .unwrap()
            .observations
            .push_back(recent_obs);

        assert_eq!(tracker.observation_count(&provider_id), 2);

        // Run cleanup
        tracker.cleanup_old_observations();

        // Only recent observation should remain
        let count = tracker.observation_count(&provider_id);
        assert_eq!(count, 1, "Should have 1 observation after cleanup");

        let stats = tracker
            .compute_latency_percentiles(&provider_id)
            .expect("Should have stats");
        assert_eq!(stats.min, 200, "Only recent observation should remain");
    }

    #[test]
    fn test_ttft_filtering() {
        let config = TelemetryConfig::default();
        let tracker = TelemetryTracker::new(config);
        let provider_id = ProviderId::from("anthropic");

        // Record mix: 5 streaming (with TTFT), 5 non-streaming (no TTFT)
        for i in 1..=5 {
            tracker.record_observation(
                provider_id.clone(),
                i * 100,
                Some(i * 50),
                ObservationStatus::Success,
            );
        }
        for i in 6..=10 {
            tracker.record_observation(
                provider_id.clone(),
                i * 100,
                None,
                ObservationStatus::Success,
            );
        }

        // Latency percentiles should include all 10
        let latency_stats = tracker
            .compute_latency_percentiles(&provider_id)
            .expect("Should have latency stats");
        assert_eq!(latency_stats.count, 10);

        // TTFT percentiles should only include 5 streaming requests
        let ttft_stats = tracker
            .compute_ttft_percentiles(&provider_id)
            .expect("Should have TTFT stats");
        assert_eq!(
            ttft_stats.count, 5,
            "TTFT should only count streaming requests"
        );
        assert_eq!(ttft_stats.min, 50);
        assert_eq!(ttft_stats.max, 250);
    }

    #[tokio::test]
    async fn test_concurrent_recording() {
        let config = TelemetryConfig::default();
        let tracker = Arc::new(TelemetryTracker::new(config));
        let provider_id = ProviderId::from("openai");

        let mut handles = vec![];

        // Spawn 100 tasks, each records 10 observations
        for task_id in 0..100 {
            let tracker_clone = tracker.clone();
            let provider_id_clone = provider_id.clone();
            let handle = tokio::spawn(async move {
                for i in 0..10 {
                    tracker_clone.record_observation(
                        provider_id_clone.clone(),
                        (task_id * 10 + i) as u64,
                        None,
                        ObservationStatus::Success,
                    );
                }
            });
            handles.push(handle);
        }

        // Wait for all tasks
        for handle in handles {
            handle.await.expect("Task should complete");
        }

        // Should have 1000 observations (or max_observations limit)
        let count = tracker.observation_count(&provider_id);
        assert_eq!(
            count, 1000,
            "Should have 1000 observations from concurrent writes"
        );
    }
}
