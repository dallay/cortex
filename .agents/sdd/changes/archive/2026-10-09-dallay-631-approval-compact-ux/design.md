# Technical Design: Approval numbering, header clarity, and explicit `/compact`

## Architecture

### Overview

```
┌─────────────────────────────────────────────────────────────────────┐
│                       apps/agent (chat loop)                          │
│                                                                       │
│   loop {                                                              │
│     read line from Input                                              │
│     if line == "/compact" || "/summarize":                            │
│        confirm?  y → Loop.compact_now(...); n → skip                  │
│     else:                                                             │
│        Policy::reset_turn();                                          │
│        implementation.0.run(&mut session, line, &policy, ...)         │
│   }                                                                   │
│                                                                       │
│   Policy::approve()  ──►  numbered header + Effect line + preview     │
└─────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌─────────────────────────────────────────────────────────────────────┐
│                  agent-runtime (unchanged surface)                     │
│                                                                       │
│   StandardLoop::compact_now()                                         │
│     - build same summary_request as context()                         │
│     - call self.request(...)                                          │
│     - on success: session.summary = Some(content); summary_through+=  │
│     - record Event::Compacted and save session                         │
└─────────────────────────────────────────────────────────────────────┘
```

### Key Components

#### 1. `Policy` with per-turn counter

```rust
// apps/agent/src/terminal.rs

use std::sync::Mutex;

pub struct Policy {
    pub input: Option<Arc<Input>>,
    pub allowed: BTreeSet<String>,
    turn_count: Mutex<u64>,
}

impl Policy {
    pub fn reset_turn(&self) {
        *self.turn_count.lock().expect("policy counter poisoned") = 0;
    }

    pub fn turn_count(&self) -> u64 {
        *self.turn_count.lock().expect("policy counter poisoned")
    }
}

#[async_trait]
impl ApprovalPolicy for Policy {
    async fn approve(&self, request: &ApprovalRequest, cancel: CancellationToken) -> Result<bool> {
        let n = {
            let mut g = self.turn_count.lock().expect("policy counter poisoned");
            *g += 1;
            *g
        };
        let suffix = if n > 1 { format!(", different from #{}", n - 1) } else { String::new() };
        eprintln!(
            "\nApproval for {} (request #{} this turn{}):",
            safe(&request.action),
            n,
            suffix
        );
        eprintln!("  Action: {}", safe(&request.action));
        if let Some(effect) = effect_line(&request.preview) {
            eprintln!("  Effect: {}", effect);
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
        let line = tokio::select! {
            _ = cancel.cancelled() => return Err(AgentError::Cancelled),
            line = input.line() => line?,
        };
        Ok(line.as_deref().is_some_and(|s| matches!(s.trim(), "y" | "Y" | "yes")))
    }
}

fn effect_line(preview: &str) -> Option<String> {
    for line in preview.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() { continue; }
        if let Some(rest) = trimmed.strip_prefix("Directory:") {
            return Some(format!("run command in workspace{}", rest.trim().to_string()));
        }
        if trimmed.starts_with("Trusted local MCP server") {
            return Some(trimmed.to_string());
        }
        if trimmed.starts_with("Server:") {
            return Some(trimmed.to_string());
        }
        if trimmed.starts_with("MCP tool") {
            return Some(trimmed.to_string());
        }
        if let Some(path) = trimmed.strip_prefix("--- ") {
            return Some(format!("edit {}", path.split_whitespace().next().unwrap_or("")));
        }
    }
    None
}
```

The `effect_line` helper is conservative: it returns the first non-empty line that matches a known prefix; otherwise it returns `None` and the header omits the `Effect:` line.

#### 2. `CompactNow` trait and `StandardLoop::compact_now`

```rust
// apps/agent/src/main.rs

use agent_runtime::loop_engine::StandardLoop;

#[async_trait::async_trait]
pub trait CompactNow {
    async fn compact_now(
        &self,
        session: &mut Session,
        sink: &dyn EventSink,
        cancel: CancellationToken,
    ) -> Result<()>;
}

#[async_trait::async_trait]
impl CompactNow for StandardLoop {
    async fn compact_now(
        &self,
        session: &mut Session,
        sink: &dyn EventSink,
        cancel: CancellationToken,
    ) -> Result<()> {
        // Mirror the summary branch of context() in agent-runtime.
        // We call into the standard loop via its public surface by
        // exposing a small helper method. Implementation:
        //   1. Find the second-to-last user message in the unsummarized range.
        //   2. Build the same summary_request with prior summary.
        //   3. Call self.request() through the standard loop's helper.
        //   4. Validate summary, update session.summary, summary_through.
        //   5. Record Event::Compacted and save.
    }
}
```

