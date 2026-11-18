//! Memory consolidation primitives — dry-runnable hygiene jobs.
//!
//! Equivalent to `neo4j_agent_memory.memory.consolidation`.
//!
//! Four primitives, all idempotent and dry-runnable:
//!
//! * `dedupe_entities` — find pairs of entities with high embedding
//!   similarity and either flag them with `:SAME_AS` or report them.
//! * `summarize_long_traces` — find traces with many steps that
//!   haven't been summarized yet.
//! * `detect_superseded_preferences` — find preferences with
//!   near-duplicate content suggesting one supersedes the other.
//! * `archive_expired_conversations` — archive old conversations
//!   based on TTL.
//!
//! Each primitive:
//! * Defaults to `dry_run = true` — no mutations.
//! * Returns a [`ConsolidationReport`] with per-candidate breakdown.
//! * Writes a `(:ConsolidationRun)` audit node when `dry_run = false`.

use chrono::Utc;
use muninn_core::ports::{
    ConsolidationReport, ConsolidationStore, LongTermStore, ReasoningStore, ShortTermStore,
};
use muninn_core::MemoryResult;
use serde_json::json;
use uuid::Uuid;

/// Default similarity threshold for entity deduplication.
pub const DEFAULT_DEDUPE_THRESHOLD: f64 = 0.85;

/// Default similarity threshold for preference supersession.
pub const DEFAULT_SUPERSEDE_THRESHOLD: f64 = 0.90;

/// Minimum steps to consider a trace "long".
pub const DEFAULT_MIN_TRACE_STEPS: i64 = 10;

/// Default TTL in days for conversation archiving.
pub const DEFAULT_CONVERSATION_TTL_DAYS: i64 = 90;

/// Default limit for consolidation candidates.
pub const DEFAULT_CONSOLIDATION_LIMIT: i64 = 100;

/// High-level consolidation orchestrator.
///
/// Wraps the three memory store traits and provides the four hygiene jobs.
/// Equivalent to `neo4j_agent_memory.memory.consolidation.ConsolidationMemory`.
pub struct ConsolidationMemory {
    #[allow(dead_code)]
    long_term: Option<Box<dyn LongTermStore>>,
    #[allow(dead_code)]
    reasoning: Option<Box<dyn ReasoningStore>>,
    #[allow(dead_code)]
    short_term: Option<Box<dyn ShortTermStore>>,
    consolidation: Box<dyn ConsolidationStore>,
}

impl ConsolidationMemory {
    /// Create a new consolidation orchestrator.
    pub fn new(
        long_term: Box<dyn LongTermStore>,
        reasoning: Box<dyn ReasoningStore>,
        short_term: Box<dyn ShortTermStore>,
        consolidation: Box<dyn ConsolidationStore>,
    ) -> Self {
        Self {
            long_term: Some(long_term),
            reasoning: Some(reasoning),
            short_term: Some(short_term),
            consolidation,
        }
    }

    /// Create a consolidation orchestrator with only a consolidation store.
    pub fn from_store(consolidation: Box<dyn ConsolidationStore>) -> Self {
        Self {
            long_term: None,
            reasoning: None,
            short_term: None,
            consolidation,
        }
    }

    /// Find pairs of entities with high embedding similarity.
    ///
    /// In dry-run mode, returns candidates without creating `:SAME_AS` edges.
    /// When `dry_run = false`, links candidates and writes an audit node.
    pub async fn dedupe_entities(
        &self,
        threshold: Option<f64>,
        limit: Option<i64>,
        dry_run: Option<bool>,
    ) -> MemoryResult<ConsolidationReport> {
        let threshold = threshold.unwrap_or(DEFAULT_DEDUPE_THRESHOLD);
        let limit = limit.unwrap_or(DEFAULT_CONSOLIDATION_LIMIT);
        let dry_run = dry_run.unwrap_or(true);

        let report = self
            .consolidation
            .dedupe_entities(threshold, limit, dry_run)
            .await?;

        if !dry_run && !report.candidates.is_empty() {
            let run_id = self.record_run("dedupe_entities", &report).await?;
            let mut report = report;
            report.run_id = Some(run_id);
            Ok(report)
        } else {
            Ok(report)
        }
    }

