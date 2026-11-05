//! Muninn application services: the Rust counterpart of upstream
//! `MemoryClient` and its `short_term`, `long_term` and `reasoning` tiers.

pub mod client;
pub mod long_term;
pub mod reasoning;
pub mod reasoning_observer;
pub mod recorder;
pub mod resolution;
pub mod short_term;

pub use client::{MemoryClient, MemoryOptions, MemoryPorts};
pub use long_term::{
    AddEntityOptions, AddFactOptions, AddPreferenceOptions, AddRelationshipOptions, LongTermMemory,
    SearchEntitiesOptions, SearchPreferencesOptions,
};
pub use reasoning::{
    ReasoningMemory, SimilarityOptions, StartTraceOptions, StepInput, ToolCallOptions,
};
pub use reasoning_observer::{HookContext, ToolCallObserver};
pub use recorder::{RecorderOptions, RecorderStep, StreamingTraceRecorder};
pub use short_term::{AddMessageOptions, SearchMessagesOptions, ShortTermMemory};
