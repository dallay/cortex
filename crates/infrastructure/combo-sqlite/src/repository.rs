use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rook_core::ports::{ComboRepositoryError, ComboRepositoryPort};
use rook_core::{Combo, ComboStep, ComboStrategy};
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};
use shared_kernel::{ComboId, ConnectionId, ModelId, ProviderId};

pub struct ComboSqliteRepository {
    conn: Mutex<Connection>,
}

impl ComboSqliteRepository {
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;",
        )?;
        db_migration::run_on_connection(&mut conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ComboRepositoryError> {
        self.conn
            .lock()
            .map_err(|_| ComboRepositoryError::Database("sqlite mutex poisoned".to_string()))
    }
}

#[async_trait]
impl ComboRepositoryPort for ComboSqliteRepository {
    async fn list(&self) -> Result<Vec<Combo>, ComboRepositoryError> {
        let conn = self.lock()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, name, strategy, created_at, updated_at
                 FROM combos
                 ORDER BY created_at DESC",
            )
            .map_err(db_error)?;

        let combo_rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(db_error)?;

        let mut combos = Vec::new();
        for combo_row in combo_rows {
            let (id_str, name, strategy_str, created_at_str, updated_at_str) =
                combo_row.map_err(db_error)?;

            let id = ComboId::parse_str(&id_str)
                .map_err(|e| ComboRepositoryError::Database(format!("invalid combo id: {e}")))?;
            let strategy = parse_strategy(&strategy_str)?;
            let created_at = parse_datetime(&created_at_str)?;
            let updated_at = parse_datetime(&updated_at_str)?;

            let steps = load_steps(&conn, &id)?;

            combos.push(Combo {
                id,
                name,
                strategy,
                steps,
                created_at,
                updated_at,
            });
        }

