use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use async_trait::async_trait;
use chrono::Utc;
use rook_core::ports::{ModelAliasRepositoryError, ModelAliasRepositoryPort};
use rook_core::ModelAlias;
use rusqlite::{params, Connection, OptionalExtension};
use shared_kernel::{ModelId, ProviderId};

use crate::builtin::DEFAULT_ALIASES;

pub struct SqliteModelAliasRepository {
    conn: Mutex<Connection>,
}

impl SqliteModelAliasRepository {
    pub fn new(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut conn = Connection::open(&db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA synchronous = NORMAL;
             PRAGMA foreign_keys = ON;",
        )?;

        // Run migrations for in-memory databases
        if db_path.as_ref().to_str() == Some(":memory:") {
            db_migration::run_on_connection(&mut conn)?;
        }

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Connection>, ModelAliasRepositoryError> {
        self.conn
            .lock()
            .map_err(|_| ModelAliasRepositoryError::Database("sqlite mutex poisoned".to_string()))
    }
}

#[async_trait]
impl ModelAliasRepositoryPort for SqliteModelAliasRepository {
    async fn find_by_alias(
        &self,
        alias: &ModelId,
        provider_id: Option<&ProviderId>,
    ) -> Result<Option<ModelAlias>, ModelAliasRepositoryError> {
        let conn = self.lock()?;

        let result = if let Some(pid) = provider_id {
            // Provider-scoped query
            conn.query_row(
                "SELECT alias, canonical, provider_id, created_at 
                 FROM model_aliases 
                 WHERE alias = ?1 AND (provider_id = ?2 OR provider_id IS NULL)
                 ORDER BY CASE WHEN provider_id IS NOT NULL THEN 0 ELSE 1 END
                 LIMIT 1",
                params![alias.as_str(), pid.as_str()],
                |row| {
                    Ok(ModelAlias {
                        alias: ModelId::new(row.get::<_, String>(0)?),
                        canonical: ModelId::new(row.get::<_, String>(1)?),
                        provider_id: row
                            .get::<_, Option<String>>(2)?
                            .map(|s| ProviderId::new(&s)),
                        created_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|e| ModelAliasRepositoryError::Database(e.to_string()))?
        } else {
            // Global query
            conn.query_row(
                "SELECT alias, canonical, provider_id, created_at 
                 FROM model_aliases 
                 WHERE alias = ?1 
                 LIMIT 1",
                params![alias.as_str()],
                |row| {
                    Ok(ModelAlias {
                        alias: ModelId::new(row.get::<_, String>(0)?),
                        canonical: ModelId::new(row.get::<_, String>(1)?),
                        provider_id: row
                            .get::<_, Option<String>>(2)?
                            .map(|s| ProviderId::new(&s)),
                        created_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|e| ModelAliasRepositoryError::Database(e.to_string()))?
        };

        Ok(result)
    }

    async fn list(&self) -> Result<Vec<ModelAlias>, ModelAliasRepositoryError> {
        let conn = self.lock()?;

        let mut stmt = conn
            .prepare(
                "SELECT alias, canonical, provider_id, created_at 
                 FROM model_aliases 
                 ORDER BY alias",
            )
            .map_err(|e| ModelAliasRepositoryError::Database(e.to_string()))?;

        let aliases = stmt
            .query_map([], |row| {
                Ok(ModelAlias {
                    alias: ModelId::new(row.get::<_, String>(0)?),
                    canonical: ModelId::new(row.get::<_, String>(1)?),
                    provider_id: row
                        .get::<_, Option<String>>(2)?
                        .map(|s| ProviderId::new(&s)),
                    created_at: row.get(3)?,
                })
            })
            .map_err(|e| ModelAliasRepositoryError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ModelAliasRepositoryError::Database(e.to_string()))?;

        Ok(aliases)
    }

    async fn create(&self, alias: ModelAlias) -> Result<(), ModelAliasRepositoryError> {
        let conn = self.lock()?;

        // Check if canonical is itself an alias (prevent cycles)
        let canonical_is_alias = conn
            .query_row(
                "SELECT 1 FROM model_aliases WHERE alias = ?1 LIMIT 1",
                params![alias.canonical.as_str()],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| ModelAliasRepositoryError::Database(e.to_string()))?;

        if canonical_is_alias.is_some() {
            return Err(ModelAliasRepositoryError::InvalidAlias(
                "Canonical model cannot be an alias".to_string(),
            ));
        }

        // Insert the alias
        let result = conn.execute(
            "INSERT INTO model_aliases (alias, canonical, provider_id, created_at) 
             VALUES (?1, ?2, ?3, ?4)",
            params![
                alias.alias.as_str(),
                alias.canonical.as_str(),
                alias.provider_id.as_ref().map(|p| p.as_str()),
                alias.created_at,
            ],
        );

        match result {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(ModelAliasRepositoryError::AlreadyExists(alias.alias))
            }
            Err(e) => Err(ModelAliasRepositoryError::Database(e.to_string())),
        }
    }

    async fn delete(&self, alias: &ModelId) -> Result<bool, ModelAliasRepositoryError> {
        let conn = self.lock()?;

        let rows_affected = conn
            .execute(
                "DELETE FROM model_aliases WHERE alias = ?1",
                params![alias.as_str()],
            )
            .map_err(|e| ModelAliasRepositoryError::Database(e.to_string()))?;

        Ok(rows_affected > 0)
    }

    async fn seed(&self, aliases: Vec<ModelAlias>) -> Result<usize, ModelAliasRepositoryError> {
        let mut conn = self.lock()?;

        let tx = conn
            .transaction()
            .map_err(|e| ModelAliasRepositoryError::Database(e.to_string()))?;

        let mut inserted = 0;
        for alias in aliases {
            let result = tx.execute(
                "INSERT OR IGNORE INTO model_aliases (alias, canonical, provider_id, created_at) 
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    alias.alias.as_str(),
                    alias.canonical.as_str(),
                    alias.provider_id.as_ref().map(|p| p.as_str()),
                    alias.created_at,
                ],
            );

            match result {
                Ok(rows) => inserted += rows,
                Err(e) => {
                    return Err(ModelAliasRepositoryError::Database(e.to_string()));
                }
            }
        }

        tx.commit()
            .map_err(|e| ModelAliasRepositoryError::Database(e.to_string()))?;

        Ok(inserted)
    }
}

/// Helper function to create built-in aliases from constants
pub fn builtin_aliases() -> Vec<ModelAlias> {
    let now = Utc::now().to_rfc3339();

    DEFAULT_ALIASES
        .iter()
        .map(|(alias_str, canonical_str, provider_id_str)| {
            let provider_id = provider_id_str.map(ProviderId::new);
            ModelAlias {
                alias: ModelId::new(alias_str.to_string()),
                canonical: ModelId::new(canonical_str.to_string()),
                provider_id,
                created_at: now.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn create_test_repo() -> SqliteModelAliasRepository {
        SqliteModelAliasRepository::new(":memory:").expect("failed to create test repository")
    }

    fn test_alias(alias: &str, canonical: &str) -> ModelAlias {
        ModelAlias {
            alias: ModelId::new(alias),
            canonical: ModelId::new(canonical),
            provider_id: None,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    #[tokio::test]
    async fn test_find_by_alias_found() {
        let repo = create_test_repo().await;
        let alias = test_alias("gpt-4o-latest", "gpt-4o-2024-05-13");

        repo.create(alias.clone()).await.expect("create failed");

        let result = repo
            .find_by_alias(&ModelId::new("gpt-4o-latest"), None)
            .await
            .expect("find failed");

        assert!(result.is_some());
        let found = result.unwrap();
        assert_eq!(found.alias.as_str(), "gpt-4o-latest");
        assert_eq!(found.canonical.as_str(), "gpt-4o-2024-05-13");
    }

    #[tokio::test]
    async fn test_find_by_alias_not_found() {
        let repo = create_test_repo().await;

        let result = repo
            .find_by_alias(&ModelId::new("non-existent"), None)
            .await
            .expect("find failed");

        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_create_success() {
        let repo = create_test_repo().await;
        let alias = test_alias("my-alias", "gpt-4o");

        repo.create(alias).await.expect("create failed");

        let found = repo
            .find_by_alias(&ModelId::new("my-alias"), None)
            .await
            .expect("find failed")
            .expect("alias not found");

        assert_eq!(found.canonical.as_str(), "gpt-4o");
    }

    #[tokio::test]
    async fn test_create_duplicate() {
        let repo = create_test_repo().await;
        let alias = test_alias("duplicate", "gpt-4o");

        repo.create(alias.clone())
            .await
            .expect("first create failed");

        let result = repo.create(alias).await;

        assert!(matches!(
            result,
            Err(ModelAliasRepositoryError::AlreadyExists(_))
        ));
    }

    #[tokio::test]
    async fn test_create_alias_cycle() {
        let repo = create_test_repo().await;

        // Create first alias
        let alias1 = test_alias("alias-a", "canonical-model");
        repo.create(alias1).await.expect("create alias-a failed");

        // Try to create alias pointing to another alias
        let alias2 = test_alias("alias-b", "alias-a");
        let result = repo.create(alias2).await;

        assert!(matches!(
            result,
            Err(ModelAliasRepositoryError::InvalidAlias(_))
        ));
    }

    #[tokio::test]
    async fn test_delete_success() {
        let repo = create_test_repo().await;
        let alias = test_alias("to-delete", "gpt-4o");

        repo.create(alias).await.expect("create failed");

        let deleted = repo
            .delete(&ModelId::new("to-delete"))
            .await
            .expect("delete failed");

        assert!(deleted);

        let found = repo
            .find_by_alias(&ModelId::new("to-delete"), None)
            .await
            .expect("find failed");

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn test_delete_not_found() {
        let repo = create_test_repo().await;

        let deleted = repo
            .delete(&ModelId::new("non-existent"))
            .await
            .expect("delete failed");

        assert!(!deleted);
    }

    #[tokio::test]
    async fn test_seed_empty_table() {
        let repo = create_test_repo().await;
        let aliases = builtin_aliases();
        let count = aliases.len();

        let inserted = repo.seed(aliases).await.expect("seed failed");

        assert_eq!(inserted, count);

        let all = repo.list().await.expect("list failed");
        assert_eq!(all.len(), count);
    }

    #[tokio::test]
    async fn test_seed_idempotent() {
        let repo = create_test_repo().await;
        let aliases = builtin_aliases();
        let count = aliases.len();

        // First seed
        let inserted1 = repo.seed(aliases.clone()).await.expect("first seed failed");
        assert_eq!(inserted1, count);

        // Second seed (should be idempotent)
        let inserted2 = repo.seed(aliases).await.expect("second seed failed");
        assert_eq!(inserted2, 0); // No new inserts

        let all = repo.list().await.expect("list failed");
        assert_eq!(all.len(), count); // Still same count
    }

    #[tokio::test]
    async fn test_list_returns_all_aliases() {
        let repo = create_test_repo().await;

        repo.create(test_alias("alias-1", "model-1"))
            .await
            .expect("create 1 failed");
        repo.create(test_alias("alias-2", "model-2"))
            .await
            .expect("create 2 failed");
        repo.create(test_alias("alias-3", "model-3"))
            .await
            .expect("create 3 failed");

        let all = repo.list().await.expect("list failed");

        assert_eq!(all.len(), 3);
        assert_eq!(all[0].alias.as_str(), "alias-1"); // Sorted by alias
        assert_eq!(all[1].alias.as_str(), "alias-2");
        assert_eq!(all[2].alias.as_str(), "alias-3");
    }

    #[tokio::test]
    async fn test_builtin_aliases_count() {
        let aliases = builtin_aliases();

        // Verify we have at least 26 built-in aliases as per design
        assert!(
            aliases.len() >= 26,
            "Expected at least 26 built-in aliases, got {}",
            aliases.len()
        );

        // Verify structure
        assert!(aliases.iter().any(|a| a.alias.as_str() == "gpt-4o-latest"));
        assert!(aliases.iter().any(|a| a.alias.as_str() == "claude-opus"));
        assert!(aliases.iter().any(|a| a.alias.as_str() == "gemini-pro"));
    }
}
