#![cfg(feature = "experimental-wasm")]

use huginn_core::{
    kernel::Supervisor, AgentError, CancellationToken, Result, ToolCall, ToolContext, ToolRegistry,
};
use huginn_runtime::{
    tools::{Registry, RegistryPlugin},
    wasm::WasmOutlinePlugin,
};
use serde_json::{json, Value};
use std::sync::Arc;

#[path = "support/wasm_adversary.rs"]
mod adversary;

fn declaration() -> Value {
    json!({
        "name":"f", "kind":"function", "container":null, "signature":"fn f()",
        "range":{"start_byte":0,"end_byte":8,"start_line":1,"start_column":1,
            "end_line":1,"end_column":9}
    })
}

fn outline(entries: Vec<Value>) -> String {
    json!({"complete":true,"declarations":entries,"diagnostics":[]}).to_string()
}

async fn execute(output: &str, source: &str, cancel_during_validation: bool) -> Result<String> {
    let (_fixture, component) = adversary::fixture("", output);
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(workspace.path().join("sample.rs"), source).unwrap();
    let plugin = WasmOutlinePlugin::load(
        &component,
        workspace.path(),
        Arc::new(huginn_guards::Guardrails::default()),
    )
    .unwrap();
    let activity = plugin.activity();
    let registry = Registry::default();
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(RegistryPlugin(registry.clone())))
        .unwrap();
    supervisor.register(Box::new(plugin)).unwrap();
    supervisor.start().await.unwrap();
    let ctx = ToolContext {
        selected_skills: vec![],
        workspace: workspace.path().to_path_buf(),
        cancellation: CancellationToken::new(),
    };
    let tool = registry.get("rust_file_outline").unwrap();
    let prepared = tool
        .prepare(
            &ToolCall {
                id: "validation".into(),
                name: "rust_file_outline".into(),
                arguments: json!({"path":"sample.rs"}),
            },
            &ctx,
        )
        .await
        .unwrap();
    let observer = cancel_during_validation.then(|| {
        let cancel = ctx.cancellation.clone();
        let activity = activity.clone();
        tokio::spawn(async move {
            while activity.snapshot_imports() == 0 {
                tokio::task::yield_now().await;
            }
            cancel.cancel();
        })
    });
    let result = tool.execute(prepared, &ctx).await;
    if let Some(observer) = observer {
        observer.await.unwrap();
    }
    assert_eq!(activity.active_invocations(), 0);
    supervisor.shutdown().await;
    result
}

#[tokio::test]
async fn empty_or_blank_declaration_names_are_rejected_in_every_entry() {
    for name in ["", " \t\n"] {
        let mut invalid = declaration();
        invalid["name"] = json!(name);
        assert!(
            execute(&outline(vec![declaration(), invalid]), "fn f(){}", false)
                .await
                .is_err(),
            "accepted name {name:?}"
        );
    }
}

#[tokio::test]
async fn empty_declaration_signature_is_rejected() {
    let mut invalid = declaration();
    invalid["signature"] = json!("");
    assert!(
        execute(&outline(vec![declaration(), invalid]), "fn f(){}", false)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn unsupported_declaration_kind_is_rejected() {
    let mut invalid = declaration();
    invalid["kind"] = json!("unsupported");
    assert!(
        execute(&outline(vec![declaration(), invalid]), "fn f(){}", false)
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn large_source_duplicate_ranges_observe_cancellation_during_host_validation() {
    let prefix = 1_048_568;
    let source = " ".repeat(prefix) + "fn f(){}";
    let mut entry = declaration();
    entry["range"]["start_byte"] = json!(prefix);
    entry["range"]["end_byte"] = json!(prefix + 8);
    entry["range"]["start_column"] = json!(prefix + 1);
    entry["range"]["end_column"] = json!(prefix + 9);
    let output = outline(vec![entry; 200]);
    assert!(output.len() <= huginn_runtime::wasm::MAX_OUTPUT_BYTES);
    assert!(matches!(
        execute(&output, &source, true).await,
        Err(AgentError::Cancelled)
    ));
    assert!(execute(&output, &source, false).await.is_ok());
}

#[tokio::test]
async fn byte_ranges_and_unicode_columns_are_checked_exactly() {
    let source = "// é\nfn café(){}";
    let mut entry = declaration();
    entry["name"] = json!("café");
    entry["signature"] = json!("fn café()");
    entry["range"] = json!({"start_byte":6,"end_byte":18,"start_line":2,
        "start_column":1,"end_line":2,"end_column":12});
    assert!(execute(&outline(vec![entry.clone()]), source, false)
        .await
        .is_ok());
    for (field, incorrect) in [
        ("start_byte", 5),
        ("start_byte", 13),
        ("end_byte", 17),
        ("end_byte", 13),
        ("start_line", 1),
        ("start_column", 2),
        ("end_line", 1),
        ("end_column", 13),
    ] {
        let mut invalid = entry.clone();
        invalid["range"][field] = json!(incorrect);
        assert!(
            execute(&outline(vec![invalid]), source, false)
                .await
                .is_err(),
            "accepted {field}"
        );
    }
}

#[tokio::test]
async fn supported_kinds_remain_structurally_valid_without_host_semantic_parsing() {
    let entries = [
        "function", "struct", "enum", "module", "trait", "impl", "method", "type", "const",
        "static", "union",
    ]
    .into_iter()
    .map(|kind| {
        let mut entry = declaration();
        entry["kind"] = json!(kind);
        entry
    })
    .collect();
    assert!(execute(&outline(entries), "fn f(){}", false).await.is_ok());
}

#[tokio::test]
async fn leading_unicode_whitespace_and_reordered_overlapping_ranges_keep_exact_positions() {
    let source = "\u{2003}\n fn f(){}";
    let mut first = declaration();
    first["range"] = json!({"start_byte":5,"end_byte":13,"start_line":2,
        "start_column":2,"end_line":2,"end_column":10});
    let mut overlapping = first.clone();
    overlapping["range"]["start_byte"] = json!(0);
    overlapping["range"]["start_line"] = json!(1);
    overlapping["range"]["start_column"] = json!(1);
    assert!(execute(
        &outline(vec![first.clone(), overlapping, first]),
        source,
        false
    )
    .await
    .is_ok());
    let mut blank = declaration();
    blank["range"] = json!({"start_byte":0,"end_byte":3,"start_line":1,
        "start_column":1,"end_line":1,"end_column":2});
    blank["signature"] = json!("\u{2003}");
    assert!(execute(&outline(vec![blank]), source, false).await.is_err());
}