        Ok(combos)
    }

    async fn find(&self, id: &ComboId) -> Result<Option<Combo>, ComboRepositoryError> {
        let conn = self.lock()?;
        let combo_row = conn
            .query_row(
                "SELECT id, name, strategy, created_at, updated_at
                 FROM combos
                 WHERE id = ?1",
                params![id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(db_error)?;

        match combo_row {
            Some((id_str, name, strategy_str, created_at_str, updated_at_str)) => {
                let id = ComboId::parse_str(&id_str).map_err(|e| {
                    ComboRepositoryError::Database(format!("invalid combo id: {e}"))
                })?;
                let strategy = parse_strategy(&strategy_str)?;
                let created_at = parse_datetime(&created_at_str)?;
                let updated_at = parse_datetime(&updated_at_str)?;

                let steps = load_steps(&conn, &id)?;

                Ok(Some(Combo {
                    id,
                    name,
                    strategy,
                    steps,
                    created_at,
                    updated_at,
                }))
            }
            None => Ok(None),
        }
    }

    async fn find_by_name(&self, name: &str) -> Result<Option<Combo>, ComboRepositoryError> {
        let conn = self.lock()?;
        let combo_row = conn
            .query_row(
                "SELECT id, name, strategy, created_at, updated_at
                 FROM combos
                 WHERE name = ?1 COLLATE NOCASE",
                params![name],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(db_error)?;

        match combo_row {
            Some((id_str, name, strategy_str, created_at_str, updated_at_str)) => {
                let id = ComboId::parse_str(&id_str).map_err(|e| {
                    ComboRepositoryError::Database(format!("invalid combo id: {e}"))
                })?;
                let strategy = parse_strategy(&strategy_str)?;
                let created_at = parse_datetime(&created_at_str)?;
                let updated_at = parse_datetime(&updated_at_str)?;

                let steps = load_steps(&conn, &id)?;

                Ok(Some(Combo {
                    id,
                    name,
                    strategy,
                    steps,
                    created_at,
                    updated_at,
                }))
            }
            None => Ok(None),
        }
    }

    async fn create(&self, combo: &Combo) -> Result<(), ComboRepositoryError> {
        // Validate before persisting
        combo.validate().map_err(ComboRepositoryError::Validation)?;

        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(db_error)?;

        // Check for duplicate name
        let exists = tx
            .query_row(
                "SELECT 1 FROM combos WHERE name = ?1 COLLATE NOCASE",
                params![combo.name],
                |_| Ok(()),
            )
            .optional()
            .map_err(db_error)?
            .is_some();

        if exists {
            return Err(ComboRepositoryError::DuplicateName(combo.name.clone()));
        }

        // Insert combo
        tx.execute(
            "INSERT INTO combos (id, name, strategy, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                combo.id.to_string(),
                combo.name,
                strategy_to_string(combo.strategy),
                combo.created_at.to_rfc3339(),
                combo.updated_at.to_rfc3339(),
            ],
        )
        .map_err(db_error)?;

        // Insert steps
        for (idx, step) in combo.steps.iter().enumerate() {
            tx.execute(
                "INSERT INTO combo_steps (combo_id, step_order, provider_id, model, connection_id, priority)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    combo.id.to_string(),
                    idx as i64,
                    step.provider_id.as_str(),
                    step.model.as_str(),
                    step.connection_id.as_ref().map(|id| id.to_string()),
                    step.priority,
                ],
            )
            .map_err(|e| {
                if e.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) {
                    ComboRepositoryError::Validation(
                        rook_core::ComboValidationError::DuplicatePriority {
                            priority: step.priority,
                        },
                    )
                } else {
                    db_error(e)
                }
            })?;
        }

        tx.commit().map_err(db_error)
    }

    async fn update(&self, combo: &Combo) -> Result<(), ComboRepositoryError> {
        // Validate before persisting
        combo.validate().map_err(ComboRepositoryError::Validation)?;

        let mut conn = self.lock()?;
        let tx = conn.transaction().map_err(db_error)?;

        // Check if combo exists
        let exists = tx
            .query_row(
                "SELECT 1 FROM combos WHERE id = ?1",
                params![combo.id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(db_error)?
            .is_some();

        if !exists {
            return Err(ComboRepositoryError::NotFound(combo.id));
        }

        // Check for duplicate name (excluding current combo)
        let name_conflict = tx
            .query_row(
                "SELECT 1 FROM combos WHERE name = ?1 COLLATE NOCASE AND id != ?2",
                params![combo.name, combo.id.to_string()],
                |_| Ok(()),
            )
            .optional()
            .map_err(db_error)?
            .is_some();

        if name_conflict {
            return Err(ComboRepositoryError::DuplicateName(combo.name.clone()));
        }

        // Update combo
        tx.execute(
            "UPDATE combos SET name = ?1, strategy = ?2, updated_at = ?3 WHERE id = ?4",
            params![
                combo.name,
                strategy_to_string(combo.strategy),
                combo.updated_at.to_rfc3339(),
                combo.id.to_string(),
            ],
        )
        .map_err(db_error)?;

        // Delete old steps
        tx.execute(
            "DELETE FROM combo_steps WHERE combo_id = ?1",
            params![combo.id.to_string()],
        )
        .map_err(db_error)?;

        // Insert new steps
        for (idx, step) in combo.steps.iter().enumerate() {
            tx.execute(
                "INSERT INTO combo_steps (combo_id, step_order, provider_id, model, connection_id, priority)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    combo.id.to_string(),
                    idx as i64,
                    step.provider_id.as_str(),
                    step.model.as_str(),
                    step.connection_id.as_ref().map(|id| id.to_string()),
                    step.priority,
                ],
            )
            .map_err(|e| {
                if e.sqlite_error_code() == Some(ErrorCode::ConstraintViolation) {
                    ComboRepositoryError::Validation(
                        rook_core::ComboValidationError::DuplicatePriority {
                            priority: step.priority,
                        },
                    )
                } else {
                    db_error(e)
                }
            })?;
        }

        tx.commit().map_err(db_error)
    }

    async fn delete(&self, id: &ComboId) -> Result<(), ComboRepositoryError> {
        let conn = self.lock()?;
        let rows = conn
            .execute("DELETE FROM combos WHERE id = ?1", params![id.to_string()])
            .map_err(db_error)?;

        if rows == 0 {
            return Err(ComboRepositoryError::NotFound(*id));
        }

        Ok(())
    }
}

// Helper functions