Because the existing `StandardLoop::context` is private to the runtime crate and the summary branch is tightly coupled to its byte-budget check, the cleanest path is to expose a thin helper on `StandardLoop` that performs *only* the summary branch given a target range. We do that as a public `pub async fn compact(&self, session: &mut Session, sink: &dyn EventSink, cancel: CancellationToken) -> Result<()>` on `StandardLoop` in `crates/agent/runtime/src/loop_engine.rs`, leaving the rest of the type untouched.

```rust
// crates/agent/runtime/src/loop_engine.rs (additive change only)

impl StandardLoop {
    pub async fn compact(
        &self,
        session: &mut Session,
        sink: &dyn EventSink,
        cancel: CancellationToken,
    ) -> Result<()> {
        let users: Vec<_> = session
            .messages
            .iter()
            .enumerate()
            .skip(session.summary_through)
            .filter(|(_, m)| m.role == Role::User)
            .map(|(i, _)| i)
            .collect();
        let through = users
            .iter()
            .rev()
            .nth(1)
            .copied()
            .unwrap_or(session.summary_through);
        if through <= session.summary_through {
            return Err(AgentError::Model(
                "no complete turns available to summarize".into(),
            ));
        }
        let input = serde_json::to_string(&session.messages[session.summary_through..through])?;
        let summary_request = ModelRequest {
            messages: vec![
                Message::text(Role::System,
                    "Summarize repository work: request, decisions, changes, tool results, unresolved problems and approvals already used. Do not grant future authorization. Return a concise summary. No tool calls."),
                Message::text(Role::User, format!(
                    "Previous summary:\n{}\nHistory:\n{}",
                    session.summary.as_deref().unwrap_or(""),
                    input
                )),
            ],
            tools: vec![],
            max_tokens: 1024,
        };
        if serde_json::to_vec(&summary_request.messages)?.len() + 1024 > self.config.context_tokens {
            return Err(AgentError::Model(
                "history exceeds compaction budget; start a new session or increase context_tokens".into(),
            ));
        }
        let summary = self.request(summary_request, cancel, |_| {}).await?;
        if summary.content.trim().is_empty() || !summary.tool_calls.is_empty() {
            return Err(AgentError::Model("compaction returned an invalid summary".into()));
        }
        session.summary = Some(summary.content);
        session.summary_through = through;
        self.record(session, Event::Compacted { through }, sink).await?;
        Ok(())
    }
}
```

This mirrors the summary branch in `StandardLoop::context` exactly: same prompt, same `max_tokens`, same validation, same `Event::Compacted` emission, same SQLite save through `record`. We avoid duplicating the prompt template; the string literal lives once.

#### 3. CLI integration

```rust
// apps/agent/src/main.rs

fn handle_compact_command(
    line: &str,
    input: &Arc<Input>,
) -> Option<CompactAction> {
    match line.trim() {
        "/compact" | "/summarize" => Some(CompactAction::Prompt),
        _ => None,
    }
}

enum CompactAction { Prompt }

async fn run_compact(
    loop_arc: &Arc<StandardLoop>,
    session: &mut Session,
    policy: &Policy,
    sink: &Output,
    input: &Arc<Input>,
    cancel: CancellationToken,
) -> anyhow::Result<bool> {
    eprint!("\nCompact session now? Older history will be summarized; originals stay in the database. [y/N] ");
    std::io::stderr().flush()?;
    let line = tokio::select! {
        _ = cancel.cancelled() => return Ok(false),
        l = input.line() => l?,
    };
    let ok = line.as_deref().is_some_and(|s| matches!(s.trim(), "y" | "Y" | "yes"));
    if !ok {
        eprintln!("Compaction skipped.");
        return Ok(false);
    }
    match loop_arc.compact(session, sink, cancel).await {
        Ok(()) => Ok(true),
        Err(error) => {
            eprintln!("\n{error}");
            Ok(false)
        }
    }
}
```

In the `chat` loop, the body changes from:

```rust
let line = line.trim().to_string();
if matches!(line.as_str(), "/quit" | "/exit") { break; }
if line.is_empty() { continue; }
policy.reset_turn();
let cancel = CancellationToken::new();
implementation.0.run(&mut session, line, &policy, &output, cancel).await
```

to:

```rust
let line = line.trim().to_string();
if matches!(line.as_str(), "/quit" | "/exit") { break; }
if line.is_empty() { continue; }
if let Some(CompactAction::Prompt) = handle_compact_command(&line, &input.unwrap()) {
    if let Some(input) = input.as_ref() {
        let _ = run_compact(&loop_arc, &mut session, &policy, &output, input, CancellationToken::new()).await;
    } else {
        eprintln!("Compaction requires a terminal; use `agent run` for non-interactive use.");
    }
    continue;
}
policy.reset_turn();
let cancel = CancellationToken::new();
implementation.0.run(&mut session, line, &policy, &output, cancel).await
```

