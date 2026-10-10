# Tasks: Approval numbering, header clarity, and explicit `/compact`

## Implementation Phases

---

## Phase 1: Terminal — numbering and headers

### Task 1.1: Add per-turn counter to `Policy`

**File**: `apps/agent/src/terminal.rs`

**Steps**:

- [ ] Add `turn_count: std::sync::Mutex<u64>` to `Policy`.
- [ ] Add `Policy::reset_turn()` and `Policy::turn_count()`.
- [ ] Construct `Policy` in `apps/agent/src/main.rs` with `turn_count: Mutex::new(0)`.

**Verification**: `cargo build -p cortex-agent` succeeds.

### Task 1.2: Rewrite `Policy::approve()` header

**File**: `apps/agent/src/terminal.rs`

**Steps**:

- [ ] Increment `turn_count` on entry to `approve()` (after the `--allow` short-circuit is evaluated against the request).
- [ ] Render the header as `Approval for <action> (request #N this turn[, different from #<N-1>]):`.
- [ ] Add a single `Action: <action>` line and a conditional `Effect: <effect_line>` line above the preview.
- [ ] Change the prompt text from `Approve once? [y/N]` to `Approve this exact change? [y/N]`.

**Verification**: New test in `apps/agent/tests/cli.rs` (`numbered_approval_header_includes_request_counter`).

### Task 1.3: Implement `effect_line()` helper

**File**: `apps/agent/src/terminal.rs`

**Steps**:

- [ ] Return the first non-empty line that starts with one of: `Directory:`, `Trusted local MCP server`, `Server:`, `MCP tool`, or `--- `.
- [ ] For `Directory:` derive `run command in workspace` plus the directory path.
- [ ] For `--- ` derive `edit <file>`.
- [ ] Return `None` if no known prefix matches; the header then omits the `Effect:` line.

**Verification**: New test asserts that a `native.shell` preview produces an `Effect: run command in workspace …` line, and a `native.write_file` diff preview produces `Effect: write <file>`.

### Task 1.4: Reset counter on every new turn in the CLI

**File**: `apps/agent/src/main.rs`

**Steps**:

- [ ] In the `chat` loop, call `policy.reset_turn()` before invoking `implementation.0.run(...)` for a regular prompt.
- [ ] In `Run`, call `policy.reset_turn()` before invoking the loop.
- [ ] In `Resume`, the chat loop's reset already covers the resumed session; no extra call needed.

**Verification**: Existing tests continue to pass; new repeat-approval test shows counter increments across the same turn and resets at the next prompt.

---

## Phase 2: Runtime — expose `StandardLoop::compact`

### Task 2.1: Add public `compact` method to `StandardLoop`

**File**: `crates/agent/runtime/src/loop_engine.rs`

**Steps**:

- [ ] Add `pub async fn compact(&self, session: &mut Session, sink: &dyn EventSink, cancel: CancellationToken) -> Result<()>`.
- [ ] Mirror the summary branch of `context()` exactly: same `users` enumeration, same `through` selection, same `summary_request`, same validation, same `Event::Compacted { through }` emission, same `record` save.
- [ ] Refuse to run with `Err(AgentError::Model("no complete turns available to summarize"))` if no unsummarized user turn exists.
- [ ] Refuse to run with the existing `Err(AgentError::Model("history exceeds compaction budget…"))` if the request would exceed `context_tokens`.

**Verification**: Existing `failed_compaction_preserves_original_history_and_stops` and `compaction_preserves_history_and_tool_pairs` continue to pass; the new behavior is exercised by the CLI test.

---

## Phase 3: CLI — `/compact` and `/summarize` commands

### Task 3.1: Capture an `Arc<StandardLoop>` handle in `main`

**File**: `apps/agent/src/main.rs`

**Steps**:

- [ ] When building the composition kernel, also keep `let loop_arc: Arc<StandardLoop> = Arc::new(StandardLoop { model: model.clone(), tools: registry.clone(), sessions: sessions.clone(), config: LoopConfig { ... }, lifecycle: startup.clone() });`.
- [ ] Pass `loop_arc` into the loop body's closures.
- [ ] Register the existing `loop_id` service using the same handle (avoid double-allocating the loop state).

**Verification**: `cargo build -p cortex-agent` succeeds; existing CLI tests continue to pass.

