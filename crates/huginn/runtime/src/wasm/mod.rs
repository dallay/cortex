//! Experimental embedded components. No ambient WASI capabilities are linked.
mod engine;
mod snapshot;
mod tool;
use engine::Runtime;
use huginn_core::{
    kernel::{Manifest, Plugin, PluginContext},
    tools::{tool_registration_id, ToolRegistrationService},
    Result,
};
use std::{path::Path, sync::Arc};

pub const MAX_COMPONENT_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_SOURCE_BYTES: usize = 1_048_576;
pub const MAX_OUTPUT_BYTES: usize = 49_152;
pub const MAX_READ_BYTES: u32 = 65_536;
pub const MEMORY_BYTES: usize = 64 * 1024 * 1024;
pub const FUEL: u64 = 100_000_000;
pub const YIELD_FUEL: u64 = 10_000;
pub const DEADLINE_SECS: u64 = 5;

/// Payload-free counters useful for diagnosing cancellation and cleanup.
#[derive(Clone, Default)]
pub struct WasmActivity(Arc<Activity>);
#[derive(Default)]
struct Activity {
    active: std::sync::atomic::AtomicUsize,
    imports: std::sync::atomic::AtomicUsize,
}
impl WasmActivity {
    pub fn active_invocations(&self) -> usize {
        self.0.active.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn snapshot_imports(&self) -> usize {
        self.0.imports.load(std::sync::atomic::Ordering::Acquire)
    }
}

pub struct WasmOutlinePlugin {
    runtime: Arc<Runtime>,
    guards: Arc<huginn_guards::Guardrails>,
    workspace: Arc<snapshot::Workspace>,
}
impl WasmOutlinePlugin {
    pub fn activity(&self) -> WasmActivity {
        self.runtime.activity.clone()
    }
    pub fn load(
        component_path: &Path,
        workspace: &Path,
        guards: Arc<huginn_guards::Guardrails>,
    ) -> Result<Self> {
        // Pin the operator-selected workspace before loading untrusted code.
        let workspace = Arc::new(snapshot::Workspace::pin(workspace)?);
        Ok(Self {
            runtime: Arc::new(Runtime::load(component_path)?),
            guards,
            workspace,
        })
    }
}
#[async_trait::async_trait]
impl Plugin for WasmOutlinePlugin {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "wasm.rust-file-outline".into(),
            provides: vec![],
            requires: vec![tool_registration_id()],
        }
    }
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let registration = ctx.resolve::<ToolRegistrationService>(&tool_registration_id())?;
        registration.register(
            ctx,
            Arc::new(tool::OutlineTool {
                runtime: self.runtime.clone(),
                guards: self.guards.clone(),
                workspace: self.workspace.clone(),
            }),
        )
    }
}
