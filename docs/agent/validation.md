# Agent validation record

Date: 2026-10-06. Implementation host: macOS, Rust 1.89.
Linux fixture host: ARM64 container, Rust 1.99 (existing stable toolchain).

## New interactive frontend — pending implementation

The accepted [ADR-0010](adr/0010-ratatui-plugin-first-interactive-terminal.md) now selects Ratatui + Crossterm + Tokio as the next-phase interactive frontend through the plugin kernel. **None of the CLI results below validate that Ratatui has shipped or passed PTY acceptance.** The existing line-oriented CLI remains the currently implemented interface and a future explicit fallback.

New TUI evidence will require: inline scrollback/resize and cursor-query PTY tests; plugin-driven command/render replacement; reliable stream projection and responsive editor; complete approval previews without stale-key acceptance; terminal restoration; macOS/Linux validation; and preservation of `run`/`--json`/diagnostic output. Record dependency versions, terminals and failures here when the code is implemented.

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
and CLI integration tests (4 passed); `cargo clippy -p cortex-agent
-p agent-runtime --all-targets -- -D warnings`; focused Markdown lint (0 issues);
`git diff --check`.
