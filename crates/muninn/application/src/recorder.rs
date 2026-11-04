//! Explicit application-driven recording. No provider stream subscription.
//!
//! Content must already be approved by the application; field names do not
//! validate, redact or authenticate it. Always await `finish` or `finish_failed`.
//! Drop, cancellation, panic and process failure cannot complete a trace.

use std::time::Instant;

use muninn_core::model::{Metadata, ReasoningStep, ReasoningTrace, ToolCall, TraceOutcome};
use muninn_core::{MemoryError, MemoryResult};
use uuid::Uuid;

use crate::{ReasoningMemory, StartTraceOptions, StepInput, ToolCallOptions};

/// Recorder embedding defaults match the pinned streaming recorder.
#[derive(Debug, Clone, Default)]
pub struct RecorderOptions {
    /// Includes the task embedding switch (true by default) and native scope/link options.
    pub trace: StartTraceOptions,
    /// Embed on step creation and request deferred embeddings on completion.
    pub generate_step_embeddings: bool,
}

/// Explicit, application-approved content only. Summary maps to the compatible
/// `thought` property; it is not a provider-private reasoning channel.
#[derive(Debug, Clone, Default)]
pub struct RecorderStep {
    pub summary: Option<String>,
    pub action: Option<String>,
    pub observation: Option<String>,
}

enum Outcome {
    Legacy { summary: String, success: bool },
    Structured(TraceOutcome),
}

/// One explicitly started trace with sequential, application-driven steps.
///
/// Completion consumes the recorder; failed completion remains an error and
/// may have partial writes, according to the existing memory completion helpers.
#[must_use = "await finish or finish_failed; dropping a recorder leaves the trace open"]
pub struct StreamingTraceRecorder<'a> {
    memory: &'a ReasoningMemory,
    trace_id: Uuid,
    step_id: Option<Uuid>,
    step_started: Option<Instant>,
    generate_step_embeddings: bool,
    outcome: Option<Outcome>,
}

impl<'a> StreamingTraceRecorder<'a> {
    pub async fn start(
        memory: &'a ReasoningMemory,
        session_id: &str,
        task: &str,
        options: RecorderOptions,
    ) -> MemoryResult<Self> {
        let trace = memory.start_trace(session_id, task, options.trace).await?;
        Ok(Self {
            memory,
            trace_id: trace.id,
            step_id: None,
            step_started: None,
            generate_step_embeddings: options.generate_step_embeddings,
            outcome: None,
        })
    }

    pub const fn trace_id(&self) -> Uuid {
        self.trace_id
    }

    pub const fn step_id(&self) -> Option<Uuid> {
        self.step_id
    }

    pub async fn start_step(&mut self, input: RecorderStep) -> MemoryResult<ReasoningStep> {
        let started = Instant::now();
        let step = self
            .memory
            .add_step(
                self.trace_id,
                StepInput {
                    thought: input.summary,
                    action: input.action,
                    observation: input.observation,
                    generate_embedding: self.generate_step_embeddings,
                    ..StepInput::default()
                },
            )
            .await?;
        self.step_id = Some(step.id);
        self.step_started = Some(started);
        Ok(step)
    }

    pub async fn record_tool_call(
        &mut self,
        tool_name: &str,
        arguments: Metadata,
        mut options: ToolCallOptions,
    ) -> MemoryResult<ToolCall> {
        let step_id = match self.step_id {
            Some(id) => id,
            None => {
                self.start_step(RecorderStep {
                    action: Some(format!("call:{tool_name}")),
                    ..RecorderStep::default()
                })
                .await?
                .id
            }
        };
        if options.duration_ms.is_none() {
            options.duration_ms = self
                .step_started
                .map(|start| i64::try_from(start.elapsed().as_millis()).unwrap_or(i64::MAX));
        }
        self.memory
            .record_tool_call(step_id, tool_name, arguments, options)
            .await
    }

    pub async fn add_observation(&self, observation: &str) -> MemoryResult<()> {
        let id = self
            .step_id
            .ok_or_else(|| MemoryError::invalid("No active step. Call start_step() first."))?;
        self.memory.set_step_observation(id, observation).await
    }

    pub fn set_outcome(&mut self, summary: impl Into<String>, success: bool) {
        self.outcome = Some(Outcome::Legacy {
            summary: summary.into(),
            success,
        });
    }

    pub fn set_structured_outcome(&mut self, outcome: TraceOutcome) -> MemoryResult<()> {
        outcome.validate()?;
        self.outcome = Some(Outcome::Structured(outcome));
        Ok(())
    }

    /// Normal exit defaults to success, even with no explicit outcome or steps.
    pub async fn finish(self) -> MemoryResult<ReasoningTrace> {
        match self.outcome {
            Some(Outcome::Structured(outcome)) => {
                self.memory
                    .complete_trace_with_outcome(
                        self.trace_id,
                        &outcome,
                        self.generate_step_embeddings,
                    )
                    .await
            }
            outcome => {
                let (summary, success) = match &outcome {
                    Some(Outcome::Legacy { summary, success }) => {
                        (Some(summary.as_str()), *success)
                    }
                    _ => (None, true),
                };
                self.memory
                    .complete_trace(
                        self.trace_id,
                        summary,
                        Some(success),
                        self.generate_step_embeddings,
                    )
                    .await
            }
        }
    }

    /// Explicit error-path exit overrides any outcome with failure. The caller
    /// supplies a safe summary, never an automatically formatted exception payload.
    /// Await this after a failed operation and handle completion errors separately.
    pub async fn finish_failed(
        mut self,
        safe_summary: impl Into<String>,
    ) -> MemoryResult<ReasoningTrace> {
        self.set_outcome(safe_summary, false);
        self.finish().await
    }
}