fn load_steps(
    conn: &Connection,
    combo_id: &ComboId,
) -> Result<Vec<ComboStep>, ComboRepositoryError> {
    let mut stmt = conn
        .prepare(
            "SELECT provider_id, model, connection_id, priority
             FROM combo_steps
             WHERE combo_id = ?1
             ORDER BY step_order ASC",
        )
        .map_err(db_error)?;

    let step_rows = stmt
        .query_map(params![combo_id.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, u8>(3)?,
            ))
        })
        .map_err(db_error)?;

    let mut steps = Vec::new();
    for step_row in step_rows {
        let (provider_id_str, model_str, connection_id_str, priority) =
            step_row.map_err(db_error)?;

        let connection_id = match connection_id_str {
            Some(id_str) => Some(ConnectionId::parse_str(&id_str).map_err(|e| {
                ComboRepositoryError::Database(format!("invalid connection id: {e}"))
            })?),
            None => None,
        };

        steps.push(ComboStep {
            provider_id: ProviderId::new(provider_id_str),
            model: ModelId::new(model_str),
            connection_id,
            priority,
        });
    }

    Ok(steps)
}

fn parse_strategy(s: &str) -> Result<ComboStrategy, ComboRepositoryError> {
    match s {
        "priority" => Ok(ComboStrategy::Priority),
        _ => Err(ComboRepositoryError::Database(format!(
            "unknown strategy: {s}"
        ))),
    }
}

fn strategy_to_string(strategy: ComboStrategy) -> &'static str {
    match strategy {
        ComboStrategy::Priority => "priority",
    }
}

fn parse_datetime(s: &str) -> Result<DateTime<Utc>, ComboRepositoryError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| ComboRepositoryError::Database(format!("invalid datetime: {e}")))
}

