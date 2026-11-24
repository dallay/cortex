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
use neo4rs::{BoltNull, BoltType, Query, Row, Txn};
use serde_json::json;
use uuid::Uuid;

use crate::bolt::{column, opt_string, to_datetime};
use crate::client::{Neo4jClient, Params};
use crate::long_term::PREFERENCE_WRITE_LOCK;
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

        let mut mutations_applied = 0usize;
        let mut run_id = None;

        if !dry_run && !candidates.is_empty() {
            let mut txn = self.client.start_txn().await?;
            let tx_result: MemoryResult<(usize, Option<String>)> = async {
                let mut applied = 0usize;
                for candidate in &candidates {
                    let target_id = candidate.target_id.as_deref().ok_or_else(|| {
                        MemoryError::storage("dedupe candidate missing target_id".to_owned())
                    })?;
                    let score = candidate.score.unwrap_or(threshold);
                    let result = execute_in_txn(
                        &mut txn,
                        q::LINK_CONSOLIDATED_ENTITY_PAIR,
                        vec![
                            ("a_id", candidate.id.clone().into()),
                            ("b_id", target_id.into()),
                            ("score", score.into()),
                        ],
                    )
                    .await?;
                    applied += count_from_rows(&result, "created")?;
                }

                let id = if applied > 0 {
                    let stats = json!({
                        "candidates": candidates.len(),
                        "applied": applied,
                        "threshold": threshold,
                    });
                    Some(record_run_in_txn(&mut txn, "dedupe_entities", &stats.to_string()).await?)
                } else {
                    None
                };
                Ok((applied, id))
            }
            .await;

            match tx_result {
                Ok((applied, id)) => {
                    commit_txn(txn).await?;
                    mutations_applied = applied;
                    run_id = id;
                }
                Err(err) => {
                    let _ = txn.rollback().await;
                    return Err(err);
                }
            }
        }

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

        let mut mutations_applied = 0usize;
        let mut run_id = None;

        if !dry_run && !candidates.is_empty() {
            let trace_ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
            let mut txn = self.client.start_txn().await?;
            let tx_result: MemoryResult<(usize, Option<String>)> = async {
                let result = execute_in_txn(
                    &mut txn,
                    q::FLAG_LONG_TRACES_FOR_SUMMARY,
                    vec![("trace_ids", trace_ids.into())],
                )
                .await?;
                let applied = count_from_rows(&result, "flagged")?;

                let id = if applied > 0 {
                    let stats = json!({
                        "candidates": candidates.len(),
                        "applied": applied,
                        "min_steps": min_steps,
                    });
                    Some(
                        record_run_in_txn(&mut txn, "summarize_long_traces", &stats.to_string())
                            .await?,
                    )
                } else {
                    None
                };
                Ok((applied, id))
            }
            .await;

            match tx_result {
                Ok((applied, id)) => {
                    commit_txn(txn).await?;
                    mutations_applied = applied;
                    run_id = id;
                }
                Err(err) => {
                    let _ = txn.rollback().await;
                    return Err(err);
                }
            }
        }

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

        let mut mutations_applied = 0usize;
        let mut run_id = None;

        if !dry_run && !candidates.is_empty() {
            let mut txn = self.client.start_txn().await?;
            let tx_result: MemoryResult<(usize, Option<String>)> = async {
                // Serialize preference supersession mutations with write_preference.
                execute_in_txn(&mut txn, PREFERENCE_WRITE_LOCK, vec![]).await?;
                let mut applied = 0usize;
                for candidate in &candidates {
                    let new_id = candidate.target_id.as_deref().ok_or_else(|| {
                        MemoryError::storage("supersede candidate missing target_id".to_owned())
                    })?;
                    let result = execute_in_txn(
                        &mut txn,
                        q::APPLY_SUPERSEDE_PREFERENCE,
                        vec![
                            ("old_id", candidate.id.clone().into()),
                            ("new_id", new_id.into()),
                        ],
                    )
                    .await?;
                    applied += count_from_rows(&result, "updated")?;
                }

                let id = if applied > 0 {
                    let stats = json!({
                        "candidates": candidates.len(),
                        "applied": applied,
                        "threshold": threshold,
                    });
                    Some(
                        record_run_in_txn(
                            &mut txn,
                            "detect_superseded_preferences",
                            &stats.to_string(),
                        )
                        .await?,
                    )
                } else {
                    None
                };
                Ok((applied, id))
            }
            .await;

            match tx_result {
                Ok((applied, id)) => {
                    commit_txn(txn).await?;
                    mutations_applied = applied;
                    run_id = id;
                }
                Err(err) => {
                    let _ = txn.rollback().await;
                    return Err(err);
                }
            }
        }

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

        let mut mutations_applied = 0usize;
        let mut run_id = None;

        if !dry_run && !candidates.is_empty() {
            let ids: Vec<String> = candidates.iter().map(|c| c.id.clone()).collect();
            let mut txn = self.client.start_txn().await?;
            let tx_result: MemoryResult<(usize, Option<String>)> = async {
                let result = execute_in_txn(
                    &mut txn,
                    q::ARCHIVE_CONVERSATIONS,
                    vec![("ids", ids.into())],
                )
                .await?;
                let applied = count_from_rows(&result, "archived")?;

                let id = if applied > 0 {
                    let stats = json!({
                        "candidates": candidates.len(),
                        "applied": applied,
                        "ttl_days": ttl_days,
                    });
                    Some(
                        record_run_in_txn(
                            &mut txn,
                            "archive_expired_conversations",
                            &stats.to_string(),
                        )
                        .await?,
                    )
                } else {
                    None
                };
                Ok((applied, id))
            }
            .await;

            match tx_result {
                Ok((applied, id)) => {
                    commit_txn(txn).await?;
                    mutations_applied = applied;
                    run_id = id;
                }
                Err(err) => {
                    let _ = txn.rollback().await;
                    return Err(err);
                }
            }
        }

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
        let run_id = Uuid::new_v4().to_string();
        self.client
            .execute(
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

    async fn record_read_audit(&self, record: &ReadAuditRecord) -> MemoryResult<String> {
        validate_read_audit(record)?;
        let audit_id = Uuid::new_v4().to_string();
        let result_count = record
            .result_count
            .map_or(BoltType::Null(BoltNull), BoltType::from);

        let mut txn = self.client.start_txn().await?;
        let tx_result: MemoryResult<()> = async {
            execute_in_txn(
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
                execute_in_txn(
                    &mut txn,
                    q::LINK_USER_TO_READ_AUDIT,
                    vec![
                        ("user_identifier", user_identifier.into()),
                        ("audit_id", audit_id.clone().into()),
                    ],
                )
                .await?;
            }
            Ok(())
        }
        .await;

        match tx_result {
            Ok(()) => {
                commit_txn(txn).await?;
                Ok(audit_id)
            }
            Err(err) => {
                let _ = txn.rollback().await;
                Err(err)
            }
        }
    }
}

