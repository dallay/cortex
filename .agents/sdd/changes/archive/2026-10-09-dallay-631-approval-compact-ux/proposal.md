# Proposal: Approval numbering, header clarity, and explicit `/compact` for the agent CLI

> **Naming provenance (2026-10-10):** This change predates the
> product rename to **Huginn** (see ADR-0011). The original proposal
> refers to `apps/agent`, `cortex-agent`, `agent-core` and
> `agent-runtime`. Those names are historical and intentionally
> preserved as-is so the decision record stays accurate; the
> corresponding live paths are now `apps/huginn`, `huginn`,
> `huginn-core`, and `huginn-runtime`.

## 1. Intent

**Problem.** Manual acceptance of DALLAY-631 surfaced two product-level UX gaps in `apps/agent`:

1. When the same tool triggers a second approval in the same turn (a denied edit followed by a model-driven retry with a different payload), the terminal prints the new diff and asks for `Approve once? [y/N]` again. The user sees two similar diffs in a row but no signal that the second request is *different* from the first. In one captured session a user denied the first diff but the next prompt already showed a reformulated, partially overlapping diff without re-stating action and target. The wording is technically correct but easy to misread under time pressure.
2. The CLI has no way to proactively compact the session before the configured threshold. Goose's CLI offers `/compact` with a confirmation step; Cortex only compacts automatically when the conservative byte budget is exceeded. That is the right default for safety, but during interactive use it leaves the user without a knob to free room for a longer request, see the compacted state, and decide explicitly.

**Goal.** Reduce the chance of a user approving a different change than intended, and add a confirmation-gated `/compact` command that runs the existing real-model summarization path on demand. No new persistence format, no new provider, no new tool kind. The existing `ApprovalRequest` contract stays unchanged; the loop stays unchanged. All change lives in the terminal policy and the CLI command parser.

**Reference.** Goose's `ToolApprovalOperation` and `/compact` flow: in Goose the approval operation deduplicates and tags each request, and `/compact` asks for explicit confirmation before `do_compact` runs the real `summarize` with the configured `CompactionModel`. We adopt the *outcome* (numbered prompts, confirmation before compaction) without the *mechanism* (a state machine). Cortex's `StandardLoop` already records `ApprovalRequested` and `ApprovalResolved` events and re-summarizes on the same provider; we expose the entry point and tag the prompts.

---

## 2. Scope

### Changes

| Component                                 | Change                                                                                  |
|-------------------------------------------|------------------------------------------------------------------------------------------|
| `apps/agent/src/terminal.rs`              | Number approval requests per turn, print `Action:` and `Effect:` headers, show `different from #N` for repeats, and reword `Approve once?` to `Approve this exact change?` |
| `apps/agent/src/main.rs`                  | Reset the per-turn approval counter when a new user prompt starts; add a `/compact` chat command that runs the loop's existing compaction with a confirmation prompt |
| `apps/agent/src/terminal.rs`              | Provide a `Policy::reset_turn()` helper invoked by the CLI before each new turn         |
| `apps/agent/tests/cli.rs`                 | New test covering two approvals in one turn (deny + confirm `different from #1`)        |
| `docs/agent/implementation-specification.md` | Update approval UX section to describe the new headers and `/compact` flow            |
| `docs/agent/linux-daily-use.md`          | Mention `/compact` as a manual trigger                                                    |
| `docs/agent/validation.md`                | Note the change in the local Ollama record                                                |

### Does NOT Change

- `ApprovalRequest` and `ApprovalPolicy` traits.
- The streaming adapter, the MCP previews, the compaction itself, the SQLite session format, or the resume flow.
- The tool registry, transport, or provider selection.

---

## 3. Approach

### High-Level Strategy

1. **Per-turn approval counter, no contract change.** `Policy` holds a `Mutex<u64>` that tracks how many approvals have been requested in the current turn. The CLI calls `policy.reset_turn()` when it starts a new turn (`chat` loop, `Run`, `Resume`). The `approve()` method increments the counter and renders a header like:

   ```
   Approval for native.write_file (request #1 this turn):
     Action: native.write_file
     Effect: write README.md
   <diff completo>
   Approve this exact change? [y/N]
   ```

   For a repeat in the same turn the header gains a tail line:

   ```
   Approval for native.write_file (request #2 this turn, different from #1):
   ```

   This keeps the existing `ApprovalRequest` struct untouched (no new required fields) and stays inside the per-turn scope we already document in `implementation-specification.md` (the action grant `last only for the current invocation`).

2. **Stable `Effect:` line derived from the action.** The first line of the `preview` is a `Directory:` for `native.shell` and the diff header `--- README.md / +++ README.md` for `native.write_file`/`native.edit_file`. MCP previews already identify server and tool. We render a single `Effect:` line — kept short — derived from the first non-empty line of the preview when the action is `native.*`. This avoids parsing the full preview and adds a second visible signal for action and target alongside the action name itself.

