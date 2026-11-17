//! Metrics collection for Muninn memory operations.
//!
//! Provides counters, gauges, and histograms for monitoring
//! memory operations, similar to `neo4j_agent_memory.core.metrics`.
//!
//! # Usage
//!
//! ```rust,no_run
//! use muninn_observability::metrics::Metrics;
//!
//! let metrics = Metrics::new();
//! // In async context:
//! // metrics.message_added().await;
//! // metrics.entity_created("PERSON".to_string()).await;
//! let snapshot = metrics.snapshot();
//! ```

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

/// A single metric data point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricPoint {
    /// Metric name.
    pub name: String,
    /// Metric value.
    pub value: f64,
    /// Timestamp (Unix nanoseconds).
    pub timestamp: u64,
    /// Labels/tags.
    pub labels: HashMap<String, String>,
}

/// Counter that only increases.
pub struct Counter {
    inner: AtomicI64,
}

impl Counter {
    fn new() -> Self {
        Self {
            inner: AtomicI64::new(0),
        }
    }

    /// Increment by 1.
    pub fn inc(&self) {
        self.inner.fetch_add(1, Ordering::Relaxed);
    }

    /// Increment by a value.
    pub fn add(&self, v: i64) {
        self.inner.fetch_add(v, Ordering::Relaxed);
    }

    /// Get the current value.
    pub fn get(&self) -> i64 {
        self.inner.load(Ordering::Relaxed)
    }
}

/// Gauge that can go up and down.
pub struct Gauge {
    inner: AtomicI64,
}

impl Gauge {
    fn new() -> Self {
        Self {
            inner: AtomicI64::new(0),
        }
    }

    /// Set the value.
    pub fn set(&self, v: i64) {
        self.inner.store(v, Ordering::Relaxed);
    }

