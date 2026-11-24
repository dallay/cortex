//! Muninn application services: the Rust counterpart of upstream
//! `MemoryClient` and its `short_term`, `long_term` and `reasoning` tiers.

pub mod buffered;
pub mod client;
pub mod consolidation;
pub mod enrichment;
pub mod eval;
pub mod extraction;
pub mod local_operator;
#[cfg(test)]
mod local_operator_tests;
pub mod long_term;
pub mod reasoning;
pub mod reasoning_observer;
pub mod recorder;
pub mod resolution;
pub mod short_term;

pub use buffered::{
    BufferedWriter, BufferedWriterConfig, ErrorCallback, DEFAULT_CALLBACK_TIMEOUT,
    DEFAULT_MAX_PENDING, DEFAULT_MAX_RETAINED_ERRORS,
};
pub use client::{MemoryClient, MemoryOptions, MemoryPorts};
pub use consolidation::{
    ConsolidationMemory, ConsolidationOptions, DEFAULT_CONSOLIDATION_LIMIT,
    DEFAULT_CONVERSATION_TTL_DAYS, DEFAULT_DEDUPE_LIMIT, DEFAULT_DEDUPE_THRESHOLD,
    DEFAULT_MIN_TRACE_STEPS, DEFAULT_SUPERSEDE_THRESHOLD, DEFAULT_TRACE_LIMIT,
};
pub use eval::{
    bcubed_f1, recall_at_k, set_f1, DimensionScore, EvalMemory, EvalReport, EvalRequest,
    ExpectedEntity, ExpectedRelation, ExtractionCase, PreferenceCase, ResolutionCase,
    RetrievalCase,
};
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
