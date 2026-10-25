//! Opt-in real-server probe; never part of deterministic tests and never pulls models.
use futures::StreamExt;
use huginn_core::{
    connections::{ConnectionMaterial, ConnectionSettings},
    providers::{ConnectionIdentity, ModelSelection, ProviderAdapter, ProviderRegistration},
    CancellationToken, Message, ModelDelta, ModelProvider, ModelRequest, Role, ToolDefinition,
};
use huginn_runtime::providers::InferenceAdapter;
use serde_json::json;
use tokio::fs;

async fn collect(
    model: &dyn ModelProvider,
    request: ModelRequest,
) -> huginn_core::Result<Vec<ModelDelta>> {
    let mut stream = model.stream(request, CancellationToken::new()).await?;
    let mut deltas = vec![];
    while let Some(delta) = stream.next().await {
        deltas.push(delta?);
    }
    if !matches!(deltas.last(), Some(ModelDelta::Finished)) {
        return Err(huginn_core::AgentError::Model(
            "probe missing completion".into(),
        ));
    }
    Ok(deltas)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model = std::env::args()
        .nth(1)
        .ok_or("usage: verify_ollama <installed-model> [root-endpoint]")?;
    let endpoint = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "http://127.0.0.1:11434".into());
    let adapter = InferenceAdapter::ollama(180);
    let material = ConnectionMaterial {
        settings: ConnectionSettings {
            provider: "ollama".into(),
            endpoint: Some(endpoint),
            model: model.clone(),
            requires_secret: false,
            allow_loopback_http: true,
        },
        secret: None,
    };
    let selection = ModelSelection {
        connection: ConnectionIdentity {
            id: "local-verification".into(),
            adapter: ProviderRegistration::new("ollama".into(), 1),
            revision: 1,
        },
        model,
    };
    let model = adapter
        .create_connection_model(&selection, material, CancellationToken::new())
        .await?;
    let system = Message::text(
        Role::System,
        "You are a concise coding agent. Do not reason aloud. /no_think",
    );
    let text = collect(
        model.as_ref(),
        ModelRequest {
            messages: vec![
                system.clone(),
                Message::text(Role::User, "Reply with LOCAL_OK. /no_think"),
            ],
            tools: vec![],
            max_tokens: 1024,
        },
    )
    .await?;
    let text = text
        .iter()
        .filter_map(|d| {
            if let ModelDelta::Text(t) = d {
                Some(t.as_str())
            } else {
                None
            }
        })
        .collect::<String>();
    if !text.contains("LOCAL_OK") {
        return Err("probe text assertion failed".into());
    }
    println!("PASS streamed text and completion");
    let tools = vec![ToolDefinition {
        name: "list_files".into(),
        description: "List files in the current workspace. Use this tool to inspect actual files."
            .into(),
        input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
    }];
    let mut messages = vec![
        system,
        Message::text(
            Role::User,
            "Call list_files now to inspect the workspace. Do not guess its contents. /no_think",
        ),
    ];
    let deltas = collect(
        model.as_ref(),
        ModelRequest {
            messages: messages.clone(),
            tools: tools.clone(),
            max_tokens: 2048,
        },
    )
    .await?;
    let calls = deltas
        .iter()
        .filter_map(|d| {
            if let ModelDelta::ToolCall(call) = d {
                Some(call.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if calls.len() != 1 || calls[0].name != "list_files" || !calls[0].arguments.is_object() {
        return Err("probe expected one valid list_files tool call".into());
    }
    println!("PASS streamed tool call and JSON arguments");
    let workspace = tempfile::tempdir()?;
    fs::write(
        workspace.path().join("verified_fixture.rs"),
        "// local verification fixture\n",
    )
    .await?;
    let mut entries = fs::read_dir(workspace.path()).await?;
    let mut files = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        files.push(entry.file_name().to_string_lossy().into_owned());
    }
    let mut assistant = Message::text(Role::Assistant, "");
    assistant.tool_calls = calls.clone();
    messages.push(assistant);
    messages.push(Message::tool(
        calls[0].id.clone(),
        json!({"files":files}).to_string(),
    ));
    let result = collect(
        model.as_ref(),
        ModelRequest {
            messages,
            tools: vec![],
            max_tokens: 2048,
        },
    )
    .await?;
    let text = result
        .iter()
        .filter_map(|d| {
            if let ModelDelta::Text(t) = d {
                Some(t.as_str())
            } else {
                None
            }
        })
        .collect::<String>();
    if !text.contains("verified_fixture.rs") {
        return Err("probe continuation did not use the tool result".into());
    }
    println!("PASS actual local read-only tool execution and result continuation");
    Ok(())
}
