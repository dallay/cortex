# Agent validation record

Date: 2026-10-06. Implementation host: macOS, Rust 1.89.
Linux fixture host: ARM64 container, Rust 1.99 (existing stable toolchain).

## Ratatui vertical slice — 2026-10-10

The first [ADR-0010](adr/0010-ratatui-plugin-first-interactive-terminal.md) presentation plugin is implemented. Dependency pins: Ratatui 0.30.2, Crossterm 0.29.0; Tokio is resolved by `Cargo.lock`. Evidence here is automated macOS PTY evidence, **not** native Terminal.app/iTerm2, SSH, tmux, Linux or Windows certification. `--line-mode` remains the deliberate recovery route. The slice was rebased onto the Huginn rename (`agent-*` → `huginn-*`; service ids stay `agent:*`); `session_ownership` pins its lease owner to `--line-mode` because interactive `resume` opens Ratatui by default.

- `cargo test -p huginn-presentation --all-features --tests`: 8 deterministic state/plugin tests and 7 PTY tests passed. Coverage includes UTF-8 multiline editing, one-time transcript synchronization, 10,000 synchronous text transitions plus critical events without a renderer, full large approval payloads, challenge freshness/cancellation, command/renderer unload and re-registration, missing dependencies, delayed stream typing/cancellation, stale/paste confirmation rejection, fresh approval, normal/panic/failed-init restoration, synchronous connection drop, unload→immediate reopen and drop→replacement input.
- `cargo test -p huginn --tests`: 10 existing unit tests, 5 CLI tests and 2 new routing/PTY tests passed. The actual binary's default chat and interactive resume use the inline driver, save multiline prompts and preserve session messages. Non-TTY default failure and explicit piped line recovery passed. Existing JSON/run/doctor/sessions/manual-compaction checks remained green.
- `cargo clippy -p huginn-presentation -p huginn --all-targets --all-features -- -D warnings`: passed. Formatting and focused Markdown lint passed. Post-review verification ran all 15 presentation tests and focused all-features Clippy against the final input-lease implementation.
- Minimized cursor-query failure: an EventStream reader running while stock Ratatui inserted initial inline history caused cursor-position timeouts in two PTY tests. The driver now uses bounded zero-timeout Crossterm polling without a competing input thread; those tests pass. No custom terminal or copied upstream code was introduced. This is relevant to stock Ratatui #2640 and is tracked with the broader [DALLAY-665 compatibility gate](https://linear.app/dallay/issue/DALLAY-665).
- Independent review found End→Up navigation, asynchronous connection-drop restoration and unload cleanup timing defects. Regressions failed before fixes and now pass. Supervisor-owned cleanup waits for terminal destruction; connection destruction restores raw mode synchronously.
- Follow-up review found an input-revocation race because task abort is asynchronous. Both input paths now share the restoration lock and check generation activity before poll/read. Final focused review confirms this structurally addresses the race. The added drop→reopen→input PTY test passed before and after, so it is coverage, **not** a deterministic RED→GREEN reproduction of that race. `cargo tree` verified no presentation/Ratatui/Crossterm dependency in core/runtime/Rook.
- Review follow-ups fixed with regression tests: Unicode bidi controls filtered in `safe()`; `UiState` event log bounded to non-text authoritative transitions; contribution command validation (invalid/reserved/same-generation duplicate/cross-generation collision) covered deterministically.
- Review round 2 (scoped REQUEST CHANGES, no P0/P1) addressed with tests: out-of-scope Rook doc deletions reverted out of the PR; approval numbering (`request #N`) and `Effect:` summaries restored in the TUI modal through the shared `huginn-core::approval` model (line adapter rewired to the same functions, output byte-identical); contribution surface extended with a contributed failure heading rendered with error severity and scoped as status-only in the spec; composer title carries the Huginn identity with a PTY regression guard; O(tail) re-render cost noted in code and tracked with the DALLAY-665 load/latency measurements.
- Known advisory: `codecov/patch/backend` reports ~78% against an 85% target. The Ratatui driver is verified by PTY tests that drive the binary as a subprocess, which `llvm-cov` cannot attribute; in-process presentation-crate coverage is ~86% regions. Overall `codecov/patch` passes and the repository gate (`just ci-local`) sets no coverage threshold.
- Semgrep source scan before generation passed. The first dependency selection (Ratatui 0.29) exposed an `lru` advisory; selecting stock Ratatui 0.30.2 resolved it. The subsequent supply-chain scan reported no `Cargo.lock` findings, but 75 findings in existing non-Rust lockfiles; this is not a repository-wide clean security scan.

Remaining DALLAY-665 evidence: semantic terminal-emulator scrollback/resize checks (including 100 rapid transitions), 10k-line/100 tok/s/10× burst and input-latency measurements, native-terminal versions, Linux, SSH/tmux, SIGHUP and subprocess handoff. Human approval usability and real-model TUI smoke remain unvalidated. The PTY harness responds to cursor queries as a simulated xterm; it does not certify xterm or any actual terminal product.