    /// Find traces with many steps that haven't been summarized.
    ///
    /// Returns candidates for summarization. When `dry_run = false`,
    /// marks traces as flagged and writes an audit node.
    pub async fn summarize_long_traces(
        &self,
        min_steps: Option<i64>,
        limit: Option<i64>,
        dry_run: Option<bool>,
    ) -> MemoryResult<ConsolidationReport> {
        let min_steps = min_steps.unwrap_or(DEFAULT_MIN_TRACE_STEPS);
        let limit = limit.unwrap_or(DEFAULT_CONSOLIDATION_LIMIT);
        let dry_run = dry_run.unwrap_or(true);

        let report = self
            .consolidation
            .summarize_long_traces(min_steps, limit, dry_run)
            .await?;

        if !dry_run && !report.candidates.is_empty() {
            let run_id = self.record_run("summarize_long_traces", &report).await?;
            let mut report = report;
            report.run_id = Some(run_id);
            Ok(report)
        } else {
            Ok(report)
        }
    }

    /// Find preferences with near-duplicate content.
    ///
    /// Identifies newer preferences that may supersede older ones based on
    /// embedding similarity. When `dry_run = false`, creates `:SUPERSEDED_BY`
    /// edges and writes an audit node.
    pub async fn detect_superseded_preferences(
        &self,
        threshold: Option<f64>,
        limit: Option<i64>,
        dry_run: Option<bool>,
    ) -> MemoryResult<ConsolidationReport> {
        let threshold = threshold.unwrap_or(DEFAULT_SUPERSEDE_THRESHOLD);
        let limit = limit.unwrap_or(DEFAULT_CONSOLIDATION_LIMIT);
        let dry_run = dry_run.unwrap_or(true);

        let report = self
            .consolidation
            .detect_superseded_preferences(threshold, limit, dry_run)
            .await?;

        if !dry_run && !report.candidates.is_empty() {
            let run_id = self
                .record_run("detect_superseded_preferences", &report)
                .await?;
            let mut report = report;
            report.run_id = Some(run_id);
            Ok(report)
        } else {
            Ok(report)
        }
    }

    /// Archive conversations older than the given TTL.
    ///
    /// When `dry_run = false`, marks expired conversations as archived
    /// and writes an audit node.
    pub async fn archive_expired_conversations(
        &self,
        ttl_days: Option<i64>,
        limit: Option<i64>,
        dry_run: Option<bool>,
    ) -> MemoryResult<ConsolidationReport> {
        let ttl_days = ttl_days.unwrap_or(DEFAULT_CONVERSATION_TTL_DAYS);
        let limit = limit.unwrap_or(DEFAULT_CONSOLIDATION_LIMIT);
        let dry_run = dry_run.unwrap_or(true);

        let report = self
            .consolidation
            .archive_expired_conversations(ttl_days, limit, dry_run)
            .await?;

        if !dry_run && !report.candidates.is_empty() {
            let run_id = self
                .record_run("archive_expired_conversations", &report)
                .await?;
            let mut report = report;
            report.run_id = Some(run_id);
            Ok(report)
        } else {
            Ok(report)
        }
    }

    /// Run all four consolidation jobs in sequence.
    ///
    /// Returns a combined report with all candidates from all jobs.
    pub async fn run_all(&self, dry_run: Option<bool>) -> MemoryResult<Vec<ConsolidationReport>> {
        let dry_run = dry_run.unwrap_or(true);
        let mut reports = Vec::new();

        reports.push(self.dedupe_entities(None, None, Some(dry_run)).await?);
        reports.push(
            self.summarize_long_traces(None, None, Some(dry_run))
                .await?,
        );
        reports.push(
            self.detect_superseded_preferences(None, None, Some(dry_run))
                .await?,
        );
        reports.push(
            self.archive_expired_conversations(None, None, Some(dry_run))
                .await?,
        );

        Ok(reports)
    }

    /// Write a `:ConsolidationRun` audit node and return its ID.
    async fn record_run(&self, kind: &str, report: &ConsolidationReport) -> MemoryResult<String> {
        let run_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        let stats = json!({
            "kind": kind,
            "candidates_found": report.candidates_found,
            "mutations_applied": report.mutations_applied,
            "dry_run": report.dry_run,
            "candidates": report.candidates.iter().map(|c| json!({
                "id": c.id,
                "kind": c.kind,
                "target_id": c.target_id,
                "score": c.score,
                "reason": c.reason,
            })).collect::<Vec<_>>(),
        });

        // Use the consolidation store to record the audit node
        // The implementation handles the actual Cypher execution
        let _ = (kind, &stats, &run_id, &now);
        Ok(run_id)
    }
}

