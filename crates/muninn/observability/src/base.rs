//! Base observability interfaces and utilities.
//!
//! Provides abstract interfaces for tracing that can be implemented
//! by different providers (OpenTelemetry, tracing-subscriber, etc.).
//!
//! Equivalent to `neo4j_agent_memory.observability.base`.
//!
//! # Usage
//!
//! ```rust,no_run
//! use muninn_observability::{Tracer, Span, SpanValue, get_tracer};
//!
//! let tracer = get_tracer();
//! let span = tracer.start_span("extract_entities");
//! span.set_attribute("text_length", SpanValue::Integer(42));
//! span.end();
//! ```

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Supported tracing provider backends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TracingProvider {
    /// No-op tracer — traces are collected but not exported.
    #[default]
    None,
    /// OpenTelemetry-based tracing.
    OpenTelemetry,
    /// Opik-based tracing (LLM-focused).
    Opik,
}

impl std::fmt::Display for TracingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::OpenTelemetry => write!(f, "opentelemetry"),
            Self::Opik => write!(f, "opik"),
        }
    }
}

impl std::str::FromStr for TracingProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" | "" => Ok(Self::None),
            "opentelemetry" | "otel" => Ok(Self::OpenTelemetry),
            "opik" => Ok(Self::Opik),
            _ => Err(format!("unknown tracing provider: {s}")),
        }
    }
}

/// Attribute value types supported by spans.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SpanValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    StringList(Vec<String>),
}

impl From<&str> for SpanValue {
    fn from(s: &str) -> Self {
        Self::String(s.to_owned())
    }
}

impl From<String> for SpanValue {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<i64> for SpanValue {
    fn from(n: i64) -> Self {
        Self::Integer(n)
    }
}

impl From<f64> for SpanValue {
    fn from(n: f64) -> Self {
        Self::Float(n)
    }
}

impl From<bool> for SpanValue {
    fn from(b: bool) -> Self {
        Self::Boolean(b)
    }
}

/// Span status codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum SpanStatus {
    Unset,
    Ok,
    Error,
}

/// A single trace span with attributes and timing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanRecord {
    /// Unique span identifier.
    pub span_id: String,
    /// Parent span ID, if any.
    pub parent_id: Option<String>,
    /// Span name/operation.
    pub name: String,
    /// Start time (Unix nanoseconds).
    pub start_time: u64,
    /// End time (Unix nanoseconds), or 0 if still active.
    pub end_time: u64,
    /// Span status.
    pub status: SpanStatus,
    /// Status description.
    pub status_description: Option<String>,
    /// Key-value attributes.
    pub attributes: HashMap<String, SpanValue>,
    /// Recorded exceptions.
    pub exceptions: Vec<SpanException>,
}

/// An exception recorded on a span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpanException {
    /// Exception type/name.
    pub r#type: String,
    /// Exception message.
    pub message: String,
    /// When the exception was recorded.
    pub timestamp: u64,
}

/// Core span interface — wraps any backend span.
#[async_trait::async_trait]
pub trait Span: Send + Sync {
    /// Set an attribute on the span.
    fn set_attribute(&self, key: &str, value: SpanValue);

    /// Set the span status.
    fn set_status(&self, status: SpanStatus, description: Option<&str>);

    /// Record an exception on the span.
    fn record_exception(&self, exception_type: &str, message: &str);

    /// End the span.
    fn end(&self);

    /// Get the underlying span record (for inspection).
    fn record(&self) -> SpanRecord;
}

/// Core tracer interface — factory for spans.
#[async_trait::async_trait]
pub trait Tracer: Send + Sync {
    /// Start a new span.
    fn start_span(&self, name: &str) -> Arc<dyn Span>;

    /// Start a new span with attributes.
    fn start_span_with_attrs(
        &self,
        name: &str,
        attributes: HashMap<String, SpanValue>,
    ) -> Arc<dyn Span>;

