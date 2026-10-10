# Specification: Approval numbering, header clarity, and explicit `/compact`

> **Naming provenance (2026-10-10):** See ADR-0011. Crate/binary names
> in this document (`apps/agent`, `cortex-agent`, `agent-core`,
> `agent-runtime`) are historical; live code uses
> `apps/huginn`/`huginn`/`huginn-core`/`huginn-runtime`.

## Overview

This spec defines the behavior for two user-facing changes in the `agent` CLI:

1. Each interactive approval within a single user turn is numbered, and repeats carry a `different from #N` hint.
2. The `chat` command supports `/compact` (and keeps `/summarize` as a deprecated alias) to run the existing real-model summarizer with a confirmation prompt.

The change is local to the terminal policy in `apps/agent/src/terminal.rs` and the chat loop in `apps/agent/src/main.rs`. The `agent-core` and `agent-runtime` crates do not change shape.

---

## 1. Requirements

### R1: Numbered approval prompts

**Given**: A turn in `chat` that triggers a tool with an `approval` preview.
**When**: `Policy::approve(request, cancel)` is invoked.
**Then**: The terminal prints the request number for that turn in the header line, and increments an internal counter that resets at the start of each new turn.

**Header shape**:

```
Approval for <request.action> (request #N this turn):
  Action: <request.action>
  Effect: <one-line summary derived from preview>
<preview>
Approve this exact change? [y/N]
```

When `N > 1`, the header suffix becomes `(request #N this turn, different from #<N-1>):`.

### R2: Per-turn counter lifecycle

**Given**: A turn has multiple approval requests.
**When**: The next turn starts (`chat` reads a new line from `Input`, or `Run`/`Resume` is invoked).
**Then**: The counter resets to 0 *before* the next call to `run(...)`.

The counter is held in a `Mutex<u64>` on `Policy`. The CLI calls `Policy::reset_turn()` exactly once before each new turn. `approve()` increments the counter on entry, after the `--allow` short-circuit is taken. `reset_turn()` is the only way to bring the counter back to 0.

### R3: `Effect:` line derivation

**Given**: An `ApprovalRequest` with a non-empty `preview`.
**When**: The header is rendered.
**Then**: A single `Effect:` line is included, drawn from the first non-empty line of `preview` if it starts with a known prefix (`Directory:`, `Trusted local MCP server`, `Server:`, `MCP tool`, or a unified-diff `--- ` line). Otherwise the line is omitted.

This avoids parsing the full diff and stays within the existing preview content. The diff itself is still printed in full below the header.

### R4: `/compact` command in `chat`

**Given**: A user types `/compact` (or `/summarize`) at the `agent>` prompt.
**When**: The chat loop reads the line.
**Then**: The CLI prints a confirmation prompt `Compact session now? Older history will be summarized; originals stay in the database. [y/N]`. Only a `y`/`Y`/`yes` line continues.

**On yes**: The CLI invokes the loop's real-model summarizer on the same session, persists the result, and emits the same `Event::Compacted` the automatic path emits. The terminal prints `Context compacted; original history retained.`

**On no or empty input**: The CLI prints `Compaction skipped.` and returns to `agent>` without touching the session.

`/compact` is not exposed in `Run` (single prompt, no chat). `Run` already calls the loop exactly once; the automatic threshold is the only compaction trigger there.

### R5: Reuse of existing summarization

**Given**: The loop is asked to compact now.
**When**: It runs.
**Then**: It must use the same `summary_request` already used by `StandardLoop::context()` (system prompt `Summarize repository work…`, no tools, `max_tokens=1024`, prior summary appended if present). The `Event::Compacted { through }` is recorded, `session.summary` and `session.summary_through` are updated, and the SQLite store is saved.

No new request shape. No new model call surface. No new provider path.

### R6: Approval state on denial

**Given**: A `Tool` whose `prepare()` returned an `approval` and the user denied.
**When**: The denial reaches the loop.
**Then**: The loop returns `AgentError::Tool("action denied by user; do not retry without a new user request")` for that iteration. The terminal does not block subsequent prompts; the next turn starts cleanly with a reset counter.

### R7: Backwards compatibility

**Given**: Existing callers and tests of the agent runtime and core crates.
**When**: This change is built.
**Then**: No public API of `agent-core` or `agent-runtime` changes. The only API change is the addition of `Policy::reset_turn()` and `Policy::turn_count()` on the binary-local `Policy` type. The `CompactNow` trait added to expose manual compaction is `impl`-only for `StandardLoop` and lives in the binary crate.