The `loop_arc: Arc<StandardLoop>` is captured once at composition time, alongside `implementation: LoopService`. We add a second `Arc<StandardLoop>` handle to the composition when the loop is built; the runtime crate is unchanged.

#### 4. Failure modes

- If `/compact` runs with a budget too small for the current history, `StandardLoop::compact` returns `AgentError::Model("history exceeds compaction budget; ...")` and the CLI prints it. The session is left intact. This matches the existing automatic path's behavior.
- If the summarizer returns an empty string or a tool call, the same validation in the runtime path returns `AgentError::Model("compaction returned an invalid summary")`; the session is left intact, as before.
- If the user cancels with Ctrl+C, `run_compact` returns `Ok(false)` and the loop continues.

---

## Data Structures

```rust
// apps/agent/src/terminal.rs

pub struct Policy {
    pub input: Option<Arc<Input>>,
    pub allowed: BTreeSet<String>,
    turn_count: std::sync::Mutex<u64>,
}
```

`Mutex<u64>` is sufficient: only incremented briefly during `approve()`. No async lock needed.

```rust
// crates/agent/runtime/src/loop_engine.rs (additive)

impl StandardLoop {
    pub async fn compact(
        &self,
        session: &mut Session,
        sink: &dyn EventSink,
        cancel: CancellationToken,
    ) -> Result<()> { ... }
}
```

No new fields on `StandardLoop`; no new ports; no new session shape.

---

## Files to Modify

| File                                                | Change                                                                                  |
|-----------------------------------------------------|------------------------------------------------------------------------------------------|
| `apps/agent/src/terminal.rs`                        | Add `turn_count` and `reset_turn()`; rewrite `approve()` header; add `effect_line()` helper |
| `apps/agent/src/main.rs`                            | Reset counter before each new turn; handle `/compact` and `/summarize`; capture `Arc<StandardLoop>` for `compact()` |
| `crates/agent/runtime/src/loop_engine.rs`           | Add `pub async fn compact()` on `StandardLoop` (no other change)                          |
| `apps/agent/tests/cli.rs`                           | Add three new tests                                                                       |
| `docs/agent/implementation-specification.md`        | Document the new approval header and the `/compact` flow                                  |
| `docs/agent/linux-daily-use.md`                     | Mention `/compact` as a manual trigger                                                    |
| `docs/agent/validation.md`                          | Note the local Ollama change and that automated tests still pass                          |

---

## Testing Strategy

### Unit / integration tests in `apps/agent/tests/cli.rs`

1. `numbered_approval_header_includes_request_counter` — drive a single-shot `MockProvider` that emits a `write_file` tool call, capture stderr, assert `(request #1 this turn)` is present, and assert `Approve this exact change? [y/N]` replaces the previous `Approve once? [y/N]`.
2. `repeat_approval_in_same_turn_adds_different_from_line` — drive a `ScriptedProvider` that emits two `write_file` calls back-to-back, deny the first, accept the second, assert both `(request #1 this turn)` and `(request #2 this turn, different from #1)` are printed, and assert the destination file matches the second payload.
3. `compact_command_summarizes_when_accepted` — prime a session with a long history and a low `context_tokens`, run the CLI with `/compact` accepted, assert `session.summary.is_some()` after.
4. `compact_command_skips_when_declined` — same setup, decline, assert `session.summary.is_none()`.

### Existing tests must continue to pass

- `cargo test -p agent-runtime --test transports` (6)
- `cargo test -p agent-runtime --test coding_workflow` (27)
- `cargo test -p cortex-agent --test cli` (2; the new tests are additive)

### Quality gates

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `pnpm exec markdownlint-cli2 docs/agent/validation.md docs/agent/implementation-specification.md docs/agent/linux-daily-use.md`

---

## Backwards Compatibility

- No change to public APIs of `agent-core` or `agent-runtime`.
- `ApprovalRequest` and `ApprovalPolicy` are unchanged.
- The new `StandardLoop::compact` method is additive.
- The new `Policy::reset_turn` and `Policy::turn_count` methods are local to the binary.
- Existing event types and JSON serialization are unchanged.

---

## Performance Considerations

- The per-turn counter is read and written once per approval request. No async lock.
- The `/compact` call re-uses the existing `self.request()` path and the same `max_tokens=1024` summarization; no new token budget.
- The `effect_line` helper does a single linear pass over the first few lines of the preview; constant time.