### Task 3.2: Handle `/compact` in the `chat` loop

**File**: `apps/agent/src/main.rs`

**Steps**:

- [ ] When the trimmed line equals `/compact` or `/summarize`, prompt for confirmation with `Compact session now? Older history will be summarized; originals stay in the database. [y/N]`.
- [ ] Read the confirmation through the same `Input` reader; on `y/Y/yes`, call `loop_arc.compact(&mut session, &sink, cancel)`; on anything else, print `Compaction skipped.` and return to `agent>`.
- [ ] If `Input` is `None` (no terminal), print `Compaction requires a terminal; use \`agent run\` for non-interactive use.` and continue.
- [ ] On `compact()` error, print the error and continue; the session is left intact.

**Verification**: New tests `compact_command_summarizes_when_accepted` and `compact_command_skips_when_declined` pass.

### Task 3.3: Update `Run` to ignore `/compact`

**File**: `apps/agent/src/main.rs`

**Steps**:

- [ ] In `Run`, `/compact` is not a special token; the prompt is passed to the loop, which already treats it as plain text. Document this in the change's proposal.

**Verification**: Existing `json_read_only_turn_lists_and_resumes_a_session` test continues to pass.

---

## Phase 4: Tests

### Task 4.1: Terminal header tests in `apps/agent/tests/cli.rs`

**File**: `apps/agent/tests/cli.rs`

**Steps**:

- [ ] `numbered_approval_header_includes_request_counter` — capture stderr, assert `request #1 this turn` and `Approve this exact change?`.
- [ ] `repeat_approval_in_same_turn_adds_different_from_line` — script a provider with two `write_file` calls, assert both headers and the final file content equals the second payload.
- [ ] `effect_line_for_shell_includes_directory` — script a `native.shell` call, assert `Effect: run command in workspace` appears.
- [ ] `effect_line_for_write_file_includes_file_path` — script a `native.write_file` call, assert `Effect: write <file>` appears.

### Task 4.2: `/compact` tests in `apps/agent/tests/cli.rs`

**File**: `apps/agent/tests/cli.rs`

**Steps**:

- [ ] `compact_command_summarizes_when_accepted` — drive a session through one or more mock turns, type `/compact`, accept, assert `session.summary.is_some()`.
- [ ] `compact_command_skips_when_declined` — same setup, decline, assert `session.summary.is_none()`.

### Task 4.3: Regression sweep

**Commands**:

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test -p agent-runtime --test transports`
- [ ] `cargo test -p agent-runtime --test coding_workflow`
- [ ] `cargo test -p cortex-agent --test cli`
- [ ] `pnpm exec markdownlint-cli2 docs/agent/validation.md docs/agent/implementation-specification.md docs/agent/linux-daily-use.md`

---

## Phase 5: Documentation

### Task 5.1: Update `implementation-specification.md`

**File**: `docs/agent/implementation-specification.md`

**Steps**:

- [ ] Update the approval UX paragraph to include the numbered header and the `Effect:` line.
- [ ] Document `/compact` as a manual trigger for the same summarization path used by the automatic threshold.

### Task 5.2: Update `linux-daily-use.md`

**File**: `docs/agent/linux-daily-use.md`

**Steps**:

- [ ] Mention `/compact` as a manual trigger in the `Context` step.

### Task 5.3: Update `validation.md`

**File**: `docs/agent/validation.md`

**Steps**:

- [ ] Note the new tests, the new manual trigger, and that the local Ollama acceptance run still applies.

---

## Dependency Graph

```
Task 1.1 ──► Task 1.2 ──► Task 1.3 ──► Task 1.4 ──► Task 4.1 ──► Task 4.3
                                                                  │
Task 2.1 ──► Task 3.1 ──► Task 3.2 ──► Task 4.2 ─────────────────────┤
                                                                  │
Task 5.1 ──► Task 5.2 ──► Task 5.3 ◄───────────────────────────────┘
```

---

## Quick Start (Implementation Order)

1. Add the per-turn counter and rewrite the header (Phase 1).
2. Add the `compact` method on `StandardLoop` (Phase 2).
3. Wire `/compact` in the CLI (Phase 3).
4. Add the tests (Phase 4).
5. Update the documentation (Phase 5).
