use rook::{config::RookConfig, di::RookContainer};
use rook_core::{ApiKeyRestrictions, CompletionRequest, ModelId, RequestId, RequestMetadata};

const COMBO_ID: &str = "550e8400-e29b-41d4-a716-446655440000";

fn config(directory: &tempfile::TempDir, default: Option<&str>, steps: bool) -> RookConfig {
    let default = default
        .map(|id| format!("default_combo = {id:?}"))
        .unwrap_or_default();
    let combos = if steps {
        format!(
            r#"
[[combos]]
id = "{COMBO_ID}"
name = "default"
[[combos.steps]]
provider_id = "blocked-provider"
model = "blocked-model"
priority = 1
"#
        )
    } else {
        String::new()
    };
    toml::from_str(&format!(
        r#"
[server]
host = "127.0.0.1"
port = 0
[routing]
strategy = "priority"
{default}
[cache]
enabled = false
ttl_secs = 60
[database]
db_path = {path:?}
[model_aliases]
enabled = false
auto_seed = false
[provider_crud]
enabled = false
{combos}
"#,
        path = directory.path().join("rook.db").to_str().unwrap(),
    ))
    .unwrap()
}

fn request() -> CompletionRequest {
    CompletionRequest {
        id: RequestId::new(),
        model: ModelId::new("requested-model"),
        messages: vec![],
        max_tokens: None,
        temperature: None,
        stream: false,
        tools: None,
        tool_choice: None,
        metadata: RequestMetadata {
            origin: "test".to_string(),
            cacheable: false,
            priority: 0,
            api_key_id: None,
            requested_tier: None,
            combo_id: None,
        },
        restrictions: ApiKeyRestrictions {
            allowed_models: vec![ModelId::new("requested-model")],
            allowed_providers: vec![],
            spending_budget: None,
        },
    }
}

#[tokio::test]
async fn configured_default_reaches_authorization_in_both_execution_paths() {
    let directory = tempfile::tempdir().unwrap();
    let config = config(&directory, Some(COMBO_ID), true);
    let container = RookContainer::build(&config).await.unwrap();
    let route = &container.usecases.route_request;
    let error = route.execute(request()).await.unwrap_err();
    assert!(error.is_forbidden(), "expected 403, got: {error}");
    let mut req = request();
    req.stream = true;
    let error = match route.execute_stream(req).await {
        Ok(_) => panic!("forbidden default must reject before streaming"),
        Err(error) => error,
    };
    assert!(error.is_forbidden(), "expected 403, got: {error}");
    container.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_or_missing_default_prevents_startup() {
    for id in ["not-a-uuid", COMBO_ID] {
        let directory = tempfile::tempdir().unwrap();
        let result = RookContainer::build(&config(&directory, Some(id), false)).await;
        match result {
            Ok(container) => {
                container.shutdown().await.unwrap();
                panic!("invalid routing.default_combo must prevent startup: {id}");
            }
            Err(error) => assert!(
                error.to_string().contains("routing.default_combo"),
                "{error}"
            ),
        }
    }
}

#[tokio::test]
async fn default_can_reference_a_persisted_combo_without_a_toml_definition() {
    let directory = tempfile::tempdir().unwrap();
    let container = RookContainer::build(&config(&directory, None, true))
        .await
        .unwrap();
    container.shutdown().await.unwrap();
    drop(container);
    let container = RookContainer::build(&config(&directory, Some(COMBO_ID), false))
        .await
        .unwrap();
    let error = container
        .usecases
        .route_request
        .execute(request())
        .await
        .unwrap_err();
    assert!(
        error.is_forbidden(),
        "persisted default was not used: {error}"
    );
    container.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_default_definition_cannot_silently_reuse_an_old_persisted_combo() {
    let directory = tempfile::tempdir().unwrap();
    let valid = config(&directory, Some(COMBO_ID), true);
    let container = RookContainer::build(&valid).await.unwrap();
    container.shutdown().await.unwrap();
    drop(container);
    let mut invalid = valid;
    invalid.combos[0].steps.clear();
    match RookContainer::build(&invalid).await {
        Ok(_) => panic!("invalid default definition must prevent startup"),
        Err(error) => assert!(
            error.to_string().contains("routing.default_combo"),
            "{error}"
        ),
    }
}
