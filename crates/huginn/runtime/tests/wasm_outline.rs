#![cfg(feature = "experimental-wasm")]
use huginn_core::{kernel::Supervisor, CancellationToken, ToolCall, ToolContext, ToolRegistry};
use huginn_runtime::{
    tools::{Registry, RegistryPlugin},
    wasm::WasmOutlinePlugin,
};
use std::sync::Arc;
#[path = "support/wasm_component.rs"]
mod wasm_component;
use wasm_component::component;
#[path = "support/wasm_adversary.rs"]
mod adversary;

const EMPTY_OUTLINE: &str = r#"{"complete":true,"declarations":[],"diagnostics":[]}"#;

#[tokio::test]
async fn large_prefix_outline_completes_within_the_existing_guest_fuel_budget() {
    let root = tempfile::tempdir().unwrap();
    let (mut supervisor, registry, activity) = registered(&component(), root.path()).await;
    let ctx = context(root.path());
    let tool = registry.get("rust_file_outline").unwrap();
    for (separator, count) in [("\n", 1), ("\n", 100), ("", 100)] {
        let mut source = format!("/*{}*/{separator}", "x".repeat(900_000));
        for index in 0..count {
            source.push_str(&format!("pub fn item_{index}() {{}}\n"));
        }
        assert!(source.len() < huginn_runtime::wasm::MAX_SOURCE_BYTES);
        std::fs::write(root.path().join("sample.rs"), source).unwrap();
        let prepared = tool.prepare(&call("sample.rs"), &ctx).await.unwrap();
        let output = tool.execute(prepared, &ctx).await.unwrap_or_else(|error| {
            panic!("{count} declarations after a 900 KB prefix exceeded the unchanged guest budget: {error}")
        });
        assert!(output.len() <= huginn_runtime::wasm::MAX_OUTPUT_BYTES);
        let outline: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(outline["complete"], true);
        assert_eq!(outline["declarations"].as_array().unwrap().len(), count);
        assert_eq!(
            outline["declarations"][0]["range"]["start_line"],
            if separator.is_empty() { 1 } else { 2 }
        );
        assert_eq!(
            outline["declarations"][0]["range"]["start_column"],
            if separator.is_empty() { 900_005 } else { 1 }
        );
        assert_eq!(activity.active_invocations(), 0);
    }
    supervisor.shutdown().await;
}

