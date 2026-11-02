mod common;
use muninn_mcp::{serve, Server, MAX_MESSAGE_BYTES};
use serde_json::{json, Value};
use std::io::Cursor;

#[test]
fn framing_handles_parse_errors_notifications_and_eof() {
    let input = format!(
        "not-json\n{{\"jsonrpc\":\"2.0\",\"method\":\"ping\"}}\n{}",
        common::request("ping", json!({}))
    );

    let mut output = Vec::new();
    serve(
        Cursor::new(input.as_bytes()),
        &mut output,
        &mut Server::default(),
    )
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

#[test]
fn oversized_frames_stop_without_reading_the_entire_stream() {
    let input = vec![b'x'; MAX_MESSAGE_BYTES + 100];
    let mut reader = Cursor::new(input);
    let mut output = Vec::new();
    assert!(serve(&mut reader, &mut output, &mut Server::default()).is_err());
    assert!(reader.position() <= (MAX_MESSAGE_BYTES + 1) as u64);
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