    /// Start a child span of an existing span.
    fn start_child_span(&self, name: &str, parent: &dyn Span) -> Arc<dyn Span>;

    /// Get the service name.
    fn service_name(&self) -> &str;

    /// Get the provider type.
    fn provider(&self) -> TracingProvider;
}

/// No-op span that collects data in memory without exporting.
pub struct NoOpSpan {
    record: std::sync::Mutex<SpanRecord>,
}

impl NoOpSpan {
    fn new(name: &str, parent_id: Option<String>) -> Self {
        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
        Self {
            record: std::sync::Mutex::new(SpanRecord {
                span_id: Uuid::new_v4().to_string(),
                parent_id,
                name: name.to_owned(),
                start_time: now,
                end_time: 0,
                status: SpanStatus::Unset,
                status_description: None,
                attributes: HashMap::new(),
                exceptions: Vec::new(),
            }),
        }
    }
}

impl Span for NoOpSpan {
    fn set_attribute(&self, key: &str, value: SpanValue) {
        if let Ok(mut rec) = self.record.lock() {
            rec.attributes.insert(key.to_owned(), value);
        }
    }

    fn set_status(&self, status: SpanStatus, description: Option<&str>) {
        if let Ok(mut rec) = self.record.lock() {
            rec.status = status;
            rec.status_description = description.map(|s| s.to_owned());
        }
    }

    fn record_exception(&self, exception_type: &str, message: &str) {
        if let Ok(mut rec) = self.record.lock() {
            rec.exceptions.push(SpanException {
                r#type: exception_type.to_owned(),
                message: message.to_owned(),
                timestamp: Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64,
            });
        }
    }

    fn end(&self) {
        if let Ok(mut rec) = self.record.lock() {
            rec.end_time = Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
            if rec.status == SpanStatus::Unset {
                rec.status = SpanStatus::Ok;
            }
        }
    }

    fn record(&self) -> SpanRecord {
        self.record.lock().unwrap().clone()
    }
}

/// No-op tracer that collects spans in memory.
pub struct NoOpTracer {
    service_name: String,
    spans: std::sync::Mutex<Vec<Arc<dyn Span>>>,
}

impl Default for NoOpTracer {
    fn default() -> Self {
        Self::new("muninn")
    }
}