#[cfg(unix)]
async fn replaced_workspace_remains_confined(replace_ancestor: bool) {
    let root = tempfile::tempdir().unwrap();
    let ancestor = root.path().join("parent");
    let workspace = ancestor.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(workspace.join("sample.rs"), "pub fn authorized() {}\n").unwrap();
    let (mut supervisor, registry, _) = registered(&component(), &workspace).await;
    let ctx = context(&workspace);
    let tool = registry.get("rust_file_outline").unwrap();
    let prepared = tool.prepare(&call("sample.rs"), &ctx).await.unwrap();
    let outside = tempfile::tempdir().unwrap();
    let replacement = if replace_ancestor {
        outside.path().join("workspace")
    } else {
        outside.path().to_path_buf()
    };
    std::fs::create_dir_all(&replacement).unwrap();
    std::fs::write(
        replacement.join("sample.rs"),
        "pub fn redirected_secret() {}\n",
    )
    .unwrap();
    let replaced = if replace_ancestor {
        &ancestor
    } else {
        &workspace
    };
    std::fs::rename(replaced, root.path().join("original")).unwrap();
    std::os::unix::fs::symlink(outside.path(), replaced).unwrap();
    if let Ok(output) = tool.execute(prepared, &ctx).await {
        let output: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(output["declarations"][0]["name"], "authorized");
    }
    supervisor.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn replacing_workspace_root_after_prepare_never_redirects_the_snapshot() {
    replaced_workspace_remains_confined(false).await;
}

#[cfg(unix)]
#[tokio::test]
async fn replacing_workspace_ancestor_after_prepare_never_redirects_the_snapshot() {
    replaced_workspace_remains_confined(true).await;
}

#[cfg(unix)]
#[test]
fn component_fifo_is_rejected_without_blocking_startup() {
    const CHILD: &str = "HUGINN_TEST_COMPONENT_FIFO";
    if let Some(path) = std::env::var_os(CHILD) {
        assert!(WasmOutlinePlugin::load(
            std::path::Path::new(&path),
            std::path::Path::new(&path).parent().unwrap(),
            Arc::new(huginn_guards::Guardrails::default()),
        )
        .is_err());
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let fifo = root.path().join("component.fifo");
    assert!(std::process::Command::new("/usr/bin/mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success());
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "component_fifo_is_rejected_without_blocking_startup",
            "--nocapture",
        ])
        .env(CHILD, &fifo)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("component FIFO open blocked startup");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

async fn registered(
    path: &std::path::Path,
    workspace: &std::path::Path,
) -> (Supervisor, Registry, huginn_runtime::wasm::WasmActivity) {
    let plugin = WasmOutlinePlugin::load(
        path,
        workspace,
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
    (supervisor, registry, activity)
}

fn context(path: &std::path::Path) -> ToolContext {
    ToolContext {
        selected_skills: vec![],
        workspace: path.to_path_buf(),
        cancellation: CancellationToken::new(),
    }
}
fn call(path: &str) -> ToolCall {
    ToolCall {
        id: "outline".into(),
        name: "rust_file_outline".into(),
        arguments: serde_json::json!({"path":path}),
    }
}

#[tokio::test]
async fn independently_built_component_registers_in_the_real_registry() {
    let workspace = tempfile::tempdir().unwrap();
    let registry = Registry::default();
    let mut supervisor = Supervisor::default();
    supervisor
        .register(Box::new(RegistryPlugin(registry.clone())))
        .unwrap();
    supervisor
        .register(Box::new(
            WasmOutlinePlugin::load(
                &component(),
                workspace.path(),
                Arc::new(huginn_guards::Guardrails::default()),
            )
            .unwrap(),
        ))
        .unwrap();
    supervisor.start().await.unwrap();
    let tool = registry.get("rust_file_outline").unwrap();
    std::fs::write(
        workspace.path().join("sample.rs"),
        "pub fn real_guest() {}\n",
    )
    .unwrap();
    let context = ToolContext {
        selected_skills: vec![],
        workspace: workspace.path().to_path_buf(),
        cancellation: CancellationToken::new(),
    };
    let call = ToolCall {
        id: "outline-1".into(),
        name: "rust_file_outline".into(),
        arguments: serde_json::json!({"path":"sample.rs"}),
    };
    let prepared = tool.prepare(&call, &context).await.unwrap();
    assert!(prepared.approval.is_none());
    let output = tool.execute(prepared, &context).await.unwrap();
    let output: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert_eq!(output["declarations"][0]["name"], "real_guest");
    supervisor.shutdown().await;
    assert!(registry.get("rust_file_outline").is_none());
}

#[tokio::test]
async fn non_cooperative_guest_is_interrupted_and_store_is_dropped() {
    let (_fixture, path) = adversary::fixture("(loop $spin br $spin)", EMPTY_OUTLINE);
    let root = tempfile::tempdir().unwrap();
    let (mut supervisor, registry, activity) = registered(&path, root.path()).await;
    std::fs::write(root.path().join("sample.rs"), "fn sample() {}").unwrap();
    let ctx = context(root.path());
    let cancel = ctx.cancellation.clone();
    let tool = registry.get("rust_file_outline").unwrap();
    let prepared = tool.prepare(&call("sample.rs"), &ctx).await.unwrap();
    let task = tokio::spawn(async move { tool.execute(prepared, &ctx).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while activity.snapshot_imports() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("guest must actually enter the host import before cancellation");
    assert_eq!(activity.active_invocations(), 1);
    cancel.cancel();
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(result, Err(huginn_core::AgentError::Cancelled)));
    assert_eq!(activity.active_invocations(), 0);
    supervisor.shutdown().await;
}

#[tokio::test]
async fn traps_fuel_memory_output_and_fabricated_handles_fail_without_leaks() {
    for (body, output) in [
        ("unreachable", EMPTY_OUTLINE.to_owned()),
        ("(loop $spin br $spin)", EMPTY_OUTLINE.to_owned()),
        ("i32.const 1024 memory.grow drop", EMPTY_OUTLINE.to_owned()),
        (
            "local.get $file i32.const 1 i32.add call $size drop",
            EMPTY_OUTLINE.to_owned(),
        ),
        ("", "x".repeat(49_153)),
        (
            "",
            r#"{"complete":false,"declarations":[],"diagnostics":[]}"#.to_owned(),
        ),
    ] {
        let (_fixture, path) = adversary::fixture(body, &output);
        let root = tempfile::tempdir().unwrap();
        let (mut supervisor, registry, activity) = registered(&path, root.path()).await;
        std::fs::write(root.path().join("sample.rs"), "fn sample() {}").unwrap();
        let ctx = context(root.path());
        let tool = registry.get("rust_file_outline").unwrap();
        let prepared = tool.prepare(&call("sample.rs"), &ctx).await.unwrap();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(7),
            tool.execute(prepared, &ctx),
        )
        .await
        .unwrap();
        assert!(result.is_err(), "adversary unexpectedly succeeded: {body}");
        assert_eq!(activity.active_invocations(), 0);
        supervisor.shutdown().await;
        assert!(tool.prepare(&call("sample.rs"), &ctx).await.is_err());
    }
}

#[tokio::test]
async fn denied_paths_and_symlink_swaps_never_reach_guest_imports() {
    let (_fixture, path) = adversary::fixture("", EMPTY_OUTLINE);
    let root = tempfile::tempdir().unwrap();
    let (mut supervisor, registry, activity) = registered(&path, root.path()).await;
    std::fs::write(root.path().join("sample.rs"), "fn sample() {}").unwrap();
    std::fs::write(root.path().join(".env"), "private marker").unwrap();
    let ctx = context(root.path());
    let tool = registry.get("rust_file_outline").unwrap();
    let other = tempfile::tempdir().unwrap();
    std::fs::write(other.path().join("sample.rs"), "private marker").unwrap();
    let other_ctx = context(other.path());
    assert!(tool.prepare(&call("sample.rs"), &other_ctx).await.is_err());
    let prepared = tool.prepare(&call("sample.rs"), &ctx).await.unwrap();
    assert!(tool.execute(prepared, &other_ctx).await.is_err());
    for path in ["../sample.rs", "/etc/passwd", ".env", "", "./sample.rs"] {
        assert!(
            tool.prepare(&call(path), &ctx).await.is_err(),
            "accepted {path}"
        );
    }
    for path in ["x".repeat(4097), "sample.rs\n".into()] {
        assert!(tool.prepare(&call(&path), &ctx).await.is_err());
    }
    let mut unknown = call("sample.rs");
    unknown.arguments["grant"] = serde_json::json!(true);
    assert!(tool.prepare(&unknown, &ctx).await.is_err());
    std::fs::write(root.path().join("large.rs"), vec![b' '; 1_048_577]).unwrap();
    let prepared = tool.prepare(&call("large.rs"), &ctx).await.unwrap();
    assert!(tool.execute(prepared, &ctx).await.is_err());
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.rs"), "private marker").unwrap();
        let prepared = tool.prepare(&call("sample.rs"), &ctx).await.unwrap();
        std::fs::remove_file(root.path().join("sample.rs")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.rs"),
            root.path().join("sample.rs"),
        )
        .unwrap();
        assert!(tool.execute(prepared, &ctx).await.is_err());
        std::os::unix::fs::symlink(outside.path(), root.path().join("alias")).unwrap();
        let result = tool
            .execute(
                huginn_core::PreparedAction {
                    approval: None,
                    payload: serde_json::json!({"path":"alias/secret.rs"}),
                },
                &ctx,
            )
            .await;
        assert!(result.is_err());
    }
    assert_eq!(activity.snapshot_imports(), 0);
    assert_eq!(activity.active_invocations(), 0);
    supervisor.shutdown().await;
}

#[test]
fn incompatible_component_is_rejected_before_registration() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("incompatible.wasm");
    std::fs::write(&path, wat::parse_str("(component)").unwrap()).unwrap();
    assert!(WasmOutlinePlugin::load(
        &path,
        root.path(),
        Arc::new(huginn_guards::Guardrails::default())
    )
    .is_err());
    std::fs::write(
        &path,
        wat::parse_str(r#"(component (import "wasi:cli/environment@0.2.0" (instance)))"#).unwrap(),
    )
    .unwrap();
    assert!(WasmOutlinePlugin::load(
        &path,
        root.path(),
        Arc::new(huginn_guards::Guardrails::default())
    )
    .is_err());
}

#[cfg(unix)]
#[test]
fn component_loader_rejects_a_symlink_instead_of_following_an_operator_alias() {
    let (_fixture, path) = adversary::fixture("", EMPTY_OUTLINE);
    let directory = tempfile::tempdir().unwrap();
    let alias = directory.path().join("alias.component.wasm");
    std::os::unix::fs::symlink(path, &alias).unwrap();
    assert!(WasmOutlinePlugin::load(
        &alias,
        directory.path(),
        Arc::new(huginn_guards::Guardrails::default())
    )
    .is_err());
}

#[tokio::test]
async fn unloading_active_generation_interrupts_guest_and_revokes_retained_handles() {
    let (_fixture, path) = adversary::fixture("(loop $spin br $spin)", EMPTY_OUTLINE);
    let root = tempfile::tempdir().unwrap();
    let (mut supervisor, registry, activity) = registered(&path, root.path()).await;
    std::fs::write(root.path().join("sample.rs"), "fn sample() {}").unwrap();
    let ctx = context(root.path());
    let retained = registry.get("rust_file_outline").unwrap();
    let tool = retained.clone();
    let task_ctx = ctx.clone();
    let prepared = tool.prepare(&call("sample.rs"), &ctx).await.unwrap();
    let task = tokio::spawn(async move { tool.execute(prepared, &task_ctx).await });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while activity.snapshot_imports() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    supervisor.remove("wasm.rust-file-outline").await.unwrap();
    assert!(task.await.unwrap().is_err());
    assert_eq!(activity.active_invocations(), 0);
    assert!(registry.get("rust_file_outline").is_none());
    assert!(retained.prepare(&call("sample.rs"), &ctx).await.is_err());
    supervisor.shutdown().await;
}

#[tokio::test]
async fn read_quota_violation_cannot_be_hidden_by_guest_success() {
    let (_fixture, path) = adversary::fixture(
        "local.get $file i32.const 0 i32.const 65537 i32.const 16 call $read",
        EMPTY_OUTLINE,
    );
    let root = tempfile::tempdir().unwrap();
    let (mut supervisor, registry, activity) = registered(&path, root.path()).await;
    std::fs::write(root.path().join("sample.rs"), "fn sample() {}").unwrap();
    let ctx = context(root.path());
    let tool = registry.get("rust_file_outline").unwrap();
    let prepared = tool.prepare(&call("sample.rs"), &ctx).await.unwrap();
    assert!(tool.execute(prepared, &ctx).await.is_err());
    assert_eq!(activity.active_invocations(), 0);
    supervisor.shutdown().await;
}
