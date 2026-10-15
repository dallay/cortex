//! Task 1 GREEN: combo_aliases persistence (V7) — ComboAlias entity.

use combo_sqlite::{ComboAlias, ComboAliasRepositoryPort, ComboSqliteRepository};
use rook_core::ports::ComboRepositoryPort;
use rook_core::{Combo, ComboStep, ComboStrategy};
use shared_kernel::{ComboId, ModelId, ProviderId};

fn test_combo(name: &str) -> Combo {
    Combo::new(
        name.to_string(),
        ComboStrategy::Priority,
        vec![ComboStep {
            provider_id: ProviderId::new("openai"),
            model: ModelId::new("gpt-4o"),
            connection_id: None,
            priority: 1,
        }],
    )
}

async fn setup() -> ComboSqliteRepository {
    ComboSqliteRepository::new(":memory:").expect("create :memory: repo with migrations")
}

async fn create_combo(repo: &ComboSqliteRepository, name: &str) -> Combo {
    let combo = test_combo(name);
    ComboRepositoryPort::create(repo, &combo)
        .await
        .expect("create combo");
    combo
}

async fn create_alias(
    repo: &ComboSqliteRepository,
    alias: &str,
    combo_id: ComboId,
) -> Result<(), combo_sqlite::ComboAliasRepositoryError> {
    ComboAliasRepositoryPort::create(repo, ComboAlias::new(ModelId::new(alias), combo_id)).await
}

#[tokio::test]
async fn create_and_find_by_alias() {
    let repo = setup().await;
    let combo = create_combo(&repo, "coding-combo").await;
    create_alias(&repo, "rook/coding", combo.id)
        .await
        .expect("create alias");
    let found: Option<ComboAlias> = repo
        .find_by_alias(&ModelId::new("rook/coding"))
        .await
        .expect("find");
    assert!(found.is_some());
    assert_eq!(found.unwrap().combo_id, combo.id);
}

#[tokio::test]
async fn find_missing_alias_returns_none() {
    let repo = setup().await;
    let found: Option<ComboAlias> = repo
        .find_by_alias(&ModelId::new("rook/unknown"))
        .await
        .expect("find");
    assert!(found.is_none());
}

#[tokio::test]
async fn rejects_alias_without_prefix() {
    let repo = setup().await;
    let combo = create_combo(&repo, "c1").await;
    let err = create_alias(&repo, "coding", combo.id).await.unwrap_err();
    assert!(matches!(
        err,
        combo_sqlite::ComboAliasRepositoryError::InvalidAlias(_)
    ));
}

#[tokio::test]
async fn rejects_empty_rook_name() {
    let repo = setup().await;
    let combo = create_combo(&repo, "c1").await;
    let err = create_alias(&repo, "rook/", combo.id).await.unwrap_err();
    assert!(matches!(
        err,
        combo_sqlite::ComboAliasRepositoryError::InvalidAlias(_)
    ));
}

#[tokio::test]
async fn rejects_uppercase_alias() {
    let repo = setup().await;
    let combo = create_combo(&repo, "c1").await;
    let err = create_alias(&repo, "rook/Coding", combo.id)
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        combo_sqlite::ComboAliasRepositoryError::InvalidAlias(_)
    ));
}

#[tokio::test]
async fn rejects_too_long_alias() {
    let repo = setup().await;
    let combo = create_combo(&repo, "c1").await;
    let long = format!("rook/{}", "a".repeat(65));
    let err = create_alias(&repo, &long, combo.id).await.unwrap_err();
    assert!(matches!(
        err,
        combo_sqlite::ComboAliasRepositoryError::InvalidAlias(_)
    ));
}

#[tokio::test]
async fn rejects_duplicate_alias() {
    let repo = setup().await;
    let c1 = create_combo(&repo, "c1").await;
    let c2 = create_combo(&repo, "c2").await;
    create_alias(&repo, "rook/dup", c1.id).await.expect("first");
    let err = create_alias(&repo, "rook/dup", c2.id).await.unwrap_err();
    assert!(matches!(
        err,
        combo_sqlite::ComboAliasRepositoryError::AlreadyExists(_)
    ));
}

#[tokio::test]
async fn rejects_alias_for_missing_combo() {
    let repo = setup().await;
    let err = create_alias(&repo, "rook/ghost", ComboId::new())
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        combo_sqlite::ComboAliasRepositoryError::ComboNotFound(_)
    ));
}

#[tokio::test]
async fn delete_alias_returns_true_then_false() {
    let repo = setup().await;
    let combo = create_combo(&repo, "c1").await;
    create_alias(&repo, "rook/temp", combo.id)
        .await
        .expect("create");
    assert!(
        ComboAliasRepositoryPort::delete(&repo, &ModelId::new("rook/temp"))
            .await
            .expect("delete")
    );
    assert!(
        !ComboAliasRepositoryPort::delete(&repo, &ModelId::new("rook/temp"))
            .await
            .expect("delete again")
    );
}

#[tokio::test]
async fn delete_combo_cascades_alias() {
    let repo = setup().await;
    let combo = create_combo(&repo, "c1").await;
    create_alias(&repo, "rook/coding", combo.id)
        .await
        .expect("alias");
    ComboRepositoryPort::delete(&repo, &combo.id)
        .await
        .expect("delete combo");
    let found: Option<ComboAlias> = repo
        .find_by_alias(&ModelId::new("rook/coding"))
        .await
        .expect("find");
    assert!(found.is_none());
}

#[tokio::test]
async fn list_aliases_ordered() {
    let repo = setup().await;
    let combo = create_combo(&repo, "c1").await;
    for a in ["rook/zulu", "rook/alpha", "rook/mid"] {
        create_alias(&repo, a, combo.id).await.expect("create");
    }
    let list = ComboAliasRepositoryPort::list(&repo).await.expect("list");
    let names: Vec<_> = list.iter().map(|a| a.alias.to_string()).collect();
    assert_eq!(names, vec!["rook/alpha", "rook/mid", "rook/zulu"]);
}

#[tokio::test]
async fn migration_table_exists_via_migration() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("v7.db");
    let db_str = db_path.to_str().unwrap();
    let mut conn = rusqlite::Connection::open(db_str).expect("open");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("pragma");
    db_migration::run_on_connection(&mut conn).expect("migrate");
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='combo_aliases'",
            [],
            |r| r.get(0),
        )
        .expect("query master");
    assert_eq!(count, 1);
    let fk: i64 = conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .expect("fk pragma");
    assert_eq!(fk, 1, "FK enforcement must be ON per connection");
}