fn db_error(e: rusqlite::Error) -> ComboRepositoryError {
    ComboRepositoryError::Database(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    async fn setup_test_repo() -> (ComboSqliteRepository, NamedTempFile) {
        let temp_file = NamedTempFile::new().expect("create temp db");
        let repo = ComboSqliteRepository::new(temp_file.path()).expect("create repo");

        // Run migration to create tables
        let conn = repo.conn.lock().unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS combos (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                strategy TEXT NOT NULL,
                weights TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            
            CREATE UNIQUE INDEX IF NOT EXISTS idx_combos_name ON combos(name COLLATE NOCASE);
            
            CREATE TABLE IF NOT EXISTS combo_steps (
                combo_id TEXT NOT NULL,
                step_order INTEGER NOT NULL,
                provider_id TEXT NOT NULL,
                model TEXT NOT NULL,
                connection_id TEXT,
                priority INTEGER NOT NULL,
                PRIMARY KEY (combo_id, step_order),
                FOREIGN KEY (combo_id) REFERENCES combos(id) ON DELETE CASCADE
            );
            
            CREATE INDEX IF NOT EXISTS idx_combo_steps_combo_id ON combo_steps(combo_id);
            "#,
        )
        .expect("run migration");
        drop(conn);

        (repo, temp_file)
    }

    fn create_test_combo(name: &str, steps: Vec<ComboStep>) -> Combo {
        Combo::new(name.to_string(), ComboStrategy::Priority, steps)
    }

    #[tokio::test]
    async fn create_and_find_combo() {
        let (repo, _temp) = setup_test_repo().await;

        let combo = create_test_combo(
            "test-combo",
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );

        repo.create(&combo).await.expect("create combo");

        let found = repo.find(&combo.id).await.expect("find combo");
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.name, "test-combo");
        assert_eq!(found.steps.len(), 1);
        assert_eq!(found.steps[0].priority, 1);
    }

    #[tokio::test]
    async fn find_by_name_case_insensitive() {
        let (repo, _temp) = setup_test_repo().await;

        let combo = create_test_combo(
            "Test-Combo",
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );

        repo.create(&combo).await.expect("create combo");

        let found = repo.find_by_name("test-combo").await.expect("find by name");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Test-Combo");
    }

    #[tokio::test]
    async fn create_rejects_duplicate_name() {
        let (repo, _temp) = setup_test_repo().await;

        let combo1 = create_test_combo(
            "duplicate",
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );

        let combo2 = create_test_combo(
            "Duplicate",
            vec![ComboStep {
                provider_id: ProviderId::new("anthropic"),
                model: ModelId::new("claude-opus-4"),
                connection_id: None,
                priority: 1,
            }],
        );

        repo.create(&combo1).await.expect("create first combo");
        let result = repo.create(&combo2).await;
        assert!(matches!(
            result,
            Err(ComboRepositoryError::DuplicateName(_))
        ));
    }

    #[tokio::test]
    async fn update_combo_replaces_steps() {
        let (repo, _temp) = setup_test_repo().await;

        let mut combo = create_test_combo(
            "test",
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );

        repo.create(&combo).await.expect("create combo");

        // Update with new steps
        combo.steps = vec![
            ComboStep {
                provider_id: ProviderId::new("anthropic"),
                model: ModelId::new("claude-opus-4"),
                connection_id: None,
                priority: 1,
            },
            ComboStep {
                provider_id: ProviderId::new("ollama"),
                model: ModelId::new("llama3.1:70b"),
                connection_id: None,
                priority: 2,
            },
        ];

        repo.update(&combo).await.expect("update combo");

        let found = repo.find(&combo.id).await.expect("find combo").unwrap();
        assert_eq!(found.steps.len(), 2);
        assert_eq!(found.steps[0].provider_id, ProviderId::new("anthropic"));
        assert_eq!(found.steps[1].provider_id, ProviderId::new("ollama"));
    }

    #[tokio::test]
    async fn delete_combo_cascades_to_steps() {
        let (repo, _temp) = setup_test_repo().await;

        let combo = create_test_combo(
            "test",
            vec![
                ComboStep {
                    provider_id: ProviderId::new("openai"),
                    model: ModelId::new("gpt-4o"),
                    connection_id: None,
                    priority: 1,
                },
                ComboStep {
                    provider_id: ProviderId::new("anthropic"),
                    model: ModelId::new("claude-opus-4"),
                    connection_id: None,
                    priority: 2,
                },
            ],
        );

        repo.create(&combo).await.expect("create combo");
        repo.delete(&combo.id).await.expect("delete combo");

        let found = repo.find(&combo.id).await.expect("find after delete");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_combo_returns_not_found() {
        let (repo, _temp) = setup_test_repo().await;
        let result = repo.delete(&ComboId::new()).await;
        assert!(matches!(result, Err(ComboRepositoryError::NotFound(_))));
    }

    #[tokio::test]
    async fn list_returns_combos_ordered_by_created_at_desc() {
        let (repo, _temp) = setup_test_repo().await;

        let combo1 = create_test_combo(
            "first",
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );

        let combo2 = create_test_combo(
            "second",
            vec![ComboStep {
                provider_id: ProviderId::new("anthropic"),
                model: ModelId::new("claude-opus-4"),
                connection_id: None,
                priority: 1,
            }],
        );

        repo.create(&combo1).await.expect("create first");
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        repo.create(&combo2).await.expect("create second");

        let list = repo.list().await.expect("list combos");
        assert_eq!(list.len(), 2);
        // Most recent first
        assert_eq!(list[0].name, "second");
        assert_eq!(list[1].name, "first");
    }

    #[tokio::test]
    async fn create_rejects_invalid_combo() {
        let (repo, _temp) = setup_test_repo().await;

        let combo = create_test_combo("", vec![]); // Empty name and empty steps

        let result = repo.create(&combo).await;
        assert!(matches!(result, Err(ComboRepositoryError::Validation(_))));
    }

    #[tokio::test]
    async fn update_rejects_invalid_combo() {
        let (repo, _temp) = setup_test_repo().await;

        let mut combo = create_test_combo(
            "valid",
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );

        repo.create(&combo).await.expect("create combo");

        // Make it invalid
        combo.steps = vec![];

        let result = repo.update(&combo).await;
        assert!(matches!(result, Err(ComboRepositoryError::Validation(_))));
    }

    #[tokio::test]
    async fn update_nonexistent_combo_returns_not_found() {
        let (repo, _temp) = setup_test_repo().await;

        let combo = create_test_combo(
            "test",
            vec![ComboStep {
                provider_id: ProviderId::new("openai"),
                model: ModelId::new("gpt-4o"),
                connection_id: None,
                priority: 1,
            }],
        );

        let result = repo.update(&combo).await;
        assert!(matches!(result, Err(ComboRepositoryError::NotFound(_))));
    }
}
