mod common;
use async_trait::async_trait;
use muninn_mcp::{serve, Server, MAX_MESSAGE_BYTES};
use serde_json::{json, Value};
use std::io::Cursor;

struct NearLimitResult;

#[async_trait]
impl muninn_mcp::ToolHandler for NearLimitResult {
    async fn call(&self, _: &Value) -> Result<Value, String> {
        Ok(json!({"v":"x".repeat(muninn_mcp::MAX_RESULT_BYTES - 16)}))
    }
}

#[tokio::test]
async fn framing_handles_parse_errors_notifications_and_eof() {
    let input = format!(
        "not-json\n{{\"jsonrpc\":\"2.0\",\"method\":\"ping\"}}\n{}",
        common::request("ping", json!({}))
    );

    let mut output = Vec::new();
    serve(
        tokio::io::BufReader::new(Cursor::new(input.into_bytes())),
        &mut output,
        &mut Server::default(),
    )
    .await
    .unwrap();
    let text = String::from_utf8(output).unwrap();
    let responses: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["error"]["code"], -32700);
    assert_eq!(responses[1]["result"]["resultType"], "complete");
}

#[tokio::test]
async fn oversized_frames_stop_without_reading_the_entire_stream() {
    let input = vec![b'x'; MAX_MESSAGE_BYTES + 100];
    let mut reader = tokio::io::BufReader::new(Cursor::new(input));
    let mut output = Vec::new();
    assert!(serve(&mut reader, &mut output, &mut Server::default())
        .await
        .is_err());
    // Tokio's BufReader may read ahead by one fixed-size buffer. It must not
    // consume the remainder of the deliberately much larger input.
    assert!(reader.get_ref().position() <= (MAX_MESSAGE_BYTES + 8 * 1024 + 1) as u64);
    assert!(output.is_empty());
}

#[tokio::test]
async fn response_envelope_over_limit_is_rejected_before_any_bytes_are_written() {
    let request = common::request("tools/call", json!({"name":"memory_search","arguments":{}}));
    let input = format!("{request}\n");
    let mut output = Vec::new();
    let mut server = Server::default();
    server
        .register(
            muninn_mcp::Catalog::Core,
            "memory_search",
            Box::new(NearLimitResult),
        )
        .unwrap();

    let result = serve(
        tokio::io::BufReader::new(Cursor::new(input.into_bytes())),
        &mut output,
        &mut server,
    )
    .await;

    assert!(result.is_err());
    assert!(output.is_empty());
}

#[test]
fn process_stdout_contains_only_protocol_messages() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_muninn-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        child.stdin.take().unwrap(),
        "{}",
        common::request("ping", json!({}))
    )
    .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"]["resultType"],
        "complete"
    );
}
