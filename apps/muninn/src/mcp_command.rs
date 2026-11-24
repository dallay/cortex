use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use muninn_application::{
    AddEntityOptions, AddFactOptions, AddMessageOptions, AddPreferenceOptions, LongTermMemory,
    SearchEntitiesOptions, SearchMessagesOptions, SearchPreferencesOptions, ShortTermMemory,
};
use muninn_core::model::MessageRole;
use muninn_core::ports::{Embedder, LongTermStore, OwnerScope, ShortTermStore};
use muninn_mcp::{
    authorize_bearer_header, Catalog, McpHttpConfig, Server, ToolHandler, MAX_RESULT_BYTES,
};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct HostOwnerScope(String);

impl HostOwnerScope {
    pub fn parse(value: Option<&str>) -> Result<Self, &'static str> {
        let value = value.ok_or("MCP owner scope is required")?;
        let scope = OwnerScope::owned(value).map_err(|_| "MCP owner scope is invalid")?;
        Ok(Self(
            scope
                .as_str()
                .expect("owned scope has identifier")
                .to_owned(),
        ))
    }

    fn user_identifier(&self) -> &str {
        &self.0
    }
}

#[async_trait]
trait ScopedRetriever: Send + Sync {
    async fn messages(&self, query: &str, owner: &str, limit: i64) -> Result<Value, String>;
    async fn preferences(
        &self,
        query: &str,
        owner: &str,
        limit: i64,
        category: Option<&str>,
    ) -> Result<Value, String>;
}

struct ApplicationRetriever {
    messages: ShortTermMemory,
    preferences: LongTermMemory,
}

#[async_trait]
impl ScopedRetriever for ApplicationRetriever {
    async fn messages(&self, query: &str, owner: &str, limit: i64) -> Result<Value, String> {
        let options = SearchMessagesOptions {
            user_identifier: Some(owner.to_owned()),
            limit,
            ..SearchMessagesOptions::default()
        };
        let hits = self
            .messages
            .search_messages(query, &options)
            .await
            .map_err(|_| "Message retrieval failed".to_owned())?;
        serde_json::to_value(hits).map_err(|_| "Message result could not be encoded".to_owned())
    }

    async fn preferences(
        &self,
        query: &str,
        owner: &str,
        limit: i64,
        category: Option<&str>,
    ) -> Result<Value, String> {
        let options = SearchPreferencesOptions {
            user_identifier: Some(owner.to_owned()),
            limit,
            category: category.map(str::to_owned),
            ..SearchPreferencesOptions::default()
        };
        let hits = self
            .preferences
            .search_preferences(query, &options)
            .await
            .map_err(|_| "Preference retrieval failed".to_owned())?;
        serde_json::to_value(hits).map_err(|_| "Preference result could not be encoded".to_owned())
    }
}

struct MessageSearch {
    retriever: Arc<dyn ScopedRetriever>,
    scope: HostOwnerScope,
}

#[async_trait]
impl ToolHandler for MessageSearch {
    async fn call(&self, arguments: &Value) -> Result<Value, String> {
        let query = validate_query(arguments, &["query", "limit"])?;
        self.retriever
            .messages(
                query,
                self.scope.user_identifier(),
                result_limit(arguments)?,
            )
            .await
    }
}

struct PreferenceSearch {
    retriever: Arc<dyn ScopedRetriever>,
    scope: HostOwnerScope,
}

#[async_trait]
impl ToolHandler for PreferenceSearch {
    async fn call(&self, arguments: &Value) -> Result<Value, String> {
        let query = validate_query(arguments, &["query", "limit", "category"])?;
        let category = arguments
            .get("category")
            .map(|category| {
                category
                    .as_str()
                    .ok_or_else(|| "Invalid category".to_owned())
            })
            .transpose()?;
        self.retriever
            .preferences(
                query,
                self.scope.user_identifier(),
                result_limit(arguments)?,
                category,
            )
            .await
    }
}