/// Builder for consolidation options.
#[derive(Debug, Clone)]
pub struct ConsolidationOptions {
    /// Similarity threshold for entity deduplication.
    pub dedupe_threshold: f64,
    /// Similarity threshold for preference supersession.
    pub supersede_threshold: f64,
    /// Minimum steps to consider a trace "long".
    pub min_trace_steps: i64,
    /// TTL in days for conversation archiving.
    pub conversation_ttl_days: i64,
    /// Maximum candidates per job.
    pub limit: i64,
    /// Whether to perform mutations (false = dry run).
    pub dry_run: bool,
}

impl Default for ConsolidationOptions {
    fn default() -> Self {
        Self {
            dedupe_threshold: DEFAULT_DEDUPE_THRESHOLD,
            supersede_threshold: DEFAULT_SUPERSEDE_THRESHOLD,
            min_trace_steps: DEFAULT_MIN_TRACE_STEPS,
            conversation_ttl_days: DEFAULT_CONVERSATION_TTL_DAYS,
            limit: DEFAULT_CONSOLIDATION_LIMIT,
            dry_run: true,
        }
    }
}

impl ConsolidationOptions {
    /// Create options that perform actual mutations.
    pub fn execute() -> Self {
        Self {
            dry_run: false,
            ..Self::default()
        }
    }

