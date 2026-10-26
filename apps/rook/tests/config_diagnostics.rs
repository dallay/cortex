use std::sync::{Arc, Mutex};

use rook::config::RookConfig;
use tracing::{field::Visit, span, Event, Metadata, Subscriber};

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

struct Fields(String);

impl Visit for Fields {
    fn record_debug(&mut self, _: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!("{value:?} "));
    }
}

impl Subscriber for Warnings {
    fn enabled(&self, _: &Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
        span::Id::from_u64(1)
    }
    fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
    fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
    fn event(&self, event: &Event<'_>) {
        if *event.metadata().level() == tracing::Level::WARN {
            let mut fields = Fields(String::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
    }
    fn enter(&self, _: &span::Id) {}
    fn exit(&self, _: &span::Id) {}
}

fn load_warnings(extra: &str) -> Vec<String> {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        format!(
            r#"
[server]
host = "127.0.0.1"
port = 0
[routing]
strategy = "priority"
[cache]
enabled = false
ttl_secs = 60
{extra}
"#
        ),
    )
    .unwrap();
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        RookConfig::load(file.path()).expect("legacy cache keys must remain loadable");
    });
    let result = warnings.0.lock().unwrap().clone();
    result
}

#[test]
fn explicit_token_cache_keys_warn_even_with_default_values() {
    for (setting, key) in [
        ("mode = \"never\"", "mode"),
        ("providers = []", "providers"),
    ] {
        let warnings = load_warnings(&format!("[cache.token_cache]\n{setting}"));
        assert!(
            warnings.iter().any(|message| {
                message.contains(&format!("cache.token_cache.{key}"))
                    && message.contains("unsupported")
            }),
            "missing unsupported-key warning for {key}: {warnings:?}"
        );
    }
}

#[test]
fn omitted_token_cache_keys_do_not_warn() {
    assert!(load_warnings("").is_empty());
    assert!(load_warnings("[cache.token_cache]").is_empty());
}
