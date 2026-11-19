use muninn_observability::{get_tracer, TracingProvider};

#[test]
fn get_tracer_returns_noop_by_default() {
    let tracer = get_tracer();
    assert_eq!(tracer.provider(), TracingProvider::None);
    assert_eq!(tracer.service_name(), "muninn");
}