### DALLAY-664 repository gate

`just ci-local` completed successfully on 2026-10-10 in 496 seconds: Markdown,
formatting, workspace Clippy/check, 936 Rust tests (including doctests), 179 Vitest
tests, documentation, audit and Docker-backed browser E2E. Browser E2E reported
83 passed and 7 skipped across Chromium/Firefox/WebKit. The final ownership-review
change and added replacement-input PTY test also passed the subsequent focused
presentation tests and all-features Clippy; the full gate's initial lint stages
preceded that change. These dashboard E2E results are not Linux TUI evidence.

Warnings: Vite reported a future native-config import-extension warning;
`cargo audit --no-fetch` reported the existing yanked `chacha20` 0.10.0 warning.
The audit stage remains non-blocking in the repository runner. Skipped browser
tests and existing non-Rust supply-chain findings are not claimed as validated.

## Automated evidence

The deterministic suite covers service lifecycle and rollback, approved coding
changes followed by shell tests, denied edits, stale diffs, workspace containment,
fragmented SSE, durable session recovery, compaction, instruction scopes, process
timeouts, HTTP adapter behavior, MCP discovery/failure and CLI JSON/resume.
The 21 agent tests passed on macOS and in a Linux ARM64 container without network
access. The CLI also completed an offline read-only turn over Cortex itself.
The full workspace gate passed Markdown, formatting, Clippy, compile, Rust tests,
175 Vitest tests and documentation; the final Docker E2E result is pending below.

The E2E Dockerfile used an obsolete pnpm --include flag. It now installs the frozen
lockfile with dev dependencies included by default. The E2E runner also propagates
Docker build failures through its log pipeline.

The repository's cargo audit step is non-blocking (uses `|| true`). It reports four
advisories in existing lockfile packages: crossbeam-epoch 0.9.18, h2 0.4.14,
quinn-proto 0.11.14 and rustls 0.23.40, plus warnings for anyhow and chacha20.
Those versions were already present; no dependency upgrades were included in this
feature. Passing the configured CI gate does not mean the audit is clean.

Semgrep tools were unavailable for the earlier implementation checks; current
focused MCP source and test scan completed with no findings.

## Remaining acceptance

- Real-model coding and restart/resume smoke: **passed on macOS 2026-10-07**;
  see the record below. Interactive approval usability still needs human daily use.
- Linux daily use: deterministic coding and MCP fixtures passed; repeat the
  interactive workflow with a real model on a Linux host. **Checklist and
  smoke harness are ready** — see "Linux daily-use record" below. Five `ok`
  sessions across at least two repositories are the closing bar.
- ~~Rook: verify structured streaming tool-call arguments before declaring
  support.~~ **Closed 2026-10-06 — not supported as a backend.**
  See "Rook compatibility result" below.
- Public packaging and release tags follow the personal MVP.

## Real-model smoke record — 2026-10-07

Host: macOS, Rust 1.89.0. Endpoint: `https://omnirouter.ahome.quest/v1`.
Requested model route: `auto/best-coding-fast`; the underlying model was not
identified. This result establishes the tested route's compatibility for this
workflow, not every model or endpoint.

A temporary terminal harness drove the actual CLI in a disposable Python
workspace. It answered each displayed approval prompt with `y`; it did not use
`--allow`. The temporary API credential was provided in the process environment,
never in configuration, fixture files or this record.

- Coding: two `read_file` calls inspected a broken addition function and its
  unittest. `edit_file` displayed a diff and received approval before replacing
  subtraction with addition. A separately approved `shell` call ran
  `python3 -m unittest -v` successfully. An independent test execution also passed.
- Interruption: a separately approved shell command waited 30 seconds before
  creating a marker file. SIGINT after approval interrupted the turn; the saved
  session retained its interrupted state and the marker was absent.
- Restart/resume: a new CLI process resumed the interrupted session, completed
  a read-only inspection and retained an unknown-completion recovery result for
  the interrupted call. The marker remained absent; the command was not replayed.
- Regression checks: all 28 agent tests passed (4 kernel, 16 coding workflow,
  6 transport, 2 CLI). `doctor` now reports Rook as `unsupported`, matching the
  compatibility record. The Linux checklist uses `chat` for individual approvals,
  explains restarting after cancellation and requires observed compaction.

The user authorized omitting unavailable Semgrep tools for that change. Real-model
compaction, human approval UX and Linux daily use remain unvalidated by this smoke.

## DALLAY-631 acceptance session — 2026-10-09

Host: MacBook Pro M2 Max, 32 GB unified memory. Ollama server version: 0.40.2.
An isolated test directory, SQLite database and disposable workspace were used
under `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/dallay631-ollama.PhjPZU`.
The workspace contained only a README fixture and no MCP server config. No API
credential was needed; the temporary config set `api_key_env = ""`.

### Qwen2.5-Coder baseline

