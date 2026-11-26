//! Reported token usage contracts, per-request accumulation, and turn aggregation.
//!
//! Reported usage reflects what inference transports reported to Huginn; it is not a
//! provider invoice or billing reconciliation.

use crate::{Event, TurnId};
use serde::{Deserialize, Serialize};

/// Inference identity that produced a usage report.
///
/// Deliberately excludes endpoints, credentials, references, and capabilities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageAttribution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    pub model: String,
}

impl UsageAttribution {
    /// Human-readable `provider/model` or `model` label for display.
    pub fn label(&self) -> String {
        match &self.provider {
            Some(provider) if !provider.is_empty() => format!("{provider}/{}", self.model),
            _ => self.model.clone(),
        }
    }
}

/// Semantics of a usage delta within a single inference request stream.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageSemantics {
    /// Snapshot of cumulative token counts for the current inference request so far
    /// (for example, OpenAI-compatible `ChatCompletionChunk.usage`).
    #[default]
    Cumulative,
    /// Incremental token counts to add to prior deltas of the same request.
    Incremental,
}

/// Completeness of reported usage across one request or an entire turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageCompleteness {
    /// Every accounted request reported both input and output token counts.
    Complete,
    /// At least one token count was reported, but at least one request or field was missing.
    Partial,
    /// One or more requests occurred, but neither input nor output tokens were reported.
    Unknown,
}

/// Token counts reported for a single inference request.
///
/// `None` means the field was omitted or unparseable; `Some(0)` means zero was explicitly
/// reported.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportedUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

impl ReportedUsage {
    pub const fn completeness(&self) -> UsageCompleteness {
        match (self.input_tokens, self.output_tokens) {
            (Some(_), Some(_)) => UsageCompleteness::Complete,
            (Some(_), None) | (None, Some(_)) => UsageCompleteness::Partial,
            (None, None) => UsageCompleteness::Unknown,
        }
    }

    /// Known tokens contributed to local budget enforcement.
    pub const fn known_total(&self) -> u64 {
        let input = match self.input_tokens {
            Some(v) => v,
            None => 0,
        };
        let output = match self.output_tokens {
            Some(v) => v,
            None => 0,
        };
        input.saturating_add(output)
    }
}

/// Accumulates `ModelDelta::Usage` events within a single inference request stream.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RequestUsageAccumulator {
    observed: bool,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

impl RequestUsageAccumulator {
    pub fn observe(
        &mut self,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        semantics: UsageSemantics,
    ) {
        self.observed = true;
        match semantics {
            UsageSemantics::Cumulative => {
                if let Some(value) = input_tokens {
                    self.input_tokens = Some(value);
                }
                if let Some(value) = output_tokens {
                    self.output_tokens = Some(value);
                }
            }
            UsageSemantics::Incremental => {
                if let Some(value) = input_tokens {
                    self.input_tokens = Some(self.input_tokens.unwrap_or(0).saturating_add(value));
                }
                if let Some(value) = output_tokens {
                    self.output_tokens =
                        Some(self.output_tokens.unwrap_or(0).saturating_add(value));
                }
            }
        }
    }

    pub const fn observed(&self) -> bool {
        self.observed
    }

    pub const fn reported(&self) -> ReportedUsage {
        ReportedUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        }
    }
}

/// Aggregated reported usage across all inference requests of a single turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnUsage {
    pub turn: TurnId,
    /// Sum of reported input tokens across requests in this turn, or `None` if no request
    /// reported input tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Sum of reported output tokens across requests in this turn, or `None` if no request
    /// reported output tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Number of inference requests accounted for in this turn.
    pub requests: u32,
    /// Number of requests in this turn that omitted `input_tokens`.
    pub missing_input_requests: u32,
    /// Number of requests in this turn that omitted `output_tokens`.
    pub missing_output_requests: u32,
    /// Inference identity attributed to this turn's requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<UsageAttribution>,
}

impl TurnUsage {
    pub const fn new(turn: TurnId) -> Self {
        Self {
            turn,
            input_tokens: None,
            output_tokens: None,
            requests: 0,
            missing_input_requests: 0,
            missing_output_requests: 0,
            attribution: None,
        }
    }

