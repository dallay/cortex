// Metrics — prometheus-style metrics via the `metrics` crate
//
// Usage (anywhere in code):
//   metrics::increment_counter!("rook_requests_total");
//   metrics::histogram::observe!("rook_request_duration_ms", 42.5);

/// Initialize metric descriptors at module load time.
/// Call once during app startup.
pub fn init_metrics() {
    // Descriptions are no-ops at runtime — they document metrics for exporters
    metrics::describe_counter!("rook_requests_total", "Total number of requests processed");
    metrics::describe_histogram!(
        "rook_request_duration_ms",
        "Request duration in milliseconds"
    );
    metrics::describe_counter!("rook_tokens_total", "Total tokens processed");
    metrics::describe_counter!("rook_provider_errors", "Total provider errors");
    metrics::describe_counter!("rook_cache_hits", "Cache hits");
    metrics::describe_counter!("rook_cache_misses", "Cache misses");
}