    /// Increment by 1.
    pub fn inc(&self) {
        self.inner.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement by 1.
    pub fn dec(&self) {
        self.inner.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get the current value.
    pub fn get(&self) -> i64 {
        self.inner.load(Ordering::Relaxed)
    }
}

/// Histogram that tracks min, max, sum, count.
pub struct Histogram {
    min: AtomicU64,
    max: AtomicU64,
    sum: AtomicI64,
    count: AtomicI64,
}

impl Histogram {
    fn new() -> Self {
        Self {
            min: AtomicU64::new(u64::MAX),
            max: AtomicU64::new(0),
            sum: AtomicI64::new(0),
            count: AtomicI64::new(0),
        }
    }

    /// Record a value.
    pub fn record(&self, v: i64) {
        self.sum.fetch_add(v, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);

        // Update min
        loop {
            let current = self.min.load(Ordering::Relaxed);
            if v as u64 >= current {
                break;
            }
            match self.min.compare_exchange_weak(
                current,
                v as u64,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }

        // Update max
        loop {
            let current = self.max.load(Ordering::Relaxed);
            if v as u64 <= current {
                break;
            }
            match self.max.compare_exchange_weak(
                current,
                v as u64,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
    }

    /// Get the statistics.
    pub fn stats(&self) -> HistogramStats {
        let count = self.count.load(Ordering::Relaxed);
        let sum = self.sum.load(Ordering::Relaxed);
        let min = self.min.load(Ordering::Relaxed);
        let max = self.max.load(Ordering::Relaxed);
        HistogramStats {
            count,
            sum,
            min: if count > 0 { min as i64 } else { 0 },
            max: max as i64,
            avg: if count > 0 {
                sum as f64 / count as f64
            } else {
                0.0
            },
        }
    }
}

/// Histogram statistics snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistogramStats {
    pub count: i64,
    pub sum: i64,
    pub min: i64,
    pub max: i64,
    pub avg: f64,
}

/// Core memory metrics — equivalent to `neo4j_agent_memory.core.metrics`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMetrics {
    /// Total messages added.
    pub messages_added: i64,
    /// Total messages searched.
    pub messages_searched: i64,
    /// Total entities created.
    pub entities_created: i64,
    /// Total entities searched.
    pub entities_searched: i64,
    /// Total relationships created.
    pub relationships_created: i64,
    /// Total preferences stored.
    pub preferences_stored: i64,
    /// Total facts stored.
    pub facts_stored: i64,
    /// Total enrichment calls.
    pub enrichment_calls: i64,
    /// Total extraction calls.
    pub extraction_calls: i64,
    /// Total resolution calls.
    pub resolution_calls: i64,
    /// Total errors.
    pub errors: i64,
    /// Total sessions active.
    pub sessions_active: i64,
    /// Total traces created.
    pub traces_created: i64,
    /// Total reasoning steps.
    pub reasoning_steps: i64,
    /// Timestamp of the last update.
    pub last_updated: String,
}

impl Default for MemoryMetrics {
    fn default() -> Self {
        Self {
            messages_added: 0,
            messages_searched: 0,
            entities_created: 0,
            entities_searched: 0,
            relationships_created: 0,
            preferences_stored: 0,
            facts_stored: 0,
            enrichment_calls: 0,
            extraction_calls: 0,
            resolution_calls: 0,
            errors: 0,
            sessions_active: 0,
            traces_created: 0,
            reasoning_steps: 0,
            last_updated: Utc::now().to_rfc3339(),
        }
    }
}

/// Central metrics collector.
pub struct Metrics {
    messages_added: Counter,
    messages_searched: Counter,
    entities_created: Counter,
    entities_searched: Counter,
    relationships_created: Counter,
    preferences_stored: Counter,
    facts_stored: Counter,
    enrichment_calls: Counter,
    extraction_calls: Counter,
    resolution_calls: Counter,
    errors: Counter,
    sessions_active: Gauge,
    traces_created: Counter,
    reasoning_steps: Counter,
    operation_latency: Histogram,
    points: std::sync::Mutex<Vec<MetricPoint>>,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    /// Create a new metrics collector.
    pub fn new() -> Self {
        Self {
            messages_added: Counter::new(),
            messages_searched: Counter::new(),
            entities_created: Counter::new(),
            entities_searched: Counter::new(),
            relationships_created: Counter::new(),
            preferences_stored: Counter::new(),
            facts_stored: Counter::new(),
            enrichment_calls: Counter::new(),
            extraction_calls: Counter::new(),
            resolution_calls: Counter::new(),
            errors: Counter::new(),
            sessions_active: Gauge::new(),
            traces_created: Counter::new(),
            reasoning_steps: Counter::new(),
            operation_latency: Histogram::new(),
            points: std::sync::Mutex::new(Vec::new()),
        }
    }

    // — Short-term memory —

    /// Record a message added.
    pub async fn message_added(&self) {
        self.messages_added.inc();
        self.record_point("messages_added", 1.0, HashMap::new());
    }

    /// Record a message search.
    pub async fn message_searched(&self) {
        self.messages_searched.inc();
        self.record_point("messages_searched", 1.0, HashMap::new());
    }

    // — Long-term memory —

    /// Record an entity created.
    pub async fn entity_created(&self, label: String) {
        self.entities_created.inc();
        let mut labels = HashMap::new();
        labels.insert("label".to_string(), label);
        self.record_point("entities_created", 1.0, labels);
    }

    /// Record an entity search.
    pub async fn entity_searched(&self) {
        self.entities_searched.inc();
        self.record_point("entities_searched", 1.0, HashMap::new());
    }

    /// Record a relationship created.
    pub async fn relationship_created(&self, rel_type: String) {
        self.relationships_created.inc();
        let mut labels = HashMap::new();
        labels.insert("rel_type".to_string(), rel_type);
        self.record_point("relationships_created", 1.0, labels);
    }

    /// Record a preference stored.
    pub async fn preference_stored(&self, category: String) {
        self.preferences_stored.inc();
        let mut labels = HashMap::new();
        labels.insert("category".to_string(), category);
        self.record_point("preferences_stored", 1.0, labels);
    }

    /// Record a fact stored.
    pub async fn fact_stored(&self) {
        self.facts_stored.inc();
        self.record_point("facts_stored", 1.0, HashMap::new());
    }

    // — Enrichment —

    /// Record an enrichment call.
    pub async fn enrichment_call(&self, provider: String) {
        self.enrichment_calls.inc();
        let mut labels = HashMap::new();
        labels.insert("provider".to_string(), provider);
        self.record_point("enrichment_calls", 1.0, labels);
    }

    // — Extraction —

    /// Record an extraction call.
    pub async fn extraction_call(&self, extractor: String) {
        self.extraction_calls.inc();
        let mut labels = HashMap::new();
        labels.insert("extractor".to_string(), extractor);
        self.record_point("extraction_calls", 1.0, labels);
    }

    // — Resolution —

    /// Record a resolution call.
    pub async fn resolution_call(&self) {
        self.resolution_calls.inc();
        self.record_point("resolution_calls", 1.0, HashMap::new());
    }

    // — Errors —

    /// Record an error.
    pub async fn error(&self, error_type: String) {
        self.errors.inc();
        let mut labels = HashMap::new();
        labels.insert("error_type".to_string(), error_type);
        self.record_point("errors", 1.0, labels);
    }

    // — Sessions —

    /// Record a session started.
    pub async fn session_started(&self) {
        self.sessions_active.inc();
        self.record_point(
            "sessions_active",
            self.sessions_active.get() as f64,
            HashMap::new(),
        );
    }

    /// Record a session ended.
    pub async fn session_ended(&self) {
        self.sessions_active.dec();
        self.record_point(
            "sessions_active",
            self.sessions_active.get() as f64,
            HashMap::new(),
        );
    }

    // — Reasoning —

    /// Record a trace created.
    pub async fn trace_created(&self) {
        self.traces_created.inc();
        self.record_point("traces_created", 1.0, HashMap::new());
    }

    /// Record a reasoning step.
    pub async fn reasoning_step(&self) {
        self.reasoning_steps.inc();
        self.record_point("reasoning_steps", 1.0, HashMap::new());
    }

    // — Latency —

    /// Record an operation latency in milliseconds.
    pub async fn record_latency(&self, operation: &str, latency_ms: i64) {
        self.operation_latency.record(latency_ms);
        let mut labels = HashMap::new();
        labels.insert("operation".to_string(), operation.to_string());
        self.record_point("operation_latency_ms", latency_ms as f64, labels);
    }

    // — Snapshot —

    /// Get a snapshot of all memory metrics.
    pub fn snapshot(&self) -> MemoryMetrics {
        MemoryMetrics {
            messages_added: self.messages_added.get(),
            messages_searched: self.messages_searched.get(),
            entities_created: self.entities_created.get(),
            entities_searched: self.entities_searched.get(),
            relationships_created: self.relationships_created.get(),
            preferences_stored: self.preferences_stored.get(),
            facts_stored: self.facts_stored.get(),
            enrichment_calls: self.enrichment_calls.get(),
            extraction_calls: self.extraction_calls.get(),
            resolution_calls: self.resolution_calls.get(),
            errors: self.errors.get(),
            sessions_active: self.sessions_active.get(),
            traces_created: self.traces_created.get(),
            reasoning_steps: self.reasoning_steps.get(),
            last_updated: Utc::now().to_rfc3339(),
        }
    }

    /// Get the operation latency histogram stats.
    pub fn latency_stats(&self) -> HistogramStats {
        self.operation_latency.stats()
    }

    /// Get all recorded metric points.
    pub fn points(&self) -> Vec<MetricPoint> {
        self.points.lock().unwrap().clone()
    }

    fn record_point(&self, name: &str, value: f64, labels: HashMap<String, String>) {
        if let Ok(mut points) = self.points.lock() {
            points.push(MetricPoint {
                name: name.to_owned(),
                value,
                timestamp: Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64,
                labels,
            });
        }
    }
}

/// Global metrics singleton.
static GLOBAL_METRICS: once_cell::sync::Lazy<std::sync::Mutex<Metrics>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(Metrics::new()));

/// Set the global metrics instance.
pub fn set_global_metrics(metrics: Metrics) {
    *GLOBAL_METRICS.lock().unwrap() = metrics;
}

/// Get a reference to the global metrics instance.
pub fn get_metrics() -> std::sync::MutexGuard<'static, Metrics> {
    GLOBAL_METRICS.lock().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_increments() {
        let counter = Counter::new();
        counter.inc();
        counter.add(5);
        assert_eq!(counter.get(), 6);
    }

    #[test]
    fn gauge_goes_up_and_down() {
        let gauge = Gauge::new();
        gauge.inc();
        gauge.inc();
        gauge.dec();
        assert_eq!(gauge.get(), 1);
        gauge.set(10);
        assert_eq!(gauge.get(), 10);
    }

    #[test]
    fn histogram_records_values() {
        let hist = Histogram::new();
        hist.record(10);
        hist.record(20);
        hist.record(30);
        let stats = hist.stats();
        assert_eq!(stats.count, 3);
        assert_eq!(stats.sum, 60);
        assert_eq!(stats.min, 10);
        assert_eq!(stats.max, 30);
        assert!((stats.avg - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn histogram_empty_stats() {
        let hist = Histogram::new();
        let stats = hist.stats();
        assert_eq!(stats.count, 0);
        assert_eq!(stats.min, 0);
        assert_eq!(stats.max, 0);
        assert_eq!(stats.avg, 0.0);
    }

    #[tokio::test]
    async fn metrics_message_added() {
        let metrics = Metrics::new();
        metrics.message_added().await;
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.messages_added, 1);
    }

    #[tokio::test]
    async fn metrics_entity_created() {
        let metrics = Metrics::new();
        metrics.entity_created("PERSON".to_string()).await;
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.entities_created, 1);
    }

    #[tokio::test]
    async fn metrics_session_lifecycle() {
        let metrics = Metrics::new();
        metrics.session_started().await;
        assert_eq!(metrics.snapshot().sessions_active, 1);
        metrics.session_ended().await;
        assert_eq!(metrics.snapshot().sessions_active, 0);
    }

    #[tokio::test]
    async fn metrics_error() {
        let metrics = Metrics::new();
        metrics.error("ValueError".to_string()).await;
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.errors, 1);
    }

    #[tokio::test]
    async fn metrics_latency() {
        let metrics = Metrics::new();
        metrics.record_latency("search", 100).await;
        metrics.record_latency("search", 200).await;
        let stats = metrics.latency_stats();
        assert_eq!(stats.count, 2);
        assert_eq!(stats.min, 100);
        assert_eq!(stats.max, 200);
    }

    #[test]
    fn memory_metrics_default() {
        let metrics = MemoryMetrics::default();
        assert_eq!(metrics.messages_added, 0);
        assert!(!metrics.last_updated.is_empty());
    }

    #[tokio::test]
    async fn metrics_snapshot() {
        let metrics = Metrics::new();
        metrics.message_added().await;
        metrics.message_added().await;
        metrics.entity_created("ORG".to_string()).await;
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.messages_added, 2);
        assert_eq!(snapshot.entities_created, 1);
    }

    #[test]
    fn metric_point_serialization() {
        let point = MetricPoint {
            name: "test".to_string(),
            value: 1.0,
            timestamp: 12345,
            labels: HashMap::new(),
        };
        let json = serde_json::to_string(&point).unwrap();
        assert!(json.contains("test"));
    }
}