3. **`/compact` reuses the existing real-model summarization.** The CLI recognizes `/compact` (and keeps `/summarize` as a deprecated alias) before calling `implementation.0.run(...)`. The handler:
   - prints `Compact session now? Older history will be summarized; originals stay in the database. [y/N]`;
   - reads a confirmation from the same `Input` used for approvals;
   - on yes, calls a new `LoopService::compact_now(&mut session, &policy, &output, cancellation)` method that performs the same `summary_request` already executed in `StandardLoop::context()`, including the `Event::Compacted` emission and SQLite save;
   - on no, returns to the prompt and continues.

   No new provider call path. No new request shape. The same model, the same system prompt, the same 1024-token `max_tokens` for the summary. The automatic path still triggers at the conservative byte threshold; `/compact` is a manual trigger that asks the same summarizer to run now.

4. **Confirmation uses the same `Input` reader.** We do not introduce a second reader. The `Input` is already decoupled from the Tokio runtime via the detached thread, so reusing it from `/compact` does not interfere with the read loop. We treat the confirmation the same as an approval denial: empty input defaults to no.

### Data Structures

```rust
// apps/agent/src/terminal.rs

pub struct Policy {
    pub input: Option<Arc<Input>>,
    pub allowed: BTreeSet<String>,
    turn_count: Mutex<u64>,
}

impl Policy {
    pub fn reset_turn(&self) {
        *self.turn_count.lock().unwrap() = 0;
    }

    pub fn turn_count(&self) -> u64 {
        *self.turn_count.lock().unwrap()
    }
}
```

`LoopService::compact_now` is a thin wrapper over the existing summarization block. It must remain available without modifying the `AgentLoop` trait, so we expose it as an inherent method on the concrete loop type. The agent binary looks up the loop via the existing `composition::loop_id()` and downcasts, or we add a small trait extension `CompactNow` implemented only by `StandardLoop`. We choose the trait extension to keep the composition kernel unchanged.

### Concurrency Considerations

- The `Mutex<u64>` is only ever held for a few instructions per approval request; contention is bounded by the cost of one model turn. No async lock is needed.
- `/compact` runs while the chat loop is between model turns (no overlapping calls), so a simple `Mutex` is sufficient.
- The confirmation line is read through the existing `Input` reader, so a denial does not block an unrelated `read_file` later in the same session.

---

## 4. Alternatives Considered

| Alternative                                      | Why Not Chosen                                                                       |
|--------------------------------------------------|----------------------------------------------------------------------------------------|
| Add an `ApprovalRequest` field `attempt: u32`     | Cross-crate contract change for a UI concern; the per-turn counter belongs to the policy |
| Track diff hashes server-side                    | Bigger change for the same outcome; hashes add no information the human can act on    |
| Auto-retry denial as a hard rule in the loop     | The user explicitly preferred transparent retry with a new approval, not a silent rule  |
| Add `/compact` as a CLI subcommand instead        | A slash command in `chat` matches Goose UX and keeps the same prompt context          |
| Run compaction in a worker that always shows progress | The current `Compacted` event is sufficient; a progress bar is out of scope           |

---

## 5. Risks

| Risk                                              | Mitigation                                                                 |
|---------------------------------------------------|------------------------------------------------------------------------------|
| Re-asking for confirmation distracts the user     | Numbering and the `different from #N` line make repetition explicit, not opaque |
| `/compact` summary consumes tokens and time      | The existing summarizer is bounded by `max_tokens=1024` and the provider timeouts |
| Resetting the counter at the wrong time           | The CLI calls `reset_turn()` exactly once per new turn, before `run(...)` is invoked |
| Reusing the `Input` reader for `/compact` blocks other input | The reader runs in a detached thread; we treat the line as confirmation, not as a new prompt |

---

## 6. Success Criteria

1. When the model triggers the same tool twice in a turn, the second prompt header includes `(request #2 this turn, different from #1)` and the action name is reprinted.
2. Denying the first prompt in a pair still leaves the workspace unchanged.
3. `/compact` in `chat` confirms once, then runs the real-model summarizer and prints `Context compacted; original history retained.`
4. `/summarize` is still accepted and behaves identically to `/compact`.
5. Existing focused tests pass; the new test passes.
6. `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` stay clean.

---

## 7. Open Questions

- Should the action grant set (`--allow`) also reset per turn? The current contract is *invocation*, not *turn*; we will not change that here. If the user wants finer control later, it is a separate proposal.
- Should the policy print a one-line summary at the end of the turn with `Approvals: #2 (1 denied, 1 approved)`? Likely useful but can be added later if needed; not in scope.
