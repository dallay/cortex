//! Application-owned tool-call callbacks, not authorization or content validation.

use async_trait::async_trait;
use muninn_core::model::{EntityRef, ToolCall};
use muninn_core::MemoryResult;
use uuid::Uuid;

use crate::ReasoningMemory;

/// An awaited observer. Register before sharing the memory service.
///
/// Errors (including writes requested inside a callback) are logged without
/// payloads and suppressed; subsequent observers still run, as in the oracle.
#[async_trait]
pub trait ToolCallObserver: Send + Sync {
    async fn on_tool_call_recorded(&self, context: &HookContext<'_>) -> MemoryResult<()>;
}

/// Intentional callback data and a narrow touched-edge capability.
///
/// The supplied call contains application-approved arguments/results/errors.
/// This context does not expose the store or grant authentication authority.
pub struct HookContext<'a> {
    pub(crate) memory: &'a ReasoningMemory,
    pub(crate) call: &'a ToolCall,
    pub(crate) step_id: Uuid,
}

impl HookContext<'_> {
    pub const fn tool_call(&self) -> &ToolCall {
        self.call
    }

    pub const fn step_id(&self) -> Uuid {
        self.step_id
    }

    pub async fn add_touched_edge(&self, entity: &EntityRef) -> MemoryResult<bool> {
        self.memory
            .record_touched_entity(self.step_id, entity)
            .await
    }
}
