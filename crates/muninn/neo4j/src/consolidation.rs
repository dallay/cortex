//! Neo4j-backed [`ConsolidationStore`] implementation.
//!
//! Persists consolidation mutations and `:ConsolidationRun` /
//! `:MemoryReadAudit` audit nodes against Neo4j using the canonical upstream
//! graph shape (`neo4j-labs/agent-memory @ 9d45af55`).

use std::collections::HashSet;

use async_trait::async_trait;
use muninn_core::ports::{
    ConsolidationCandidate, ConsolidationReport, ConsolidationStore, ReadAuditRecord,
};
use muninn_core::{MemoryError, MemoryResult};
use neo4rs::{BoltNull, BoltType, Row, Txn};
use serde_json::json;
use uuid::Uuid;

use crate::bolt::{column, opt_string, to_datetime};
use crate::client::{transaction_rows, Neo4jClient};
use crate::long_term::{require_preference_lock_constraint, PREFERENCE_WRITE_LOCK};
use crate::queries::consolidation as q;

/// Neo4j persistence adapter for memory consolidation hygiene jobs.
#[derive(Clone)]
pub struct Neo4jConsolidationStore {
    client: Neo4jClient,
}

impl Neo4jConsolidationStore {
    /// Construct a new consolidation store backed by `client`.
    pub const fn new(client: Neo4jClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ConsolidationStore for Neo4jConsolidationStore {
    async fn dedupe_entities(
        &self,
        threshold: f64,
        limit: i64,
        dry_run: bool,
    ) -> MemoryResult<ConsolidationReport> {
        validate_threshold(threshold)?;
        validate_limit(limit)?;

        let rows = self
            .client
            .execute(
                q::FIND_DUPLICATE_ENTITY_PAIRS,
                vec![("threshold", threshold.into()), ("limit", limit.into())],
            )
            .await?;

        let mut candidates = Vec::with_capacity(rows.len());
        for row in &rows {
            let a_id = required_string(row, "a_id")?;
            let a_name = required_string(row, "a_name")?;
            let b_id = required_string(row, "b_id")?;
            let b_name = required_string(row, "b_name")?;
            let score = required_f64(row, "score")?;

            candidates.push(ConsolidationCandidate {
                id: a_id,
                kind: "merge".to_owned(),
                target_id: Some(b_id),
                score: Some(score),
                reason: format!("'{a_name}' ~= '{b_name}' (score {score:.3})"),
            });
        }

        let (mutations_applied, run_id) = if dry_run || candidates.is_empty() {
            (0, None)
        } else {
            let mut txn = self.client.start_txn().await?;
            let result = async {
                let mut mutations_applied = 0;
                for candidate in &candidates {
                    let target_id = candidate.target_id.as_deref().ok_or_else(|| {
                        MemoryError::storage("dedupe candidate missing target_id".to_owned())
                    })?;
                    let score = candidate.score.unwrap_or(threshold);
                    let result = transaction_rows(
                        &mut txn,
                        q::LINK_CONSOLIDATED_ENTITY_PAIR,
                        vec![
                            ("a_id", candidate.id.clone().into()),
                            ("b_id", target_id.into()),
                            ("score", score.into()),
                        ],
                    )
                    .await?;
                    mutations_applied += count_from_rows(&result, "created")?;
                }
                Ok(mutations_applied)
            }
            .await;
            finish_run(txn, result, "dedupe_entities", |applied| {
                json!({
                    "candidates": candidates.len(),
                    "applied": applied,
                    "threshold": threshold,
                })
            })
            .await?
        };

        Ok(ConsolidationReport {
            kind: "dedupe_entities".to_owned(),
            dry_run,
            candidates_found: candidates.len(),
            mutations_applied,
            candidates,
            run_id,
        })
    }

    async fn summarize_long_traces(
        &self,
        min_steps: i64,
        limit: i64,
        dry_run: bool,
    ) -> MemoryResult<ConsolidationReport> {
        validate_min_steps(min_steps)?;
        validate_limit(limit)?;

        let rows = self
            .client
            .execute(
                q::FIND_LONG_TRACES,
                vec![("min_steps", min_steps.into()), ("limit", limit.into())],
            )
            .await?;

        let mut candidates = Vec::with_capacity(rows.len());
        for row in &rows {
            let trace_id = required_string(row, "trace_id")?;
            let task = row.get::<String>("task").unwrap_or_default();
            let step_count = required_i64(row, "step_count")?;

            #[allow(clippy::cast_precision_loss)]
            let score = step_count as f64;
            candidates.push(ConsolidationCandidate {
                id: trace_id.clone(),
                kind: "summarize".to_owned(),
                target_id: None,
                score: Some(score),
                reason: format!("Trace {trace_id} ({step_count} steps): '{task}'"),
            });
        }

        let (mutations_applied, run_id) = if dry_run || candidates.is_empty() {
            (0, None)
        } else {
            let mut txn = self.client.start_txn().await?;
            let result = async {
                let trace_ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
                let result = transaction_rows(
                    &mut txn,
                    q::FLAG_LONG_TRACES_FOR_SUMMARY,
                    vec![("trace_ids", trace_ids.into())],
                )
                .await?;
                count_from_rows(&result, "flagged")
            }
            .await;
            finish_run(txn, result, "summarize_long_traces", |applied| {
                json!({
                    "candidates": candidates.len(),
                    "applied": applied,
                    "min_steps": min_steps,
                })
            })
            .await?
        };

        Ok(ConsolidationReport {
            kind: "summarize_long_traces".to_owned(),
            dry_run,
            candidates_found: candidates.len(),
            mutations_applied,
            candidates,
            run_id,
        })
    }

    async fn detect_superseded_preferences(
        &self,
        threshold: f64,
        limit: i64,
        dry_run: bool,
    ) -> MemoryResult<ConsolidationReport> {
        validate_threshold(threshold)?;
        validate_limit(limit)?;

        let rows = self
            .client
            .execute(
                q::FIND_SUPERSEDED_PREFERENCES,
                vec![("threshold", threshold.into()), ("limit", limit.into())],
            )
            .await?;

        let mut seen_old = HashSet::new();
        let mut candidates = Vec::with_capacity(rows.len());
        for row in &rows {
            let old_id = required_string(row, "old_id")?;
            if !seen_old.insert(old_id.clone()) {
                // Keep only the highest-scoring superseding candidate per older preference.
                continue;
            }
            let old_text = required_string(row, "old_text")?;
            let new_id = required_string(row, "new_id")?;
            let new_text = required_string(row, "new_text")?;
            let score = required_f64(row, "score")?;

            candidates.push(ConsolidationCandidate {
                id: old_id,
                kind: "supersede".to_owned(),
                target_id: Some(new_id),
                score: Some(score),
                reason: format!("'{old_text}' -> '{new_text}' (score {score:.3})"),
            });
        }

        let (mutations_applied, run_id) = if dry_run || candidates.is_empty() {
            (0, None)
        } else {
            require_preference_lock_constraint(&self.client).await?;
            let mut txn = self.client.start_txn().await?;
            let result = async {
                // Serialize ownership changes made by write_preference before rechecking candidates.
                transaction_rows(&mut txn, PREFERENCE_WRITE_LOCK, vec![]).await?;
                let mut mutations_applied = 0;
                for candidate in &candidates {
                    let new_id = candidate.target_id.as_deref().ok_or_else(|| {
                        MemoryError::storage("supersede candidate missing target_id".to_owned())
                    })?;
                    let result = transaction_rows(
                        &mut txn,
                        q::APPLY_SUPERSEDE_PREFERENCE,
                        vec![
                            ("old_id", candidate.id.clone().into()),
                            ("new_id", new_id.into()),
                        ],
                    )
                    .await?;
                    mutations_applied += count_from_rows(&result, "updated")?;
                }
                Ok(mutations_applied)
            }
            .await;
            finish_run(txn, result, "detect_superseded_preferences", |applied| {
                json!({
                    "candidates": candidates.len(),
                    "applied": applied,
                    "threshold": threshold,
                })
            })
            .await?
        };

        Ok(ConsolidationReport {
            kind: "detect_superseded_preferences".to_owned(),
            dry_run,
            candidates_found: candidates.len(),
            mutations_applied,
            candidates,
            run_id,
        })
    }

    async fn archive_expired_conversations(
        &self,
        ttl_days: i64,
        limit: i64,
        dry_run: bool,
    ) -> MemoryResult<ConsolidationReport> {
        validate_ttl_days(ttl_days)?;
        validate_limit(limit)?;

        let rows = self
            .client
            .execute(
                q::FIND_EXPIRED_CONVERSATIONS,
                vec![("ttl_days", ttl_days.into()), ("limit", limit.into())],
            )
            .await?;

        let mut candidates = Vec::with_capacity(rows.len());
        for row in &rows {
            let id = required_string(row, "id")?;
            let session_id = required_string(row, "session_id")?;
            let created_label = column(row, "created_at")
                .ok()
                .and_then(|value| to_datetime(&value))
                .map_or_else(|| "unknown".to_owned(), |dt| dt.to_rfc3339());

            #[allow(clippy::cast_precision_loss)]
            let score = ttl_days as f64;
            candidates.push(ConsolidationCandidate {
                id,
                kind: "archive".to_owned(),
                target_id: None,
                score: Some(score),
                reason: format!("Conversation {session_id} ({created_label})"),
            });
        }

        let (mutations_applied, run_id) = if dry_run || candidates.is_empty() {
            (0, None)
        } else {
            let mut txn = self.client.start_txn().await?;
            let result = async {
                let ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
                let result = transaction_rows(
                    &mut txn,
                    q::ARCHIVE_CONVERSATIONS,
                    vec![("ids", ids.into())],
                )
                .await?;
                count_from_rows(&result, "archived")
            }
            .await;
            finish_run(txn, result, "archive_expired_conversations", |applied| {
                json!({
                    "candidates": candidates.len(),
                    "applied": applied,
                    "ttl_days": ttl_days,
                })
            })
            .await?
        };

        Ok(ConsolidationReport {
            kind: "archive_expired_conversations".to_owned(),
            dry_run,
            candidates_found: candidates.len(),
            mutations_applied,
            candidates,
            run_id,
        })
    }

    async fn record_run(&self, kind: &str, stats_json: &str) -> MemoryResult<String> {
        if kind.trim().is_empty() {
            return Err(MemoryError::invalid(
                "consolidation run kind must not be blank",
            ));
        }
        let mut txn = self.client.start_txn().await?;
        let result = write_run(&mut txn, kind, stats_json).await;
        finish_transaction(txn, result).await
    }

    async fn record_read_audit(&self, record: &ReadAuditRecord) -> MemoryResult<String> {
        validate_read_audit(record)?;
        let audit_id = Uuid::new_v4().to_string();
        let result_count = record
            .result_count
            .map_or(BoltType::Null(BoltNull), BoltType::from);

        let mut txn = self.client.start_txn().await?;
        let result = async {
            transaction_rows(
                &mut txn,
                q::CREATE_MEMORY_READ_AUDIT,
                vec![
                    ("audit_id", audit_id.clone().into()),
                    ("kind", record.kind.clone().into()),
                    ("query", record.query.clone().into()),
                    (
                        "user_identifier",
                        opt_string(record.user_identifier.as_deref()),
                    ),
                    ("result_count", result_count),
                    ("metadata_json", opt_string(record.metadata_json.as_deref())),
                ],
            )
            .await?;

            if let Some(user_identifier) = record.user_identifier.as_deref() {
                let rows = transaction_rows(
                    &mut txn,
                    q::LINK_USER_TO_READ_AUDIT,
                    vec![
                        ("user_identifier", user_identifier.into()),
                        ("audit_id", audit_id.clone().into()),
                    ],
                )
                .await?;
                if rows.len() != 1 {
                    return Err(MemoryError::storage(
                        "could not link the read audit to its user",
                    ));
                }
            }

            Ok(audit_id)
        }
        .await;
        finish_transaction(txn, result).await
    }
}

async fn write_run(txn: &mut Txn, kind: &str, stats_json: &str) -> MemoryResult<String> {
    let run_id = Uuid::new_v4().to_string();
    transaction_rows(
        txn,
        q::CREATE_CONSOLIDATION_RUN,
        vec![
            ("id", run_id.clone().into()),
            ("kind", kind.into()),
            ("stats_json", stats_json.into()),
        ],
    )
    .await?;
    Ok(run_id)
}

async fn finish_run(
    mut txn: Txn,
    result: MemoryResult<usize>,
    kind: &str,
    stats: impl FnOnce(usize) -> serde_json::Value,
) -> MemoryResult<(usize, Option<String>)> {
    let result = match result {
        Ok(0) => {
            // Leave the graph unchanged, including any preference lock update.
            txn.rollback()
                .await
                .map_err(|err| MemoryError::storage(err.to_string()))?;
            return Ok((0, None));
        }
        Ok(applied) => write_run(&mut txn, kind, &stats(applied).to_string())
            .await
            .map(|run_id| (applied, Some(run_id))),
        Err(err) => Err(err),
    };
    finish_transaction(txn, result).await
}

async fn finish_transaction<T>(txn: Txn, result: MemoryResult<T>) -> MemoryResult<T> {
    match result {
        Ok(value) => {
            txn.commit()
                .await
                .map_err(|err| MemoryError::storage(err.to_string()))?;
            Ok(value)
        }
        Err(err) => {
            // Bolt can reject rollback after a failed statement. Preserve the
            // original error; neo4rs resets the connection before reusing it.
            let _ = txn.rollback().await;
            Err(err)
        }
    }
}

fn validate_threshold(threshold: f64) -> MemoryResult<()> {
    if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
        return Err(MemoryError::invalid(
            "consolidation threshold must be finite and within [0.0, 1.0]",
        ));
    }
    Ok(())
}

fn validate_limit(limit: i64) -> MemoryResult<()> {
    if limit < 1 {
        return Err(MemoryError::invalid(
            "consolidation limit must be at least 1",
        ));
    }
    Ok(())
}

fn validate_min_steps(min_steps: i64) -> MemoryResult<()> {
    if min_steps < 1 {
        return Err(MemoryError::invalid(
            "consolidation min_steps must be at least 1",
        ));
    }
    Ok(())
}

fn validate_ttl_days(ttl_days: i64) -> MemoryResult<()> {
    if ttl_days < 0 {
        return Err(MemoryError::invalid(
            "consolidation ttl_days must not be negative",
        ));
    }
    Ok(())
}

fn validate_read_audit(record: &ReadAuditRecord) -> MemoryResult<()> {
    if record.query.trim().is_empty() {
        return Err(MemoryError::invalid("read audit query must not be blank"));
    }
    if record.kind.trim().is_empty() {
        return Err(MemoryError::invalid("read audit kind must not be blank"));
    }
    if let Some(user) = record.user_identifier.as_deref() {
        if user.trim().is_empty() {
            return Err(MemoryError::invalid(
                "read audit user_identifier must not be blank",
            ));
        }
    }
    if let Some(count) = record.result_count {
        if count < 0 {
            return Err(MemoryError::invalid(
                "read audit result_count must not be negative",
            ));
        }
    }
    Ok(())
}

fn required_string(row: &Row, key: &str) -> MemoryResult<String> {
    row.get::<String>(key)
        .map_err(|_| MemoryError::storage(format!("missing string column '{key}'")))
}

fn required_f64(row: &Row, key: &str) -> MemoryResult<f64> {
    row.get::<f64>(key)
        .map_err(|_| MemoryError::storage(format!("missing float column '{key}'")))
}

fn required_i64(row: &Row, key: &str) -> MemoryResult<i64> {
    row.get::<i64>(key)
        .map_err(|_| MemoryError::storage(format!("missing integer column '{key}'")))
}

fn count_from_rows(rows: &[Row], key: &str) -> MemoryResult<usize> {
    let Some(row) = rows.first() else {
        return Ok(0);
    };
    let count = required_i64(row, key)?;
    usize::try_from(count)
        .map_err(|_| MemoryError::storage(format!("negative count in column '{key}': {count}")))
}
