// Integration tests for model alias HTTP API

use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use rook_core::{ModelAlias, ModelAliasRepositoryError, ModelAliasRepositoryPort};
use serde_json::json;
use shared_kernel::{ModelId, ProviderId};
use std::sync::Arc;
use tower::ServiceExt;
use transport_axum::alias_routes;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// In-memory alias repository for testing
#[derive(Clone)]
struct InMemoryAliasRepo {
    aliases: Arc<tokio::sync::RwLock<Vec<ModelAlias>>>,
}

impl InMemoryAliasRepo {
    fn new() -> Self {
        Self {
            aliases: Arc::new(tokio::sync::RwLock::new(Vec::new())),
        }
    }

    async fn seed_builtin(&self) {
        let builtins = vec![
            ModelAlias {
                alias: ModelId::new("gpt-4o-latest"),
                canonical: ModelId::new("gpt-4o-2024-05-13"),
                provider_id: Some(ProviderId::new("openai")),
                created_at: "2024-01-15T10:30:00Z".to_string(),
            },
            ModelAlias {
                alias: ModelId::new("claude-opus"),
                canonical: ModelId::new("claude-opus-4-5"),
                provider_id: Some(ProviderId::new("anthropic")),
                created_at: "2024-01-15T10:30:00Z".to_string(),
            },
        ];

        let mut aliases = self.aliases.write().await;
        aliases.extend(builtins);
    }
}

#[async_trait::async_trait]
impl ModelAliasRepositoryPort for InMemoryAliasRepo {
    async fn find_by_alias(
        &self,
        alias: &ModelId,
        _provider_id: Option<&ProviderId>,
    ) -> Result<Option<ModelAlias>, ModelAliasRepositoryError> {
        let aliases = self.aliases.read().await;
        Ok(aliases.iter().find(|a| a.alias == *alias).cloned())
    }

    async fn list(&self) -> Result<Vec<ModelAlias>, ModelAliasRepositoryError> {
        let aliases = self.aliases.read().await;
        Ok(aliases.clone())
    }

    async fn create(&self, alias: ModelAlias) -> Result<(), ModelAliasRepositoryError> {
        let mut aliases = self.aliases.write().await;
        if aliases.iter().any(|a| a.alias == alias.alias) {
            return Err(ModelAliasRepositoryError::AlreadyExists(alias.alias));
        }
        aliases.push(alias);
        Ok(())
    }

    async fn delete(&self, alias: &ModelId) -> Result<bool, ModelAliasRepositoryError> {
        let mut aliases = self.aliases.write().await;
        let before_len = aliases.len();
        aliases.retain(|a| a.alias != *alias);
        Ok(aliases.len() < before_len)
    }

    async fn seed(&self, builtins: Vec<ModelAlias>) -> Result<usize, ModelAliasRepositoryError> {
        let mut aliases = self.aliases.write().await;
        let mut count = 0;
        for builtin in builtins {
            if !aliases.iter().any(|a| a.alias == builtin.alias) {
                aliases.push(builtin);
                count += 1;
            }
        }
        Ok(count)
    }
}

fn test_app() -> Router {
    let repo = Arc::new(InMemoryAliasRepo::new()) as Arc<dyn ModelAliasRepositoryPort>;
    alias_routes::router(repo)
}

async fn test_app_with_seeded() -> Router {
    let repo = Arc::new(InMemoryAliasRepo::new());
    repo.seed_builtin().await;
    alias_routes::router(repo)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_get_aliases_empty() {
    let app = test_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let aliases: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(aliases.len(), 0);
}

#[tokio::test]
async fn test_get_aliases_with_builtin() {
    let app = test_app_with_seeded().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let aliases: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(aliases.len(), 2);

    // Verify structure
    assert_eq!(aliases[0]["alias"], "gpt-4o-latest");
    assert_eq!(aliases[0]["canonical"], "gpt-4o-2024-05-13");
    assert_eq!(aliases[0]["providerId"], "openai");
    assert!(aliases[0]["createdAt"].is_string());
}

#[tokio::test]
async fn test_create_alias_success() {
    let app = test_app();

    let payload = json!({
        "alias": "my-gpt4",
        "canonical": "gpt-4-0613",
        "providerId": "openai"
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn test_create_alias_duplicate() {
    let app = test_app_with_seeded().await;

    let payload = json!({
        "alias": "gpt-4o-latest",
        "canonical": "gpt-4o-2024-08-06"
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "ALIAS_ALREADY_EXISTS");
}

#[tokio::test]
async fn test_create_alias_empty_alias() {
    let app = test_app();

    let payload = json!({
        "alias": "",
        "canonical": "gpt-4-0613"
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "INVALID_ALIAS");
}

#[tokio::test]
async fn test_create_alias_empty_canonical() {
    let app = test_app();

    let payload = json!({
        "alias": "my-model",
        "canonical": ""
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "INVALID_CANONICAL");
}

#[tokio::test]
async fn test_create_alias_cycle_detection() {
    let app = test_app_with_seeded().await;

    // Try to create alias pointing to another alias
    let payload = json!({
        "alias": "my-alias",
        "canonical": "gpt-4o-latest"  // This is itself an alias
    });

    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "ALIAS_CYCLE");
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("cannot point to other aliases"));
}

#[tokio::test]
async fn test_delete_alias_success() {
    let app = test_app_with_seeded().await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/gpt-4o-latest")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_delete_alias_not_found() {
    let app = test_app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/nonexistent-alias")
                .method("DELETE")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["code"], "ALIAS_NOT_FOUND");
}

#[tokio::test]
async fn test_create_and_list() {
    let app = test_app();

    // Create alias
    let payload = json!({
        "alias": "test-alias",
        "canonical": "test-model-v1",
        "providerId": "test-provider"
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/")
                .method("POST")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&payload).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    // List aliases
    let response = app
        .oneshot(
            Request::builder()
                .uri("/")
                .method("GET")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let aliases: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(aliases.len(), 1);
    assert_eq!(aliases[0]["alias"], "test-alias");
    assert_eq!(aliases[0]["canonical"], "test-model-v1");
    assert_eq!(aliases[0]["providerId"], "test-provider");
}