---

## 2. Scenarios

### Scenario 1: Single approval, no repeats

```
agent> In README.md add a line "Approved once."
Approval for native.write_file (request #1 this turn):
  Action: native.write_file
  Effect: write README.md
--- README.md    2026-10-09
+++ README.md    2026-10-09
@@
 Marea Demo
...
+Approved once.
Approve this exact change? [y/N] y
Tool: write_file
```

### Scenario 2: Denied then reformulated retry

```
agent> In README.md add a line "Approved once."
Approval for native.write_file (request #1 this turn):
  Action: native.write_file
  Effect: write README.md
<diff A>
Approve this exact change? [y/N] n
Tool failed: tool: action denied by user; do not retry without a new user request

(model reformulates and asks again)

Approval for native.write_file (request #2 this turn, different from #1):
  Action: native.write_file
  Effect: write README.md
<diff B>
Approve this exact change? [y/N]
```

### Scenario 3: `/compact` accepted

```
agent> /compact
Compact session now? Older history will be summarized; originals stay in the database. [y/N] y
Context compacted; original history retained.
agent>
```

### Scenario 4: `/compact` declined

```
agent> /compact
Compact session now? Older history will be summarized; originals stay in the database. [y/N] n
Compaction skipped.
agent>
```

### Scenario 5: `/summarize` alias

Same behavior as Scenario 3 / 4, with the alias accepted by the parser.

---

## 3. API Changes

### Terminal

```rust
// apps/agent/src/terminal.rs

pub struct Policy {
    pub input: Option<Arc<Input>>,
    pub allowed: BTreeSet<String>,
    turn_count: std::sync::Mutex<u64>,
}

impl Policy {
    pub fn reset_turn(&self) { ... }
    pub fn turn_count(&self) -> u64 { ... }
}
```

### Compact trait

```rust
// apps/agent/src/main.rs (or a new file under apps/agent/src/)

#[async_trait::async_trait]
pub trait CompactNow {
    async fn compact_now(
        &self,
        session: &mut Session,
        approvals: &dyn ApprovalPolicy,
        sink: &dyn EventSink,
        cancel: CancellationToken,
    ) -> Result<()>;
}

impl CompactNow for StandardLoop { ... }
```

The composition kernel already returns `LoopService(pub Arc<dyn AgentLoop>)`. We either (a) downcast in the CLI when the concrete type is `StandardLoop` (no trait change), or (b) keep a parallel `Arc<StandardLoop>` for the binary. We choose (b) to avoid `Any::downcast` fragility; the loop is built locally in the binary so the concrete handle is available.

### CLI command

No new `clap` subcommand. `/compact` is intercepted inside the `chat` loop, before invoking `implementation.0.run(...)`. `Run` does not expose it.

---

## 4. Configuration

No configuration change. The temporary test config that lowers `context_tokens` to 4096 still triggers automatic compaction; `/compact` complements, not replaces, the threshold.

---

## 5. Metrics and Observability

No new metrics. The existing `Event::Compacted { through }` already records the path. Whether the user invoked `/compact` or the threshold fired is visible in the session log by the absence/presence of the user message preceding it.

---

## 6. Testing Requirements

### Unit / integration (apps/agent/tests/cli.rs)

- `two_consecutive_approvals_number_them_and_deny_preserves_workspace` — drives the loop with a scripted provider that emits `write_file` twice in one turn, asserts the captured stderr contains `request #1 this turn` and `request #2 this turn, different from #1`, denies the second, and verifies the file is unchanged.
- `compact_command_summarizes_when_accepted` — uses the mock provider to send a long history, types `/compact`, accepts, and asserts a `summary` is now present in the session.
- `compact_command_skips_when_declined` — same setup, declines, asserts the session has no `summary`.

### Existing tests must continue to pass

- `cargo test -p agent-runtime --test transports` (6 tests)
- `cargo test -p agent-runtime --test coding_workflow` (27 tests)
- `cargo test -p cortex-agent --test cli` (2 tests; new tests added to this file)

---

## 7. Open Questions (Deferred)

- Per-turn approval summary line at the end of the turn.
- A `--strict` mode that re-prompts on denial instead of letting the loop surface the error to the model.
- A `/compact dry-run` that prints the projected summary size without running the model.