    pub fn record(
        &mut self,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        attribution: Option<UsageAttribution>,
    ) {
        self.requests = self.requests.saturating_add(1);
        match input_tokens {
            Some(value) => {
                self.input_tokens = Some(self.input_tokens.unwrap_or(0).saturating_add(value));
            }
            None => {
                self.missing_input_requests = self.missing_input_requests.saturating_add(1);
            }
        }
        match output_tokens {
            Some(value) => {
                self.output_tokens = Some(self.output_tokens.unwrap_or(0).saturating_add(value));
            }
            None => {
                self.missing_output_requests = self.missing_output_requests.saturating_add(1);
            }
        }
        if self.attribution.is_none() {
            self.attribution = attribution;
        }
    }

    pub const fn completeness(&self) -> UsageCompleteness {
        if self.input_tokens.is_none() && self.output_tokens.is_none() {
            UsageCompleteness::Unknown
        } else if self.requests > 0
            && self.missing_input_requests == 0
            && self.missing_output_requests == 0
        {
            UsageCompleteness::Complete
        } else {
            UsageCompleteness::Partial
        }
    }

    /// Reconstructs the latest turn's aggregated usage from persisted session events.
    ///
    /// Returns `None` if the latest turn (or session) has no recorded `Event::Usage`.
    pub fn from_session_events(events: &[Event]) -> Option<Self> {
        let mut active_turn: Option<TurnId> = None;
        let mut latest: Option<Self> = None;
        for event in events {
            match event {
                Event::TurnStarted { turn, .. } => {
                    active_turn = Some(*turn);
                    latest = None;
                }
                Event::Usage {
                    turn,
                    input_tokens,
                    output_tokens,
                    attribution,
                    ..
                } => {
                    let effective_turn = if *turn != TurnId::UNKNOWN {
                        *turn
                    } else {
                        active_turn.unwrap_or(TurnId::UNKNOWN)
                    };
                    let reuse = latest.as_ref().is_some_and(|existing| {
                        existing.turn == effective_turn
                            && (effective_turn != TurnId::UNKNOWN || active_turn.is_some())
                    });
                    let aggregate = if reuse {
                        latest.as_mut().expect("checked above")
                    } else {
                        latest.insert(Self::new(effective_turn))
                    };
                    aggregate.record(*input_tokens, *output_tokens, attribution.clone());
                }
                Event::TurnFinished { .. } | Event::Error { .. } => {
                    active_turn = None;
                }
                _ => {}
            }
        }
        latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StepId;

    fn attr(provider: Option<&str>, model: &str) -> UsageAttribution {
        UsageAttribution {
            provider: provider.map(str::to_owned),
            connection: Some("conn-1".into()),
            model: model.into(),
        }
    }

    #[test]
    fn request_accumulator_handles_cumulative_snapshots_without_double_counting() {
        let mut acc = RequestUsageAccumulator::default();
        assert!(!acc.observed());
        acc.observe(Some(100), Some(5), UsageSemantics::Cumulative);
        acc.observe(Some(100), Some(12), UsageSemantics::Cumulative);
        assert!(acc.observed());
        let reported = acc.reported();
        assert_eq!(reported.input_tokens, Some(100));
        assert_eq!(reported.output_tokens, Some(12));
        assert_eq!(reported.completeness(), UsageCompleteness::Complete);
        assert_eq!(reported.known_total(), 112);
    }

    #[test]
    fn request_accumulator_handles_incremental_deltas_by_summing() {
        let mut acc = RequestUsageAccumulator::default();
        acc.observe(Some(100), Some(5), UsageSemantics::Incremental);
        acc.observe(None, Some(7), UsageSemantics::Incremental);
        let reported = acc.reported();
        assert_eq!(reported.input_tokens, Some(100));
        assert_eq!(reported.output_tokens, Some(12));
        assert_eq!(reported.completeness(), UsageCompleteness::Complete);
    }

    #[test]
    fn explicit_zero_is_distinguishable_from_missing_and_partial() {
        let mut zero = TurnUsage::new(TurnId(1));
        zero.record(Some(0), Some(0), Some(attr(Some("openai"), "model-a")));
        assert_eq!(zero.input_tokens, Some(0));
        assert_eq!(zero.output_tokens, Some(0));
        assert_eq!(zero.completeness(), UsageCompleteness::Complete);

        let mut unknown = TurnUsage::new(TurnId(2));
        unknown.record(None, None, Some(attr(Some("openai"), "model-a")));
        assert_eq!(unknown.input_tokens, None);
        assert_eq!(unknown.output_tokens, None);
        assert_eq!(unknown.completeness(), UsageCompleteness::Unknown);

        let mut partial_zero = TurnUsage::new(TurnId(3));
        partial_zero.record(Some(0), None, Some(attr(Some("openai"), "model-a")));
        assert_eq!(partial_zero.input_tokens, Some(0));
        assert_eq!(partial_zero.output_tokens, None);
        assert_eq!(partial_zero.completeness(), UsageCompleteness::Partial);
    }

    #[test]
    fn turn_usage_marks_known_subtotal_plus_missing_request_as_partial() {
        let mut turn = TurnUsage::new(TurnId(1));
        turn.record(Some(100), Some(40), Some(attr(Some("openai"), "model-a")));
        turn.record(None, None, Some(attr(Some("openai"), "model-a")));
        assert_eq!(turn.input_tokens, Some(100));
        assert_eq!(turn.output_tokens, Some(40));
        assert_eq!(turn.requests, 2);
        assert_eq!(turn.missing_input_requests, 1);
        assert_eq!(turn.missing_output_requests, 1);
        assert_eq!(turn.completeness(), UsageCompleteness::Partial);
    }

    #[test]
    fn from_session_events_reconstructs_latest_turn_and_preserves_attribution() {
        let events = vec![
            Event::TurnStarted {
                session_id: "s1".into(),
                turn: TurnId(1),
            },
            Event::Usage {
                turn: TurnId(1),
                step: StepId(1),
                input_tokens: Some(10),
                output_tokens: Some(5),
                attribution: Some(attr(Some("openai"), "old-model")),
            },
            Event::TurnFinished { turn: TurnId(1) },
            Event::TurnStarted {
                session_id: "s1".into(),
                turn: TurnId(2),
            },
            Event::Usage {
                turn: TurnId(2),
                step: StepId(1),
                input_tokens: Some(30),
                output_tokens: Some(10),
                attribution: Some(attr(Some("ollama"), "pinned-a")),
            },
            Event::Usage {
                turn: TurnId(2),
                step: StepId(2),
                input_tokens: Some(20),
                output_tokens: None,
                attribution: Some(attr(Some("ollama"), "pinned-a")),
            },
        ];

        let recovered = TurnUsage::from_session_events(&events).expect("latest turn usage");
        assert_eq!(recovered.turn, TurnId(2));
        assert_eq!(recovered.input_tokens, Some(50));
        assert_eq!(recovered.output_tokens, Some(10));
        assert_eq!(recovered.requests, 2);
        assert_eq!(recovered.missing_input_requests, 0);
        assert_eq!(recovered.missing_output_requests, 1);
        assert_eq!(recovered.completeness(), UsageCompleteness::Partial);
        assert_eq!(
            recovered.attribution.as_ref().map(UsageAttribution::label),
            Some("ollama/pinned-a".into())
        );
    }

    #[test]
    fn legacy_usage_event_deserializes_backward_compatibly() {
        let legacy = r#"{"type":"usage","step":1,"input_tokens":10,"output_tokens":20}"#;
        let event: Event = serde_json::from_str(legacy).expect("deserialize legacy usage");
        match event {
            Event::Usage {
                turn,
                step,
                input_tokens,
                output_tokens,
                attribution,
            } => {
                assert_eq!(turn, TurnId::UNKNOWN);
                assert_eq!(step, StepId(1));
                assert_eq!(input_tokens, Some(10));
                assert_eq!(output_tokens, Some(20));
                assert_eq!(attribution, None);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