async fn execute_in_txn(txn: &mut Txn, cypher: &str, params: Params) -> MemoryResult<Vec<Row>> {
    let mut query = Query::new(cypher.to_owned());
    for (name, value) in params {
        query = query.param(name, value);
    }
    let mut stream = txn
        .execute(query)
        .await
        .map_err(|err| MemoryError::storage(err.to_string()))?;
    let mut rows = Vec::new();
    while let Some(row) = stream
        .next(txn.handle())
        .await
        .map_err(|err| MemoryError::storage(err.to_string()))?
    {
        rows.push(row);
    }
    Ok(rows)
}

async fn commit_txn(txn: Txn) -> MemoryResult<()> {
    txn.commit()
        .await
        .map_err(|err| MemoryError::storage(err.to_string()))
}

async fn record_run_in_txn(txn: &mut Txn, kind: &str, stats_json: &str) -> MemoryResult<String> {
    if kind.trim().is_empty() {
        return Err(MemoryError::invalid(
            "consolidation run kind must not be blank",
        ));
    }
    let run_id = Uuid::new_v4().to_string();
    execute_in_txn(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_thresholds() {
        assert!(validate_threshold(0.0).is_ok());
        assert!(validate_threshold(0.85).is_ok());
        assert!(validate_threshold(1.0).is_ok());
        assert!(validate_threshold(-0.01).is_err());
        assert!(validate_threshold(1.01).is_err());
        assert!(validate_threshold(f64::NAN).is_err());
        assert!(validate_threshold(f64::INFINITY).is_err());
    }

    #[test]
    fn validates_limits_steps_and_ttl() {
        assert!(validate_limit(1).is_ok());
        assert!(validate_limit(0).is_err());
        assert!(validate_limit(-5).is_err());

        assert!(validate_min_steps(1).is_ok());
        assert!(validate_min_steps(0).is_err());

        assert!(validate_ttl_days(0).is_ok());
        assert!(validate_ttl_days(90).is_ok());
        assert!(validate_ttl_days(-1).is_err());
    }

    #[test]
    fn validates_read_audit_records() {
        let mut record = ReadAuditRecord::new("search preferences");
        assert!(validate_read_audit(&record).is_ok());

        record.user_identifier = Some("alice".to_owned());
        record.result_count = Some(3);
        assert!(validate_read_audit(&record).is_ok());

        let blank_query = ReadAuditRecord::new("   ");
        assert!(validate_read_audit(&blank_query).is_err());

        let mut blank_kind = ReadAuditRecord::new("ok");
        blank_kind.kind = " ".to_owned();
        assert!(validate_read_audit(&blank_kind).is_err());

        let mut blank_user = ReadAuditRecord::new("ok");
        blank_user.user_identifier = Some("  ".to_owned());
        assert!(validate_read_audit(&blank_user).is_err());

        let mut negative_count = ReadAuditRecord::new("ok");
        negative_count.result_count = Some(-1);
        assert!(validate_read_audit(&negative_count).is_err());
    }
}
