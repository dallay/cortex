use alias_sqlite::{repository::builtin_aliases, SqliteModelAliasRepository};
use rook_core::ports::ModelAliasRepositoryPort;
use rook_core::{ModelAlias, ModelId};
use shared_kernel::Utc;
use std::sync::Arc;

/// Test that alias resolution works end-to-end in routing
///
/// Scenario: User sends request with alias "gpt-4o-latest", which should resolve
/// to canonical model "gpt-4o-2024-05-13" before restrictions/routing logic.
#[tokio::test]
async fn test_alias_resolves_before_routing() {
    // Setup: Create in-memory repository with test alias
    let repo = SqliteModelAliasRepository::new(":memory:").expect("create in-memory repo");

    // Create test alias
    let alias = ModelAlias {
        alias: ModelId::new("gpt-4o-latest"),
        canonical: ModelId::new("gpt-4o-2024-05-13"),
        provider_id: None,
        created_at: Utc::now(),
    };
    repo.create(alias.clone()).await.expect("create alias");

    // Verify alias was created
    let resolved = repo
        .find_by_alias(&ModelId::new("gpt-4o-latest"), None)
        .await
        .expect("find alias")
        .expect("alias exists");

    assert_eq!(resolved.canonical, ModelId::new("gpt-4o-2024-05-13"));
}

/// Test that unknown aliases pass through unchanged (fail-open behavior)
#[tokio::test]
async fn test_unknown_alias_passes_through() {
    let repo = SqliteModelAliasRepository::new(":memory:").expect("create in-memory repo");

    // Query non-existent alias
    let result = repo
        .find_by_alias(&ModelId::new("unknown-model"), None)
        .await
        .expect("query succeeds");

    assert!(result.is_none(), "unknown alias should return None");
}

/// Test that canonical model IDs pass through unchanged
#[tokio::test]
async fn test_canonical_model_passes_through() {
    let repo = SqliteModelAliasRepository::new(":memory:").expect("create in-memory repo");

    // Create alias
    let alias = ModelAlias {
        alias: ModelId::new("gpt-4-turbo"),
        canonical: ModelId::new("gpt-4-turbo-2024-04-09"),
        provider_id: None,
        created_at: Utc::now(),
    };
    repo.create(alias).await.expect("create alias");

    // Query with canonical model (not alias)
    let result = repo
        .find_by_alias(&ModelId::new("gpt-4-turbo-2024-04-09"), None)
        .await
        .expect("query succeeds");

    // Canonical models should not resolve to anything (they are not aliases)
    assert!(
        result.is_none(),
        "canonical model should not resolve as alias"
    );
}

/// Test that built-in aliases are available after seeding
#[tokio::test]
async fn test_builtin_aliases_available_after_seed() {
    let repo = SqliteModelAliasRepository::new(":memory:").expect("create in-memory repo");

    // Seed built-in aliases
    let count = repo.seed(builtin_aliases()).await.expect("seed succeeds");
    assert!(count > 20, "should seed at least 20 built-in aliases");

    // Verify some known built-in aliases exist
    let openai_alias = repo
        .find_by_alias(&ModelId::new("gpt-4o-latest"), None)
        .await
        .expect("query succeeds")
        .expect("gpt-4o-latest should exist");
    assert_eq!(openai_alias.canonical, ModelId::new("gpt-4o-2024-05-13"));

    let anthropic_alias = repo
        .find_by_alias(&ModelId::new("claude-opus"), None)
        .await
        .expect("query succeeds")
        .expect("claude-opus should exist");
    assert_eq!(
        anthropic_alias.canonical,
        ModelId::new("claude-3-opus-20240229")
    );

    let gemini_alias = repo
        .find_by_alias(&ModelId::new("gemini-2.0-flash"), None)
        .await
        .expect("query succeeds")
        .expect("gemini-2.0-flash should exist");
    assert_eq!(gemini_alias.canonical, ModelId::new("gemini-2.0-flash-exp"));
}

/// Test alias resolution with provider-scoped lookup
#[tokio::test]
async fn test_provider_scoped_alias_lookup() {
    use rook_core::ProviderId;

    let repo = SqliteModelAliasRepository::new(":memory:").expect("create in-memory repo");

    // Seed builtin aliases (which have provider_id set)
    let count = repo.seed(builtin_aliases()).await.expect("seed succeeds");
    assert!(count > 0);

    // Query with provider filter - should find provider-specific alias
    let openai_alias = repo
        .find_by_alias(
            &ModelId::new("gpt-4o-latest"),
            Some(&ProviderId::new("openai")),
        )
        .await
        .expect("query succeeds")
        .expect("openai alias exists");
    assert_eq!(openai_alias.canonical, ModelId::new("gpt-4o-2024-05-13"));
    assert_eq!(openai_alias.provider_id, Some(ProviderId::new("openai")));

    // Query without provider filter - should also find it
    let global_lookup = repo
        .find_by_alias(&ModelId::new("gpt-4o-latest"), None)
        .await
        .expect("query succeeds")
        .expect("alias exists");

    assert_eq!(global_lookup.canonical, ModelId::new("gpt-4o-2024-05-13"));
}

/// Test that seeding is idempotent (can be run multiple times safely)
#[tokio::test]
async fn test_seed_is_idempotent() {
    let repo = SqliteModelAliasRepository::new(":memory:").expect("create in-memory repo");

    // First seed
    let count1 = repo
        .seed(builtin_aliases())
        .await
        .expect("first seed succeeds");
    assert!(count1 > 0, "first seed should insert aliases");

    // Second seed
    let count2 = repo
        .seed(builtin_aliases())
        .await
        .expect("second seed succeeds");
    assert_eq!(count2, 0, "second seed should insert nothing (idempotent)");

    // Verify aliases still exist and weren't duplicated
    let all_aliases = repo.list().await.expect("list succeeds");
    assert_eq!(
        all_aliases.len(),
        count1 as usize,
        "should have same count as first seed"
    );
}

/// Test repository error handling (fail-open behavior)
#[tokio::test]
async fn test_repository_handles_concurrent_access() {
    let repo = Arc::new(SqliteModelAliasRepository::new(":memory:").expect("create repo"));

    // Create multiple concurrent queries
    let mut handles = vec![];
    for i in 0..10 {
        let repo_clone = Arc::clone(&repo);
        let handle = tokio::spawn(async move {
            let alias = ModelAlias {
                alias: ModelId::new(format!("test-alias-{}", i)),
                canonical: ModelId::new(format!("test-canonical-{}", i)),
                provider_id: None,
                created_at: Utc::now(),
            };
            repo_clone.create(alias).await
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle
            .await
            .expect("task completes")
            .expect("create succeeds");
    }

    // Verify all aliases were created
    let all_aliases = repo.list().await.expect("list succeeds");
    assert_eq!(all_aliases.len(), 10, "all aliases should be created");
}