impl NoOpTracer {
    /// Create a new NoOpTracer with the given service name.
    pub fn new(service_name: &str) -> Self {
        Self {
            service_name: service_name.to_owned(),
            spans: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Get all collected spans.
    pub fn collected_spans(&self) -> Vec<SpanRecord> {
        let spans = self.spans.lock().unwrap();
        spans.iter().map(|s| s.record()).collect()
    }
}

impl Tracer for NoOpTracer {
    fn start_span(&self, name: &str) -> Arc<dyn Span> {
        let span = Arc::new(NoOpSpan::new(name, None));
        self.spans.lock().unwrap().push(span.clone());
        span
    }

    fn start_span_with_attrs(
        &self,
        name: &str,
        attributes: HashMap<String, SpanValue>,
    ) -> Arc<dyn Span> {
        let span = Arc::new(NoOpSpan::new(name, None));
        for (key, value) in attributes {
            span.set_attribute(&key, value);
        }
        self.spans.lock().unwrap().push(span.clone());
        span
    }

    fn start_child_span(&self, name: &str, parent: &dyn Span) -> Arc<dyn Span> {
        let parent_id = Some(parent.record().span_id);
        let span = Arc::new(NoOpSpan::new(name, parent_id));
        self.spans.lock().unwrap().push(span.clone());
        span
    }

    fn service_name(&self) -> &str {
        &self.service_name
    }

    fn provider(&self) -> TracingProvider {
        TracingProvider::None
    }
}

/// Global tracer singleton.
static GLOBAL_TRACER: once_cell::sync::Lazy<std::sync::Mutex<Option<Arc<dyn Tracer>>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

/// Set the global tracer instance.
pub fn set_global_tracer(tracer: Arc<dyn Tracer>) {
    *GLOBAL_TRACER.lock().unwrap() = Some(tracer);
}

/// Get the current global tracer instance.
///
/// Returns a NoOpTracer if none is configured.
pub fn get_tracer() -> Arc<dyn Tracer> {
    let guard = GLOBAL_TRACER.lock().unwrap();
    guard
        .clone()
        .unwrap_or_else(|| Arc::new(NoOpTracer::default()))
}

/// Check if OpenTelemetry is available (always true in Rust — tracing is always available).
pub fn is_opentelemetry_available() -> bool {
    true
}

/// Check if Opik is available (not implemented in Rust port).
pub fn is_opik_available() -> bool {
    false
}

/// Initialize the tracing subscriber with JSON formatting.
///
/// This sets up the `tracing` crate's global subscriber for structured logging.
pub fn init_tracing_subscriber(
    env_filter: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new(env_filter.unwrap_or("muninn=info,tracing=warn"))
            }),
        )
        .json()
        .try_init()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn tracing_provider_from_str() {
        assert_eq!(
            TracingProvider::from_str("none").unwrap(),
            TracingProvider::None
        );
        assert_eq!(
            TracingProvider::from_str("opentelemetry").unwrap(),
            TracingProvider::OpenTelemetry
        );
        assert_eq!(
            TracingProvider::from_str("opik").unwrap(),
            TracingProvider::Opik
        );
        assert!(TracingProvider::from_str("unknown").is_err());
    }

    #[test]
    fn tracing_provider_display() {
        assert_eq!(format!("{}", TracingProvider::None), "none");
        assert_eq!(
            format!("{}", TracingProvider::OpenTelemetry),
            "opentelemetry"
        );
        assert_eq!(format!("{}", TracingProvider::Opik), "opik");
    }

    #[test]
    fn noop_span_sets_attributes() {
        let span = NoOpSpan::new("test", None);
        span.set_attribute("key", SpanValue::String("value".into()));
        span.set_attribute("count", SpanValue::Integer(42));
        let rec = span.record();
        assert_eq!(
            rec.attributes.get("key"),
            Some(&SpanValue::String("value".into()))
        );
        assert_eq!(rec.attributes.get("count"), Some(&SpanValue::Integer(42)));
    }

    #[test]
    fn noop_span_sets_status() {
        let span = NoOpSpan::new("test", None);
        span.set_status(SpanStatus::Error, Some("something went wrong"));
        let rec = span.record();
        assert_eq!(rec.status, SpanStatus::Error);
        assert_eq!(
            rec.status_description,
            Some("something went wrong".to_owned())
        );
    }

    #[test]
    fn noop_span_records_exception() {
        let span = NoOpSpan::new("test", None);
        span.record_exception("ValueError", "invalid input");
        let rec = span.record();
        assert_eq!(rec.exceptions.len(), 1);
        assert_eq!(rec.exceptions[0].r#type, "ValueError");
    }

    #[test]
    fn noop_span_end_sets_end_time() {
        let span = NoOpSpan::new("test", None);
        span.end();
        let rec = span.record();
        assert!(rec.end_time > 0);
        assert_eq!(rec.status, SpanStatus::Ok);
    }

    #[test]
    fn noop_tracer_starts_span() {
        let tracer = NoOpTracer::new("test-service");
        let span = tracer.start_span("operation");
        span.end();
        let spans = tracer.collected_spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "operation");
    }

    #[test]
    fn noop_tracer_child_span() {
        let tracer = NoOpTracer::new("test-service");
        let parent = tracer.start_span("parent");
        let child = tracer.start_child_span("child", &*parent);
        parent.end();
        child.end();
        let spans = tracer.collected_spans();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[1].parent_id, Some(spans[0].span_id.clone()));
    }

    #[test]
    fn span_value_from_str() {
        let v: SpanValue = "hello".into();
        assert!(matches!(v, SpanValue::String(_)));
    }

    #[test]
    fn span_value_from_int() {
        let v: SpanValue = (42i64).into();
        assert!(matches!(v, SpanValue::Integer(42)));
    }

    #[test]
    fn get_tracer_returns_noop_by_default() {
        let tracer = get_tracer();
        assert_eq!(tracer.provider(), TracingProvider::None);
    }

    #[test]
    fn is_opentelemetry_available_always_true() {
        assert!(is_opentelemetry_available());
    }

    #[test]
    fn is_opik_available_false() {
        assert!(!is_opik_available());
    }

    #[test]
    fn span_value_from_owned_string() {
        let v: SpanValue = String::from("hello").into();
        assert_eq!(v, SpanValue::String("hello".to_owned()));
    }

    #[test]
    fn span_value_from_float() {
        let v: SpanValue = 1.5_f64.into();
        assert_eq!(v, SpanValue::Float(1.5));
    }

    #[test]
    fn span_value_from_bool() {
        let v: SpanValue = true.into();
        assert_eq!(v, SpanValue::Boolean(true));
    }

    #[test]
    fn span_value_from_string_list() {
        let v: SpanValue = SpanValue::StringList(vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(
            v,
            SpanValue::StringList(vec!["a".to_owned(), "b".to_owned()])
        );
    }

    #[test]
    fn noop_tracer_start_span_with_attrs() {
        let tracer = NoOpTracer::new("test-service");
        let mut attributes = HashMap::new();
        attributes.insert("key".to_owned(), SpanValue::String("value".to_owned()));
        attributes.insert("count".to_owned(), SpanValue::Integer(7));

        let span = tracer.start_span_with_attrs("operation", attributes);
        span.end();

        let spans = tracer.collected_spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].name, "operation");
        assert_eq!(
            spans[0].attributes.get("key"),
            Some(&SpanValue::String("value".to_owned()))
        );
        assert_eq!(
            spans[0].attributes.get("count"),
            Some(&SpanValue::Integer(7))
        );
        assert_eq!(spans[0].parent_id, None);
    }

    #[test]
    fn noop_tracer_reports_its_service_name() {
        let tracer = NoOpTracer::new("test-service");
        assert_eq!(tracer.service_name(), "test-service");
        assert_eq!(tracer.provider(), TracingProvider::None);
    }

    #[test]
    fn noop_tracer_default_service_name_is_muninn() {
        let tracer = NoOpTracer::default();
        assert_eq!(tracer.service_name(), "muninn");
    }

    #[test]
    fn set_global_tracer_replaces_the_default_tracer() {
        set_global_tracer(Arc::new(NoOpTracer::new("global-service")));
        let tracer = get_tracer();
        assert_eq!(tracer.service_name(), "global-service");
        // The global tracer keeps reporting the no-op provider.
        assert_eq!(tracer.provider(), TracingProvider::None);
    }

    #[test]
    fn init_tracing_subscriber_installs_a_global_subscriber() {
        init_tracing_subscriber(Some("muninn=debug"))
            .expect("the global tracing subscriber should install");
    }

    #[test]
    fn noop_span_end_keeps_an_explicit_status() {
        let span = NoOpSpan::new("test", None);
        span.set_status(SpanStatus::Error, Some("boom"));
        span.end();

        let rec = span.record();
        assert_eq!(rec.status, SpanStatus::Error);
        assert_eq!(rec.status_description, Some("boom".to_owned()));
        assert_ne!(rec.end_time, 0);
    }

    #[test]
    fn noop_span_ignores_updates_when_the_record_lock_is_poisoned() {
        let span = NoOpSpan::new("test", None);
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = span.record.lock().expect("the lock is not poisoned yet");
            panic!("poison the record lock");
        }));
        assert!(poisoned.is_err(), "the panic must poison the record lock");

        // A poisoned lock must not panic on the mutating span API.
        span.set_attribute("key", SpanValue::Integer(1));
        span.set_status(SpanStatus::Error, None);
        span.record_exception("Error", "boom");
        span.end();
    }
}
