use muninn_observability::{get_tracer, set_global_tracer, NoOpTracer, TracingProvider};
use std::sync::Arc;

#[test]
fn set_global_tracer_replaces_the_default_tracer() {
    set_global_tracer(Arc::new(NoOpTracer::new("global-service")));
    let tracer = get_tracer();
    assert_eq!(tracer.service_name(), "global-service");
    assert_eq!(tracer.provider(), TracingProvider::None);
}