With `qwen2.5-coder:14b`, a direct text completion and agent `doctor` passed. In
both direct tool-schema and actual CLI attempts, the model returned tool-call-like
JSON in normal assistant `content`, rather than structured `tool_calls`. Thus the
agent correctly did not execute the requested tool. This was not counted as
acceptance of tool calling or compaction.

### Qwen3-8B structured tool compatibility

Model used: `qcwind/qwen3-8b-instruct-Q4-K-M:latest`, architecture Qwen3, 8.2B,
Q4_K_M, Ollama reports tools capability. It was loaded fully on GPU with a 32K
runtime context. On the exact minimal `read_file` tool request:

- OpenAI-compatible `/v1/chat/completions`, `stream=false`: **PASS** — response
  had `finish_reason="tool_calls"` and `message.tool_calls[0]` named `read_file`
  with `{"path":"README.md"}`.
- Native `/api/chat`, `stream=false`: **PASS** — response had `message.tool_calls`
  with the same tool name and object arguments.
- OpenAI-compatible `/v1/chat/completions`, `stream=true`: **PASS** — SSE emitted
  `choices[0].delta.tool_calls`; tool-call arguments were fragmented across events
  as allowed by the streaming protocol.
- Actual Cortex CLI `run` in the isolated workspace: **PASS** — it invoked
  `read_file`, printed the README heading `Marea Demo`, and returned without
  changing the workspace. This confirms the streamed structured tool path works
  through the current Cortex adapter for this model/runtime combination.

### Compaction and persisted-original evidence

A single CLI session was resumed across multiple processes. The temporary config
used a low `context_tokens = 4096` and `max_output_tokens = 1024` to reach the
configured compaction threshold. The model produced a coherent summary preserving
the Marea task and key decisions, and subsequent resumed turns could answer from
that summary (for example, identify Marea and `ValueError` for an empty list).
SQLite inspection confirmed the persisted `summary_through` advanced to 12 and
`summary` was present. Earlier original messages, including a complete assistant
tool call and its corresponding tool result, remained in `messages`; the workspace
README hash remained unchanged.

**Limitations observed:** with the small budget, the model summary did not retain
all four pending test cases: after compaction the model reported insufficient
context for those details. Therefore this demonstrates that compaction triggered,
original messages/tool pair were retained, and the model continued using salient
summary facts; it does **not** establish perfect semantic retention of every detail.
When a repeated follow-up ran into the intentionally tight 4096-token budget, the
loop correctly reported `context cannot fit safely` rather than sending an unsafe
request. Raising the temporary test budget to 8192 allowed the model to continue
from the summary. No corrupt session or workspace mutation was observed. The
separate automated `failed_compaction_preserves_original_history_and_stops` test
continues to cover summary failure rollback/preservation.

### Interactive approvals: not accepted

An attempt to run interactive `chat` under the shell tool failed because it did
not provide a usable TTY; the CLI correctly rejected non-terminal stdin. A PTY
harness attempt timed out before a model response, and no approval was granted.
The disposable workspace README remained unchanged. Thus no real-model human
approval was exercised for file edits or shell commands. MCP startup and MCP tool
approval were not tested because no local MCP server/config was provided.

Conclusion: Ollama/Qwen3-8B is compatible with Cortex's streamed structured tool
calls in this setup and can trigger/continue after compaction, with the noted
summary-fidelity limitation. The issue's approval-UX acceptance remains open until
file-edit, shell, MCP-start and MCP-call prompts are manually examined, including
denials and fresh-process behavior.

### 2026-10-09 follow-up — approval numbering and `/compact`

The local acceptance session surfaced an approval UX gap: a second approval in
a turn did not indicate its position in the sequence. The terminal policy now
numbers each approval and prints single-line `Action:` / `Effect:` headers
above the full preview. It does not claim a later request is different unless
that property is actually established. The CLI accepts `/compact` (with
`/summarize` as an alias) with explicit confirmation. Manual compaction selects
the latest complete finalized turn; automatic compaction preserves the latest
complete turn while trimming older history. Both share the same summary
request, budget/result validation, persistence, and event-emission helper.
The command calls the registered loop, reuses the `Output` event sink, and
propagates Ctrl+C cancellation.

Regression coverage now includes runtime compaction across 1, 2, and 3
complete turns with tool-call/result groups, cancellation without persisting a
summary, exact approval header text for repeated identical actions, and PTY
integration tests for accepted/declined `/compact` confirmation in JSON mode.
These use the mock provider and do not replace manual terminal acceptance of
repeated approvals or real-model compaction; both remain pending.

Focal checks: `cargo fmt --check`; runtime `stream_persistence` (4 passed),
`coding_workflow` (27 passed), `transports` (6 passed); CLI unit tests (7 passed)
and CLI integration tests (4 passed); `cargo clippy -p huginn
-p huginn-runtime --all-targets -- -D warnings`; focused Markdown lint (0 issues);
`git diff --check`.