fn validate_query<'a>(arguments: &'a Value, allowed: &[&str]) -> Result<&'a str, String> {
    let object = arguments
        .as_object()
        .ok_or_else(|| "Invalid arguments".to_owned())?;
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("Invalid arguments".to_owned());
    }
    object
        .get("query")
        .and_then(Value::as_str)
        .filter(|query| !query.trim().is_empty())
        .ok_or_else(|| "A non-empty query is required".to_owned())
}

fn result_limit(arguments: &Value) -> Result<i64, String> {
    let Some(value) = arguments.get("limit") else {
        return Ok(10);
    };
    let limit = value
        .as_i64()
        .filter(|limit| (1..=20).contains(limit))
        .ok_or_else(|| "Limit must be between 1 and 20".to_owned())?;
    Ok(limit)
}

pub fn server(
    owner: HostOwnerScope,
    short_term_store: Arc<dyn ShortTermStore>,
    long_term_store: Arc<dyn LongTermStore>,
    embedder: Option<Arc<dyn Embedder>>,
) -> Result<Server, &'static str> {
    let retriever = Arc::new(ApplicationRetriever {
        messages: ShortTermMemory::new(short_term_store, embedder.clone(), true),
        preferences: LongTermMemory::read_only(long_term_store, embedder, true),
    });
    register_tools(owner, retriever)
}

fn register_tools(
    owner: HostOwnerScope,
    retriever: Arc<dyn ScopedRetriever>,
) -> Result<Server, &'static str> {
    let mut server = Server::default();
    server.register(
        Catalog::ScopedRetrieval,
        "muninn_search_messages",
        Box::new(MessageSearch {
            retriever: retriever.clone(),
            scope: owner.clone(),
        }),
    )?;
    server.register(
        Catalog::ScopedRetrieval,
        "muninn_search_preferences",
        Box::new(PreferenceSearch {
            retriever,
            scope: owner,
        }),
    )?;
    Ok(server)
}

#[async_trait]
trait CoreMemoryOps: Send + Sync {
    async fn search(&self, owner: &str, query: &str, limit: i64) -> Result<Value, String>;
    async fn get_context(
        &self,
        owner: &str,
        session_id: Option<&str>,
        query: Option<&str>,
        limit: i64,
    ) -> Result<Value, String>;
    async fn store_message(
        &self,
        owner: &str,
        session_id: &str,
        role: MessageRole,
        content: &str,
    ) -> Result<Value, String>;
    async fn add_entity(
        &self,
        name: &str,
        entity_type: &str,
        description: Option<&str>,
    ) -> Result<Value, String>;
    async fn add_preference(
        &self,
        owner: &str,
        category: &str,
        preference: &str,
        context: Option<&str>,
    ) -> Result<Value, String>;
    async fn add_fact(&self, subject: &str, predicate: &str, object: &str)
        -> Result<Value, String>;
}

struct ApplicationCoreOps {
    short_term: ShortTermMemory,
    long_term: LongTermMemory,
}

