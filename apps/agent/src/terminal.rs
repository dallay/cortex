use agent_core::{
    AgentError, ApprovalPolicy, ApprovalRequest, CancellationToken, Event, EventSink, Result,
};
use async_trait::async_trait;
use std::{
    collections::BTreeSet,
    io::{BufRead, Write},
    sync::{Arc, Mutex},
};

pub struct Input {
    receiver: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<std::io::Result<String>>>,
}
impl Input {
    pub fn new() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        // A single reader owns stdin across prompts and approvals. A detached OS thread
        // avoids Tokio's blocking-stdin shutdown hang while waiting for the next line.
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                if sender.blocking_send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            receiver: tokio::sync::Mutex::new(receiver),
        }
    }
    pub async fn line(&self) -> Result<Option<String>> {
        self.receiver
            .lock()
            .await
            .recv()
            .await
            .transpose()
            .map_err(AgentError::Io)
    }
}
pub struct Policy {
    pub input: Option<Arc<Input>>,
    pub allowed: BTreeSet<String>,
    turn_count: Mutex<u64>,
}
impl Policy {
    pub const fn new(input: Option<Arc<Input>>, allowed: BTreeSet<String>) -> Self {
        Self {
            input,
            allowed,
            turn_count: Mutex::new(0),
        }
    }
    /// Reset the per-turn approval counter. The CLI must call this before
    /// every new turn so each turn's approvals number from #1 again.
    pub fn reset_turn(&self) {
        *self.turn_count.lock().expect("policy turn_count poisoned") = 0;
    }
    /// Inspect the current per-turn approval count. Exposed for tests and
    /// any future UI that wants to summarize a turn's approvals.
    #[allow(dead_code)]
    pub fn turn_count(&self) -> u64 {
        *self.turn_count.lock().expect("policy turn_count poisoned")
    }
}
#[async_trait]
impl ApprovalPolicy for Policy {
    async fn approve(&self, request: &ApprovalRequest, cancel: CancellationToken) -> Result<bool> {
        // Increment the per-turn counter. The counter is taken before the
        // --allow short-circuit so a denied allow-listed action still counts.
        let n = {
            let mut guard = self.turn_count.lock().expect("policy turn_count poisoned");
            *guard += 1;
            *guard
        };
        eprintln!("\n{}", approval_header(&request.action, n));
        eprintln!("  Action: {}", safe(&request.action));
        if let Some(effect) = effect_line(&request.preview) {
            // `effect` is derived from the same preview as the rest of the
            // block, so it must travel through `safe()` for consistency — a
            // path or directory with control bytes would otherwise be printed
            // verbatim here while the rest of the prompt is sanitized.
            eprintln!("  Effect: {}", safe(&effect));
        }
        eprintln!("{}", safe(&request.preview));
        if self.allowed.contains(&request.action) {
            eprintln!("Authorized by --allow for this invocation.");
            return Ok(true);
        }
        let Some(input) = &self.input else {
            eprintln!("Denied: no interactive approval or explicit action grant.");
            return Ok(false);
        };
        eprint!("Approve this exact change? [y/N] ");
        std::io::stderr().flush()?;
        let line = tokio::select! {_=cancel.cancelled()=>return Err(AgentError::Cancelled),line=input.line()=>line?};
        Ok(line
            .as_deref()
            .is_some_and(|s| matches!(s.trim(), "y" | "Y" | "yes")))
    }
}
pub struct Output {
    pub json: bool,
}
impl EventSink for Output {
    fn emit(&self, event: Event) {
        if self.json {
            if let Ok(encoded) = serde_json::to_string(&event) {
                println!("{encoded}");
            }
            return;
        }
        match event {
            Event::Text { text } => {
                print!("{}", safe(&text));
                let _ = std::io::stdout().flush();
            }
            Event::ToolStarted { call } => eprintln!("\nTool: {}", safe(&call.name)),
            Event::ToolFinished {
                is_error: true,
                output,
                ..
            } => eprintln!("\nTool failed: {}", safe(&output)),
            Event::Compacted { .. } => eprintln!("\nContext compacted; original history retained."),
            Event::TurnFinished => println!(),
            _ => {}
        }
    }
}
fn approval_header(action: &str, number: u64) -> String {
    format!(
        "Approval for {} (request #{} this turn):",
        safe(action),
        number
    )
}