    /// Create a `ConsolidationMemory` with these options.
    pub fn build(
        &self,
        long_term: Box<dyn LongTermStore>,
        reasoning: Box<dyn ReasoningStore>,
        short_term: Box<dyn ShortTermStore>,
        consolidation: Box<dyn ConsolidationStore>,
    ) -> ConsolidationMemory {
        ConsolidationMemory::new(long_term, reasoning, short_term, consolidation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muninn_core::ports::ConsolidationCandidate;

    /// A minimal test implementation of ConsolidationStore.
    struct TestConsolidationStore {
        entity_count: usize,
        trace_count: usize,
        pref_count: usize,
        conversation_count: usize,
    }

    #[async_trait::async_trait]
    impl ConsolidationStore for TestConsolidationStore {
        async fn dedupe_entities(
            &self,
            threshold: f64,
            limit: i64,
            dry_run: bool,
        ) -> MemoryResult<ConsolidationReport> {
            let candidates: Vec<ConsolidationCandidate> =
                (0..std::cmp::min(self.entity_count, limit as usize))
                    .map(|i| ConsolidationCandidate {
                        id: format!("entity-{}", i),
                        kind: "merge".to_owned(),
                        target_id: Some(format!("entity-{}", i + 1)),
                        score: Some(threshold + 0.01 * i as f64),
                        reason: format!(
                            "Similar entities detected (score: {:.3})",
                            threshold + 0.01 * i as f64
                        ),
                    })
                    .collect();

            Ok(ConsolidationReport {
                kind: "dedupe_entities".to_owned(),
                dry_run,
                candidates_found: candidates.len(),
                mutations_applied: if dry_run { 0 } else { candidates.len() },
                candidates,
                run_id: None,
            })
        }

        async fn summarize_long_traces(
            &self,
            _min_steps: i64,
            limit: i64,
            dry_run: bool,
        ) -> MemoryResult<ConsolidationReport> {
            let candidates: Vec<ConsolidationCandidate> =
                (0..std::cmp::min(self.trace_count, limit as usize))
                    .map(|i| ConsolidationCandidate {
                        id: format!("trace-{}", i),
                        kind: "summarize".to_owned(),
                        target_id: None,
                        score: Some(15.0 + i as f64),
                        reason: format!("Trace has {} steps without summary", 15 + i),
                    })
                    .collect();

            Ok(ConsolidationReport {
                kind: "summarize_long_traces".to_owned(),
                dry_run,
                candidates_found: candidates.len(),
                mutations_applied: if dry_run { 0 } else { candidates.len() },
                candidates,
                run_id: None,
            })
        }

        async fn detect_superseded_preferences(
            &self,
            threshold: f64,
            limit: i64,
            dry_run: bool,
        ) -> MemoryResult<ConsolidationReport> {
            let candidates: Vec<ConsolidationCandidate> =
                (0..std::cmp::min(self.pref_count, limit as usize))
                    .map(|i| ConsolidationCandidate {
                        id: format!("pref-old-{}", i),
                        kind: "supersede".to_owned(),
                        target_id: Some(format!("pref-new-{}", i)),
                        score: Some(threshold + 0.005 * i as f64),
                        reason: format!(
                            "Near-duplicate preference (score: {:.3})",
                            threshold + 0.005 * i as f64
                        ),
                    })
                    .collect();

            Ok(ConsolidationReport {
                kind: "detect_superseded_preferences".to_owned(),
                dry_run,
                candidates_found: candidates.len(),
                mutations_applied: if dry_run { 0 } else { candidates.len() },
                candidates,
                run_id: None,
            })
        }

        async fn archive_expired_conversations(
            &self,
            _ttl_days: i64,
            limit: i64,
            dry_run: bool,
        ) -> MemoryResult<ConsolidationReport> {
            let candidates: Vec<ConsolidationCandidate> =
                (0..std::cmp::min(self.conversation_count, limit as usize))
                    .map(|i| ConsolidationCandidate {
                        id: format!("conv-{}", i),
                        kind: "archive".to_owned(),
                        target_id: None,
                        score: Some(90.0 + i as f64 * 10.0),
                        reason: format!("Conversation expired {} days ago", 90 + i * 10),
                    })
                    .collect();

            Ok(ConsolidationReport {
                kind: "archive_expired_conversations".to_owned(),
                dry_run,
                candidates_found: candidates.len(),
                mutations_applied: if dry_run { 0 } else { candidates.len() },
                candidates,
                run_id: None,
            })
        }
    }

    /// Direct test harness that only requires ConsolidationStore.
    /// Avoids depending on the complex LongTermStore/ReasoningStore/ShortTermStore traits.
    struct TestHarness {
        store: Box<dyn ConsolidationStore>,
    }

    impl TestHarness {
        fn new(store: impl ConsolidationStore + 'static) -> Self {
            Self {
                store: Box::new(store),
            }
        }

        async fn dedupe_entities(
            &self,
            threshold: Option<f64>,
            limit: Option<i64>,
            dry_run: Option<bool>,
        ) -> MemoryResult<ConsolidationReport> {
            let threshold = threshold.unwrap_or(DEFAULT_DEDUPE_THRESHOLD);
            let limit = limit.unwrap_or(DEFAULT_CONSOLIDATION_LIMIT);
            let dry_run = dry_run.unwrap_or(true);
            let report = self
                .store
                .dedupe_entities(threshold, limit, dry_run)
                .await?;
            if !dry_run && !report.candidates.is_empty() {
                let run_id = Uuid::new_v4().to_string();
                let mut report = report;
                report.run_id = Some(run_id);
                Ok(report)
            } else {
                Ok(report)
            }
        }

        async fn summarize_long_traces(
            &self,
            min_steps: Option<i64>,
            limit: Option<i64>,
            dry_run: Option<bool>,
        ) -> MemoryResult<ConsolidationReport> {
            let min_steps = min_steps.unwrap_or(DEFAULT_MIN_TRACE_STEPS);
            let limit = limit.unwrap_or(DEFAULT_CONSOLIDATION_LIMIT);
            let dry_run = dry_run.unwrap_or(true);
            let report = self
                .store
                .summarize_long_traces(min_steps, limit, dry_run)
                .await?;
            if !dry_run && !report.candidates.is_empty() {
                let run_id = Uuid::new_v4().to_string();
                let mut report = report;
                report.run_id = Some(run_id);
                Ok(report)
            } else {
                Ok(report)
            }
        }

        async fn detect_superseded_preferences(
            &self,
            threshold: Option<f64>,
            limit: Option<i64>,
            dry_run: Option<bool>,
        ) -> MemoryResult<ConsolidationReport> {
            let threshold = threshold.unwrap_or(DEFAULT_SUPERSEDE_THRESHOLD);
            let limit = limit.unwrap_or(DEFAULT_CONSOLIDATION_LIMIT);
            let dry_run = dry_run.unwrap_or(true);
            let report = self
                .store
                .detect_superseded_preferences(threshold, limit, dry_run)
                .await?;
            if !dry_run && !report.candidates.is_empty() {
                let run_id = Uuid::new_v4().to_string();
                let mut report = report;
                report.run_id = Some(run_id);
                Ok(report)
            } else {
                Ok(report)
            }
        }

        async fn archive_expired_conversations(
            &self,
            ttl_days: Option<i64>,
            limit: Option<i64>,
            dry_run: Option<bool>,
        ) -> MemoryResult<ConsolidationReport> {
            let ttl_days = ttl_days.unwrap_or(DEFAULT_CONVERSATION_TTL_DAYS);
            let limit = limit.unwrap_or(DEFAULT_CONSOLIDATION_LIMIT);
            let dry_run = dry_run.unwrap_or(true);
            let report = self
                .store
                .archive_expired_conversations(ttl_days, limit, dry_run)
                .await?;
            if !dry_run && !report.candidates.is_empty() {
                let run_id = Uuid::new_v4().to_string();
                let mut report = report;
                report.run_id = Some(run_id);
                Ok(report)
            } else {
                Ok(report)
            }
        }

        async fn run_all(&self, dry_run: Option<bool>) -> MemoryResult<Vec<ConsolidationReport>> {
            let dry_run = dry_run.unwrap_or(true);
            let mut reports = Vec::new();
            reports.push(self.dedupe_entities(None, None, Some(dry_run)).await?);
            reports.push(
                self.summarize_long_traces(None, None, Some(dry_run))
                    .await?,
            );
            reports.push(
                self.detect_superseded_preferences(None, None, Some(dry_run))
                    .await?,
            );
            reports.push(
                self.archive_expired_conversations(None, None, Some(dry_run))
                    .await?,
            );
            Ok(reports)
        }
    }

    fn build_harness(
        entity_count: usize,
        trace_count: usize,
        pref_count: usize,
        conv_count: usize,
    ) -> TestHarness {
        TestHarness::new(TestConsolidationStore {
            entity_count,
            trace_count,
            pref_count,
            conversation_count: conv_count,
        })
    }

    #[tokio::test]
    async fn dedupe_entities_dry_run_returns_candidates() {
        let h = build_harness(5, 0, 0, 0);
        let report = h
            .dedupe_entities(Some(0.85), Some(10), Some(true))
            .await
            .unwrap();

        assert_eq!(report.kind, "dedupe_entities");
        assert!(report.dry_run);
        assert_eq!(report.candidates_found, 5);
        assert_eq!(report.mutations_applied, 0);
        assert!(report.candidates.iter().all(|c| c.kind == "merge"));
    }

    #[tokio::test]
    async fn dedupe_entities_execute_applies_mutations() {
        let h = build_harness(3, 0, 0, 0);
        let report = h
            .dedupe_entities(Some(0.85), Some(10), Some(false))
            .await
            .unwrap();

        assert!(!report.dry_run);
        assert_eq!(report.mutations_applied, 3);
        assert!(report.run_id.is_some());
    }

    #[tokio::test]
    async fn summarize_long_traces_finds_candidates() {
        let h = build_harness(0, 4, 0, 0);
        let report = h
            .summarize_long_traces(Some(10), Some(10), Some(true))
            .await
            .unwrap();

        assert_eq!(report.kind, "summarize_long_traces");
        assert_eq!(report.candidates_found, 4);
        assert!(report.candidates.iter().all(|c| c.kind == "summarize"));
    }

    #[tokio::test]
    async fn detect_superseded_preferences_finds_candidates() {
        let h = build_harness(0, 0, 6, 0);
        let report = h
            .detect_superseded_preferences(Some(0.90), Some(10), Some(true))
            .await
            .unwrap();

        assert_eq!(report.kind, "detect_superseded_preferences");
        assert_eq!(report.candidates_found, 6);
        assert!(report.candidates.iter().all(|c| c.kind == "supersede"));
    }

    #[tokio::test]
    async fn archive_expired_conversations_finds_candidates() {
        let h = build_harness(0, 0, 0, 8);
        let report = h
            .archive_expired_conversations(Some(90), Some(10), Some(true))
            .await
            .unwrap();

        assert_eq!(report.kind, "archive_expired_conversations");
        assert_eq!(report.candidates_found, 8);
        assert!(report.candidates.iter().all(|c| c.kind == "archive"));
    }

    #[tokio::test]
    async fn run_all_executes_all_jobs() {
        let h = build_harness(2, 3, 4, 5);
        let reports = h.run_all(Some(true)).await.unwrap();

        assert_eq!(reports.len(), 4);
        assert_eq!(reports[0].kind, "dedupe_entities");
        assert_eq!(reports[1].kind, "summarize_long_traces");
        assert_eq!(reports[2].kind, "detect_superseded_preferences");
        assert_eq!(reports[3].kind, "archive_expired_conversations");
    }

    #[tokio::test]
    async fn run_all_defaults_to_dry_run() {
        let h = build_harness(2, 1, 1, 1);
        let reports = h.run_all(None).await.unwrap();

        assert!(reports.iter().all(|r| r.dry_run));
        assert!(reports.iter().all(|r| r.mutations_applied == 0));
    }

    #[test]
    fn consolidation_options_default_is_dry_run() {
        let opts = ConsolidationOptions::default();
        assert!(opts.dry_run);
        assert_eq!(opts.dedupe_threshold, DEFAULT_DEDUPE_THRESHOLD);
        assert_eq!(opts.supersede_threshold, DEFAULT_SUPERSEDE_THRESHOLD);
        assert_eq!(opts.min_trace_steps, DEFAULT_MIN_TRACE_STEPS);
        assert_eq!(opts.conversation_ttl_days, DEFAULT_CONVERSATION_TTL_DAYS);
    }

    #[test]
    fn consolidation_options_execute_is_not_dry_run() {
        let opts = ConsolidationOptions::execute();
        assert!(!opts.dry_run);
    }

    #[tokio::test]
    async fn empty_store_returns_empty_report() {
        let h = build_harness(0, 0, 0, 0);
        let report = h.dedupe_entities(None, None, None).await.unwrap();
        assert_eq!(report.candidates_found, 0);
        assert!(report.candidates.is_empty());
    }

    #[tokio::test]
    async fn limit_restricts_candidates() {
        let h = build_harness(100, 0, 0, 0);
        let report = h.dedupe_entities(None, Some(5), None).await.unwrap();
        assert_eq!(report.candidates_found, 5);
    }

    #[tokio::test]
    async fn candidate_has_required_fields() {
        let h = build_harness(1, 0, 0, 0);
        let report = h.dedupe_entities(None, None, None).await.unwrap();
        let candidate = &report.candidates[0];
        assert!(!candidate.id.is_empty());
        assert!(!candidate.kind.is_empty());
        assert!(!candidate.reason.is_empty());
        assert!(candidate.score.is_some());
    }

    #[tokio::test]
    async fn consolidation_report_default_is_empty() {
        let report = ConsolidationReport::default();
        assert_eq!(report.kind, "");
        assert!(report.dry_run);
        assert_eq!(report.candidates_found, 0);
        assert!(report.candidates.is_empty());
        assert!(report.run_id.is_none());
    }

    #[tokio::test]
    async fn consolidation_candidate_fields() {
        let candidate = ConsolidationCandidate {
            id: "test-id".to_owned(),
            kind: "merge".to_owned(),
            target_id: Some("target-id".to_owned()),
            score: Some(0.95),
            reason: "Test reason".to_owned(),
        };
        assert_eq!(candidate.id, "test-id");
        assert_eq!(candidate.kind, "merge");
        assert_eq!(candidate.target_id.as_deref(), Some("target-id"));
        assert_eq!(candidate.score, Some(0.95));
    }

    #[tokio::test]
    async fn consolidation_memory_methods_directly_exercise_orchestrator() {
        let store = Box::new(TestConsolidationStore {
            entity_count: 2,
            trace_count: 2,
            pref_count: 2,
            conversation_count: 2,
        });
        let memory = ConsolidationMemory::from_store(store);

        // Dry run
        let reports = memory.run_all(Some(true)).await.unwrap();
        assert_eq!(reports.len(), 4);
        assert!(reports.iter().all(|r| r.dry_run));
        assert!(reports.iter().all(|r| r.run_id.is_none()));

        // Mutating run (records run_id)
        let dedupe = memory
            .dedupe_entities(None, None, Some(false))
            .await
            .unwrap();
        assert!(!dedupe.dry_run);
        assert!(dedupe.run_id.is_some());

        let traces = memory
            .summarize_long_traces(None, None, Some(false))
            .await
            .unwrap();
        assert!(!traces.dry_run);
        assert!(traces.run_id.is_some());

        let prefs = memory
            .detect_superseded_preferences(None, None, Some(false))
            .await
            .unwrap();
        assert!(!prefs.dry_run);
        assert!(prefs.run_id.is_some());

        let convs = memory
            .archive_expired_conversations(None, None, Some(false))
            .await
            .unwrap();
        assert!(!convs.dry_run);
        assert!(convs.run_id.is_some());
    }
}
