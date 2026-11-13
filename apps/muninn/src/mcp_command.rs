use std::sync::Arc;

use async_trait::async_trait;
use muninn_application::{
    LongTermMemory, SearchMessagesOptions, SearchPreferencesOptions, ShortTermMemory,
};
use muninn_core::ports::{Embedder, LongTermStore, OwnerScope, ShortTermStore};
use muninn_mcp::{Catalog, Server, ToolHandler};
use serde_json::Value;

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
        preferences: LongTermMemory::new(long_term_store, embedder, true),
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