#[async_trait]
impl CoreMemoryOps for ApplicationCoreOps {
    async fn search(&self, owner: &str, query: &str, limit: i64) -> Result<Value, String> {
        let messages = self
            .short_term
            .search_messages(
                query,
                &SearchMessagesOptions {
                    user_identifier: Some(owner.to_owned()),
                    limit,
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| "Message search failed".to_owned())?;
        let preferences = self
            .long_term
            .search_preferences(
                query,
                &SearchPreferencesOptions {
                    user_identifier: Some(owner.to_owned()),
                    limit,
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| "Preference search failed".to_owned())?;
        let entities = self
            .long_term
            .search_entities(
                query,
                &SearchEntitiesOptions {
                    limit,
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| "Entity search failed".to_owned())?;
        Ok(json!({
            "messages": messages,
            "preferences": preferences,
            "entities": entities,
        }))
    }

    async fn get_context(
        &self,
        owner: &str,
        session_id: Option<&str>,
        query: Option<&str>,
        limit: i64,
    ) -> Result<Value, String> {
        let conversation = match session_id {
            Some(sid) => Some(
                self.short_term
                    .get_conversation(sid, None, Some(owner), Some(limit))
                    .await
                    .map_err(|_| "Conversation lookup failed".to_owned())?,
            ),
            None => None,
        };
        let preferences = match query {
            Some(q) if !q.trim().is_empty() => self
                .long_term
                .search_preferences(
                    q,
                    &SearchPreferencesOptions {
                        user_identifier: Some(owner.to_owned()),
                        limit,
                        ..Default::default()
                    },
                )
                .await
                .map_err(|_| "Preference lookup failed".to_owned())?,
            _ => self
                .long_term
                .get_preferences_for(owner, true)
                .await
                .map_err(|_| "Preference lookup failed".to_owned())?,
        };
        Ok(json!({
            "conversation": conversation,
            "preferences": preferences,
        }))
    }

    async fn store_message(
        &self,
        owner: &str,
        session_id: &str,
        role: MessageRole,
        content: &str,
    ) -> Result<Value, String> {
        let stored = self
            .short_term
            .add_message(
                session_id,
                role,
                content,
                AddMessageOptions {
                    user_identifier: Some(owner.to_owned()),
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| "Storing message failed".to_owned())?;
        serde_json::to_value(stored).map_err(|_| "Message encoding failed".to_owned())
    }

    async fn add_entity(
        &self,
        name: &str,
        entity_type: &str,
        description: Option<&str>,
    ) -> Result<Value, String> {
        let (entity, _) = self
            .long_term
            .add_entity(
                name,
                entity_type,
                AddEntityOptions {
                    description: description.map(str::to_owned),
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| "Adding entity failed".to_owned())?;
        serde_json::to_value(entity).map_err(|_| "Entity encoding failed".to_owned())
    }

    async fn add_preference(
        &self,
        owner: &str,
        category: &str,
        preference: &str,
        context: Option<&str>,
    ) -> Result<Value, String> {
        let pref = self
            .long_term
            .add_preference(
                category,
                preference,
                AddPreferenceOptions {
                    context: context.map(str::to_owned),
                    user_identifier: Some(owner.to_owned()),
                    ..Default::default()
                },
            )
            .await
            .map_err(|_| "Adding preference failed".to_owned())?;
        serde_json::to_value(pref).map_err(|_| "Preference encoding failed".to_owned())
    }

    async fn add_fact(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
    ) -> Result<Value, String> {
        let fact = self
            .long_term
            .add_fact(subject, predicate, object, AddFactOptions::default())
            .await
            .map_err(|_| "Adding fact failed".to_owned())?;
        serde_json::to_value(fact).map_err(|_| "Fact encoding failed".to_owned())
    }
}

fn reject_scope_override(arguments: &Value) -> Result<&serde_json::Map<String, Value>, String> {
    let obj = arguments
        .as_object()
        .ok_or_else(|| "Invalid arguments".to_owned())?;
    if obj.contains_key("user_identifier") || obj.contains_key("owner_scope") {
        return Err("Model-supplied owner scope override is forbidden".to_owned());
    }
    Ok(obj)
}

fn required_non_empty<'a>(
    obj: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a str, String> {
    obj.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Field '{key}' is required and must be non-empty"))
}

struct CoreTool {
    kind: &'static str,
    ops: Arc<dyn CoreMemoryOps>,
    scope: HostOwnerScope,
}

#[async_trait]
impl ToolHandler for CoreTool {
    async fn call(&self, arguments: &Value) -> Result<Value, String> {
        let obj = reject_scope_override(arguments)?;
        let owner = self.scope.user_identifier();
        match self.kind {
            "memory_search" => {
                let query = required_non_empty(obj, "query")?;
                self.ops
                    .search(owner, query, result_limit(arguments)?)
                    .await
            }
            "memory_get_context" => {
                let session_id = obj.get("session_id").and_then(Value::as_str);
                let query = obj.get("query").and_then(Value::as_str);
                let limit = obj
                    .get("max_items")
                    .and_then(Value::as_i64)
                    .unwrap_or(10)
                    .clamp(1, 20);
                self.ops.get_context(owner, session_id, query, limit).await
            }
            "memory_store_message" => {
                let session_id = obj
                    .get("session_id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("default");
                let content = required_non_empty(obj, "content")?;
                let role: MessageRole = obj
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("user")
                    .parse()
                    .map_err(|_| "Invalid message role".to_owned())?;
                self.ops
                    .store_message(owner, session_id, role, content)
                    .await
            }
            "memory_add_entity" => {
                let name = required_non_empty(obj, "name")?;
                let entity_type = obj
                    .get("type")
                    .or_else(|| obj.get("entity_type"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| "Field 'type' is required".to_owned())?;
                let description = obj.get("description").and_then(Value::as_str);
                self.ops.add_entity(name, entity_type, description).await
            }
            "memory_add_preference" => {
                let category = required_non_empty(obj, "category")?;
                let preference = required_non_empty(obj, "preference")?;
                let context = obj.get("context").and_then(Value::as_str);
                self.ops
                    .add_preference(owner, category, preference, context)
                    .await
            }
            "memory_add_fact" => {
                let subject = required_non_empty(obj, "subject")?;
                let predicate = required_non_empty(obj, "predicate")?;
                let object = required_non_empty(obj, "object")?;
                self.ops.add_fact(subject, predicate, object).await
            }
            _ => Err("Unknown MCP core tool".to_owned()),
        }
    }
}

/// Build the opt-in MCP `core` profile server (6 tools) preserving host-bound immutable `OwnerScope`.
pub fn core_server(
    owner: HostOwnerScope,
    short_term_store: Arc<dyn ShortTermStore>,
    long_term_store: Arc<dyn LongTermStore>,
    embedder: Option<Arc<dyn Embedder>>,
) -> Result<Server, &'static str> {
    let long_term = LongTermMemory::new(long_term_store, embedder.clone(), true)
        .map_err(|_| "LongTermMemory requires atomic preference write support")?;
    let ops: Arc<dyn CoreMemoryOps> = Arc::new(ApplicationCoreOps {
        short_term: ShortTermMemory::new(short_term_store, embedder, true),
        long_term,
    });
    register_core_tools(owner, ops)
}

#[derive(Clone)]
struct McpHttpState {
    server: Arc<tokio::sync::Mutex<Server>>,
    config: McpHttpConfig,
    owner: HostOwnerScope,
}

/// Build the Streamable HTTP MCP Axum router with fail-closed bearer authentication.
///
/// Validates `config.validate()` upfront so non-loopback static-token configs
/// fail closed before any socket is bound.
pub fn http_router(
    server: Server,
    config: McpHttpConfig,
    owner: HostOwnerScope,
) -> Result<Router, &'static str> {
    config.validate().map_err(|err| err.public_message())?;
    let state = McpHttpState {
        server: Arc::new(tokio::sync::Mutex::new(server)),
        config,
        owner,
    };
    Ok(Router::new()
        .route("/health", get(http_health))
        .route("/mcp", post(http_mcp_dispatch))
        .layer(DefaultBodyLimit::max(MAX_RESULT_BYTES))
        .with_state(state))
}

async fn http_health() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

async fn http_mcp_dispatch(
    State(state): State<McpHttpState>,
    headers: HeaderMap,
    Json(payload): Json<Value>,
) -> Response {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if let Err(err) =
        authorize_bearer_header(auth_header, &state.config, state.owner.user_identifier()).await
    {
        let status = StatusCode::from_u16(err.status_code()).unwrap_or(StatusCode::UNAUTHORIZED);
        let body = Json(json!({ "error": err.public_message() }));
        if status == StatusCode::UNAUTHORIZED {
            return (
                status,
                [(header::WWW_AUTHENTICATE, "Bearer realm=\"muninn-mcp\"")],
                body,
            )
                .into_response();
        }
        return (status, body).into_response();
    }

    let mut server = state.server.lock().await;
    match server.handle(payload).await {
        Some(response) => (StatusCode::OK, Json(response)).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

fn register_core_tools(
    owner: HostOwnerScope,
    ops: Arc<dyn CoreMemoryOps>,
) -> Result<Server, &'static str> {
    let mut server = Server::default();
    for tool_name in [
        "memory_search",
        "memory_get_context",
        "memory_store_message",
        "memory_add_entity",
        "memory_add_preference",
        "memory_add_fact",
    ] {
        server.register(
            Catalog::Core,
            tool_name,
            Box::new(CoreTool {
                kind: tool_name,
                ops: ops.clone(),
                scope: owner.clone(),
            }),
        )?;
    }
    Ok(server)
}

#[cfg(test)]
fn test_server(
    owner: HostOwnerScope,
    retriever: Arc<dyn ScopedRetriever>,
) -> Result<Server, &'static str> {
    register_tools(owner, retriever)
}

#[cfg(test)]
mod tests {
    use super::{result_limit, validate_query, HostOwnerScope, ScopedRetriever};
    use async_trait::async_trait;
    use muninn_mcp::PROTOCOL_VERSION;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[test]
    fn host_owner_scope_rejects_missing_and_blank_values() {
        assert!(HostOwnerScope::parse(None).is_err());
        assert!(HostOwnerScope::parse(Some(" \t ")).is_err());
        assert_eq!(
            HostOwnerScope::parse(Some("owner-a"))
                .unwrap()
                .user_identifier(),
            "owner-a"
        );
    }

    #[test]
    fn retrieval_inputs_reject_model_supplied_scope_and_unbounded_limits() {
        assert!(validate_query(
            &json!({"query":"x", "user_identifier":"owner-b"}),
            &["query", "limit"]
        )
        .is_err());
        assert!(result_limit(&json!({"limit": 21})).is_err());
        assert_eq!(result_limit(&json!({"limit": 20})).unwrap(), 20);
    }

    struct OwnerAwareFake(Mutex<Vec<(String, String)>>);

    #[async_trait]
    impl ScopedRetriever for OwnerAwareFake {
        async fn messages(
            &self,
            query: &str,
            owner: &str,
            _: i64,
        ) -> Result<serde_json::Value, String> {
            self.0
                .lock()
                .unwrap()
                .push(("messages".into(), owner.into()));
            Ok(json!([{"owner":owner,"query":query}]))
        }

        async fn preferences(
            &self,
            query: &str,
            owner: &str,
            _: i64,
            _: Option<&str>,
        ) -> Result<serde_json::Value, String> {
            self.0
                .lock()
                .unwrap()
                .push(("preferences".into(), owner.into()));
            Ok(json!([{"owner":owner,"query":query}]))
        }
    }

    fn call(method: &str, tool: &str, arguments: serde_json::Value) -> serde_json::Value {
        json!({"jsonrpc":"2.0", "id":1, "method":method, "params":{
            "_meta":{"io.modelcontextprotocol/protocolVersion":PROTOCOL_VERSION,
                "io.modelcontextprotocol/clientCapabilities":{},
                "io.modelcontextprotocol/clientInfo":{"name":"test","version":"1"}},
            "name":tool,"arguments":arguments
        }})
    }

    #[tokio::test]
    async fn both_retrieval_handlers_use_only_the_fixed_host_owner() {
        let fake = Arc::new(OwnerAwareFake(Mutex::new(Vec::new())));
        let mut server = super::test_server(
            HostOwnerScope::parse(Some("owner-a")).unwrap(),
            fake.clone(),
        )
        .unwrap();
        for tool in ["muninn_search_messages", "muninn_search_preferences"] {
            let response = server
                .handle(call("tools/call", tool, json!({"query":"x"})))
                .await
                .unwrap();
            let value: serde_json::Value =
                serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(value[0]["owner"], "owner-a");
            assert_eq!(
                server
                    .handle(call(
                        "tools/call",
                        tool,
                        json!({"query":"x", "user_identifier":"owner-b"})
                    ))
                    .await
                    .unwrap()["result"]["isError"],
                true
            );
        }
        assert_eq!(
            *fake.0.lock().unwrap(),
            [
                ("messages".into(), "owner-a".into()),
                ("preferences".into(), "owner-a".into())
            ]
        );
    }

    #[tokio::test]
    async fn command_catalog_contains_only_the_two_read_only_tools() {
        let fake = Arc::new(OwnerAwareFake(Mutex::new(Vec::new())));
        let mut server =
            super::test_server(HostOwnerScope::parse(Some("owner-a")).unwrap(), fake).unwrap();
        let response = server
            .handle(call("tools/list", "", json!({})))
            .await
            .unwrap();
        let names = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["muninn_search_messages", "muninn_search_preferences"]
        );
    }

    struct FakeCoreOps(Mutex<Vec<(String, String)>>);

    #[async_trait]
    impl super::CoreMemoryOps for FakeCoreOps {
        async fn search(
            &self,
            owner: &str,
            query: &str,
            _: i64,
        ) -> Result<serde_json::Value, String> {
            self.0.lock().unwrap().push(("search".into(), owner.into()));
            Ok(json!({"owner": owner, "query": query}))
        }
        async fn get_context(
            &self,
            owner: &str,
            _: Option<&str>,
            _: Option<&str>,
            _: i64,
        ) -> Result<serde_json::Value, String> {
            self.0
                .lock()
                .unwrap()
                .push(("context".into(), owner.into()));
            Ok(json!({"owner": owner}))
        }
        async fn store_message(
            &self,
            owner: &str,
            _: &str,
            _: muninn_core::model::MessageRole,
            _: &str,
        ) -> Result<serde_json::Value, String> {
            self.0.lock().unwrap().push(("store".into(), owner.into()));
            Ok(json!({"owner": owner}))
        }
        async fn add_entity(
            &self,
            name: &str,
            _: &str,
            _: Option<&str>,
        ) -> Result<serde_json::Value, String> {
            self.0.lock().unwrap().push(("entity".into(), name.into()));
            Ok(json!({"name": name}))
        }
        async fn add_preference(
            &self,
            owner: &str,
            _: &str,
            _: &str,
            _: Option<&str>,
        ) -> Result<serde_json::Value, String> {
            self.0
                .lock()
                .unwrap()
                .push(("preference".into(), owner.into()));
            Ok(json!({"owner": owner}))
        }
        async fn add_fact(
            &self,
            subject: &str,
            _: &str,
            _: &str,
        ) -> Result<serde_json::Value, String> {
            self.0.lock().unwrap().push(("fact".into(), subject.into()));
            Ok(json!({"subject": subject}))
        }
    }

    #[tokio::test]
    async fn core_profile_registers_six_tools_and_blocks_scope_override() {
        let fake = Arc::new(FakeCoreOps(Mutex::new(Vec::new())));
        let mut server = super::register_core_tools(
            HostOwnerScope::parse(Some("owner-a")).unwrap(),
            fake.clone(),
        )
        .unwrap();
        let list = server
            .handle(call("tools/list", "", json!({})))
            .await
            .unwrap();
        let names: Vec<_> = list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), 6);
        for expected in [
            "memory_search",
            "memory_get_context",
            "memory_store_message",
            "memory_add_entity",
            "memory_add_preference",
            "memory_add_fact",
        ] {
            assert!(names.contains(&expected), "missing core tool {expected}");
        }

        // Cross-tenant override attempt is rejected fail-closed.
        let rejected = server
            .handle(call(
                "tools/call",
                "memory_store_message",
                json!({"content": "hi", "user_identifier": "owner-b"}),
            ))
            .await
            .unwrap();
        assert_eq!(rejected["result"]["isError"], true);

        // Authorized call binds host owner.
        let ok = server
            .handle(call(
                "tools/call",
                "memory_store_message",
                json!({"content": "hi"}),
            ))
            .await
            .unwrap();
        assert_ne!(ok["result"]["isError"], true);
        assert_eq!(
            *fake.0.lock().unwrap(),
            vec![("store".into(), "owner-a".into())]
        );
    }

    #[tokio::test]
    async fn streamable_http_enforces_bearer_auth_and_rejects_non_loopback_static_token() {
        use axum::http::{header, StatusCode};
        use muninn_mcp::{McpHttpAuthMode, McpHttpConfig, RedactedToken};
        use std::net::{IpAddr, Ipv4Addr, SocketAddr};

        let owner = HostOwnerScope::parse(Some("owner-a")).unwrap();
        let token = RedactedToken::new("local-dev-bearer-token-1234").unwrap();

        // Non-loopback static token fails closed before creating router.
        let fake = Arc::new(FakeCoreOps(Mutex::new(Vec::new())));
        let server = super::register_core_tools(owner.clone(), fake.clone()).unwrap();
        let non_loopback = McpHttpConfig {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 8765),
            auth: McpHttpAuthMode::LoopbackStaticToken {
                token: token.clone(),
            },
        };
        assert!(super::http_router(server, non_loopback, owner.clone()).is_err());

        // Loopback static token router validates cleanly and handler enforces bearer auth.
        let server = super::register_core_tools(owner.clone(), fake).unwrap();
        let loopback = McpHttpConfig {
            bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8765),
            auth: McpHttpAuthMode::LoopbackStaticToken { token },
        };
        assert!(super::http_router(server, loopback.clone(), owner.clone()).is_ok());

        let fake2 = Arc::new(FakeCoreOps(Mutex::new(Vec::new())));
        let state = super::McpHttpState {
            server: Arc::new(tokio::sync::Mutex::new(
                super::register_core_tools(owner.clone(), fake2).unwrap(),
            )),
            config: loopback,
            owner,
        };

        // Missing Authorization -> 401 with WWW-Authenticate: Bearer.
        let unauth_resp = super::http_mcp_dispatch(
            axum::extract::State(state.clone()),
            axum::http::HeaderMap::new(),
            axum::Json(call("tools/list", "", json!({}))),
        )
        .await;
        assert_eq!(unauth_resp.status(), StatusCode::UNAUTHORIZED);
        assert!(unauth_resp.headers().contains_key(header::WWW_AUTHENTICATE));

        // Valid Authorization -> 200 OK.
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            "Bearer local-dev-bearer-token-1234".parse().unwrap(),
        );
        let auth_resp = super::http_mcp_dispatch(
            axum::extract::State(state),
            headers,
            axum::Json(call("tools/list", "", json!({}))),
        )
        .await;
        assert_eq!(auth_resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn rmcp_client_initializes_lists_tools_and_calls_scoped_retrieval_over_stdio_io() {
        use rmcp::{
            model::{CallToolRequestParams, ContentBlock},
            ServiceExt,
        };

        let fake = Arc::new(OwnerAwareFake(Mutex::new(Vec::new())));
        let mut server = super::test_server(
            HostOwnerScope::parse(Some("owner-a")).unwrap(),
            fake.clone(),
        )
        .unwrap();
        let (client_io, server_io) = tokio::io::duplex(64 * 1024);
        let (server_read, server_write) = tokio::io::split(server_io);
        let server_task = tokio::spawn(async move {
            muninn_mcp::serve(
                tokio::io::BufReader::new(server_read),
                server_write,
                &mut server,
            )
            .await
        });

        let client = ().serve(client_io).await.unwrap();
        let tools = client.peer().list_tools(None).await.unwrap();
        let names = tools
            .tools
            .iter()
            .map(|tool| tool.name.to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            ["muninn_search_messages", "muninn_search_preferences"]
        );

        for name in names {
            let result = client
                .peer()
                .call_tool(
                    CallToolRequestParams::new(name.clone()).with_arguments(
                        json!({"query":"favorite color"})
                            .as_object()
                            .unwrap()
                            .clone(),
                    ),
                )
                .await
                .unwrap();
            let ContentBlock::Text(text) = &result.content[0] else {
                panic!("expected text tool output")
            };
            let value: serde_json::Value = serde_json::from_str(&text.text).unwrap();
            assert_eq!(value[0]["owner"], "owner-a");
            assert_eq!(result.is_error, Some(false));

            let denied = client
                .peer()
                .call_tool(
                    CallToolRequestParams::new(name).with_arguments(
                        json!({
                            "query":"favorite color",
                            "user_identifier":"owner-b"
                        })
                        .as_object()
                        .unwrap()
                        .clone(),
                    ),
                )
                .await
                .unwrap();
            assert_eq!(denied.is_error, Some(true));
        }
        assert_eq!(
            *fake.0.lock().unwrap(),
            [
                ("messages".into(), "owner-a".into()),
                ("preferences".into(), "owner-a".into())
            ]
        );

        drop(client);
        server_task.await.unwrap().unwrap();
    }
}
