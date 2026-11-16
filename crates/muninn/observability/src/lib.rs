//! Muninn observability: tracing, metrics, and spans.
//!
//! Equivalent to `neo4j_agent_memory.observability`.
//!
//! # Modules
//!
//! - [`base`] — `Tracer`, `Span`, `NoOpTracer`, global tracer singleton
//! - [`metrics`] — Counters, gauges, histograms, and `MemoryMetrics`
//!
//! # Usage
//!
//! ```rust,no_run
//! use muninn_observability::{get_tracer, Span, SpanValue};
//!
//! // Tracing
//! let tracer = get_tracer();
//! let span = tracer.start_span("operation");
//! span.set_attribute("key", SpanValue::String("value".into()));
//! span.end();
//! ```

pub mod base;
pub mod metrics;

pub use base::{
    get_tracer, init_tracing_subscriber, is_opentelemetry_available, is_opik_available,
    set_global_tracer, NoOpSpan, NoOpTracer, Span, SpanException, SpanRecord, SpanStatus,
    SpanValue, Tracer, TracingProvider,
};
pub use metrics::{
    get_metrics, set_global_metrics, Counter, Gauge, Histogram, HistogramStats, MemoryMetrics,
    MetricPoint, Metrics,
};