/// Derive a single short `Effect:` line from the first non-empty line of
/// the preview when it starts with a known prefix. Conservative by design:
/// returning `None` is always safe (the header omits the `Effect:` line).
fn effect_line(preview: &str) -> Option<String> {
    for line in preview.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("Directory:") {
            let dir = rest.trim();
            if dir.is_empty() {
                return Some("run command in workspace".to_string());
            }
            return Some(format!("run command in workspace ({dir})"));
        }
        if trimmed.starts_with("Trusted local MCP server") {
            return Some(trimmed.to_string());
        }
        if let Some(rest) = trimmed.strip_prefix("Server:") {
            return Some(format!("start MCP server {}", rest.trim()));
        }
        if let Some(rest) = trimmed.strip_prefix("Server/action:") {
            // The first non-empty line carries the MCP server name; the
            // subsequent `Tool:` line carries the remote tool name. Surface
            // both so the user knows which exact tool the approval is for,
            // not just the configured server.
            let server = rest.trim();
            for line in preview.lines().skip(1) {
                if let Some(tool) = line.trim().strip_prefix("Tool:") {
                    let tool = tool.trim();
                    if tool.is_empty() {
                        return Some(format!("call MCP tool {server}"));
                    }
                    return Some(format!("call MCP tool {server}.{tool}"));
                }
            }
            return Some(format!("call MCP tool {server}"));
        }
        if let Some(rest) = trimmed.strip_prefix("MCP tool:") {
            return Some(format!("call MCP tool {}", rest.trim()));
        }
        if let Some(rest) = trimmed.strip_prefix("--- ") {
            let target = rest.split_whitespace().next().unwrap_or("");
            if target.is_empty() {
                return Some("edit file".to_string());
            }
            return Some(format!("edit {target}"));
        }
        if let Some(rest) = trimmed.strip_prefix("+++ ") {
            let target = rest.split_whitespace().next().unwrap_or("");
            if target.is_empty() {
                return Some("create file".to_string());
            }
            return Some(format!("create {target}"));
        }
    }
    None
}
fn safe(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{approval_header, effect_line, safe};
    use crate::terminal::Policy;
    use agent_core::{ApprovalPolicy, ApprovalRequest, CancellationToken};

    #[test]
    fn approval_header_numbers_repeated_identical_requests_without_claiming_difference() {
        let first = approval_header("native.shell", 1);
        let second = approval_header("native.shell", 2);
        assert_eq!(first, "Approval for native.shell (request #1 this turn):");
        assert_eq!(second, "Approval for native.shell (request #2 this turn):");
        assert!(!second.contains("different"));
    }

    #[test]
    fn effect_line_for_shell_preview() {
        let preview = "Directory: /tmp/work\nCommand: pwd";
        let line = effect_line(preview).expect("shell preview should produce an effect line");
        assert!(line.starts_with("run command in workspace"));
        assert!(line.contains("/tmp/work"));
    }

    #[test]
    fn effect_line_for_edit_diff_includes_target_file() {
        let preview = "--- README.md\n+++ README.md\n@@\n-old\n+new";
        let line = effect_line(preview).expect("edit preview should produce an effect line");
        assert_eq!(line, "edit README.md");
    }

    #[test]
    fn effect_line_for_create_diff() {
        let preview = "--- new.txt\n+++ new.txt\n@@\n+created";
        let line = effect_line(preview).expect("create preview should produce an effect line");
        // The diff header always starts with `---` before `+++`; the
        // editor's effect line keeps using the edit prefix to stay
        // honest about what the user is being asked to approve.
        assert_eq!(line, "edit new.txt");
    }

    #[test]
    fn effect_line_for_mcp_start() {
        let preview = "Server: fixture (trusted local process; startup may have external side effects)\nCommand: /usr/bin/mcp-server";
        let line = effect_line(preview).expect("MCP start preview should produce an effect line");
        assert_eq!(line, "start MCP server fixture (trusted local process; startup may have external side effects)");
    }

    #[test]
    fn effect_line_for_mcp_call() {
        let preview =
            "Server/action: fixture\nTool: echo\nArguments (sent to the configured MCP server; external effects depend on that server):\n{}";
        let line = effect_line(preview).expect("MCP call preview should produce an effect line");
        assert_eq!(line, "call MCP tool fixture.echo");
    }

    /// When the `Tool:` line is missing the helper must keep the historical
    /// behaviour (server name only) so older call sites do not regress.
    #[test]
    fn effect_line_for_mcp_call_without_tool_field() {
        let preview = "Server/action: fixture\nArguments (sent to the configured MCP server; external effects depend on that server):\n{}";
        let line = effect_line(preview).expect("MCP call preview should produce an effect line");
        assert_eq!(line, "call MCP tool fixture");
    }

    #[test]
    fn effect_line_returns_none_for_unknown_prefix() {
        assert!(effect_line("just some text\nno recognizable prefix").is_none());
    }

    /// RPI2-002 — the `Effect:` line must be sanitized with `safe()` exactly
    /// like the rest of the prompt, so a malicious file path or directory
    /// containing control bytes cannot inject terminal escapes into the
    /// approval header.
    #[test]
    fn safe_strips_ansi_sequences_from_effect_line() {
        let preview = "Directory: \x1b[31m/tmp/evil\x1b[0m\nCommand: pwd";
        let effect = effect_line(preview).expect("shell preview should produce an effect line");
        let sanitized = safe(&effect);
        assert!(
            !sanitized.contains('\x1b'),
            "Effect line should be free of escape sequences; got: {sanitized:?}"
        );
        // The textual content survives (with whitespace trimmed) so the user
        // still sees which path the action targets.
        assert!(sanitized.contains("/tmp/evil"));
    }

    /// RPI2-003 — the per-turn approval counter must increment for each
    /// approval within a turn and reset to 0 (i.e. the next request becomes
    /// `#1`) when `reset_turn()` is called.
    #[test]
    fn policy_approval_counter_increments_within_a_turn_and_resets() {
        // Construct the policy with no terminal input so the test does not
        // depend on stdin: the counter is taken before the input short-circuit
        // and `approve` returns `Ok(false)` early when no input is available.
        let policy = Policy::new(None, Default::default());
        // No interactions yet — the counter starts at zero.
        assert_eq!(policy.turn_count(), 0);

        let cancel = CancellationToken::new();
        let request = ApprovalRequest {
            id: "first".into(),
            action: "native.shell".into(),
            preview: "Directory: /tmp/work\nCommand: echo hi".into(),
        };
        // Drive two `approve()` calls synchronously via a oneshot runtime.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime must build");
        let first_approved = runtime.block_on(policy.approve(&request, cancel.clone()));
        assert!(first_approved.is_ok());
        let second_approved = runtime.block_on(policy.approve(&request, cancel));
        assert!(second_approved.is_ok());
        // The two calls advanced the counter to 2 within the same turn.
        assert_eq!(policy.turn_count(), 2);

        // `reset_turn()` returns the counter to 0 so the next turn starts
        // numbering again at #1.
        policy.reset_turn();
        assert_eq!(policy.turn_count(), 0);
    }
}
