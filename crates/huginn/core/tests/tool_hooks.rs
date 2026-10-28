#[path = "support/hooks.rs"]
mod support;
use async_trait::async_trait;
use huginn_core::{
    hooks::{tool_hooks_id, ToolHook, ToolHooks},
    kernel::{Manifest, Plugin, PluginContext, Supervisor},
    Result,
};
use std::sync::Arc;
use support::{compose, Author, HostHooks};

struct Probe;
#[async_trait]
impl ToolHook for Probe {}

#[tokio::test]
async fn hooks_publish_on_commit_and_are_revoked_with_their_plugin() {
    let hooks = Arc::new(ToolHooks::default());
    let (mut supervisor, result) = compose(
        hooks.clone(),
        vec![Author {
            id: "guard",
            hooks: vec![("guard.shell", Arc::new(Probe))],
            fail: false,
        }],
    )
    .await;
    result.unwrap();
    assert_eq!(hooks.ids(), vec!["guard.shell".to_string()]);
    supervisor.remove("guard").await.unwrap();
    assert!(hooks.ids().is_empty());
    supervisor.shutdown().await;
}

#[tokio::test]
async fn failed_activation_leaves_no_hook_behind() {
    let hooks = Arc::new(ToolHooks::default());
    let (mut supervisor, result) = compose(
        hooks.clone(),
        vec![Author {
            id: "guard",
            hooks: vec![("guard.shell", Arc::new(Probe))],
            fail: true,
        }],
    )
    .await;
    assert!(result.is_err());
    assert!(hooks.ids().is_empty());
    supervisor.shutdown().await;
}

#[tokio::test]
async fn registration_requires_the_declared_hooks_service_and_valid_ids() {
    let declared = Arc::new(ToolHooks::default());
    let foreign = Arc::new(ToolHooks::default());
    struct Mismatch(Arc<ToolHooks>, String, bool);
    #[async_trait]
    impl Plugin for Mismatch {
        fn manifest(&self) -> Manifest {
            Manifest {
                id: "mismatch".into(),
                provides: vec![],
                requires: if self.2 {
                    vec![tool_hooks_id()]
                } else {
                    vec![]
                },
            }
        }
        async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
            self.0.register(ctx, self.1.clone(), Arc::new(Probe))
        }
    }
    for (registry, id, dependency) in [
        (foreign.clone(), "valid.id".into(), true),
        (declared.clone(), "Bad Id".into(), true),
        (declared.clone(), String::new(), true),
        (declared.clone(), "x".repeat(129), true),
        (declared.clone(), "valid.id".into(), false),
    ] {
        let mut supervisor = Supervisor::default();
        supervisor
            .register(Box::new(HostHooks(declared.clone())))
            .unwrap();
        supervisor
            .register(Box::new(Mismatch(registry, id, dependency)))
            .unwrap();
        assert!(supervisor.start().await.is_err());
        supervisor.shutdown().await;
    }
    assert!(declared.ids().is_empty() && foreign.ids().is_empty());
}

#[tokio::test]
async fn collisions_roll_back_and_ids_are_sorted() {
    let hooks = Arc::new(ToolHooks::default());
    let (mut supervisor, result) = compose(
        hooks.clone(),
        vec![Author {
            id: "duplicate",
            hooks: vec![("same", Arc::new(Probe)), ("same", Arc::new(Probe))],
            fail: false,
        }],
    )
    .await;
    assert!(result.is_err());
    assert!(hooks.ids().is_empty());
    supervisor.shutdown().await;
    let (mut supervisor, result) = compose(
        hooks.clone(),
        vec![Author {
            id: "valid",
            hooks: vec![("z.last", Arc::new(Probe)), ("a.first", Arc::new(Probe))],
            fail: false,
        }],
    )
    .await;
    result.unwrap();
    assert_eq!(hooks.ids(), vec!["a.first", "z.last"]);
    supervisor.shutdown().await;
    assert!(hooks.ids().is_empty());
}
