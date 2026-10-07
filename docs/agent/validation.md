# Agent validation record

Date: 2026-10-06. Implementation host: macOS, Rust 1.89.
Linux fixture host: ARM64 container, Rust 1.99 (existing stable toolchain).

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

Semgrep tools are unavailable in this session. The user explicitly authorized
continuing without Semgrep, retaining Rust checks and just ci-local.

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

The user authorized omitting unavailable Semgrep tools for this change. Real-model
compaction, human approval UX and Linux daily use remain unvalidated by this smoke.

Workspace verification: `just ci-local` passed stages 1–8, including workspace
Rust tests, 175 Vitest tests and documentation. The audit reported the existing
`anyhow` unsoundness and yanked `chacha20` warnings; no lockfile was changed.
Stage 9 built the Docker image but could not start the container because the
daemon cannot bind-mount the configuration file under this host's `.codex` path.
The unmodified gate therefore did not pass. A temporary copy of the same E2E
runner uses a Docker volume containing that configuration instead of the host
bind mount: **83 E2E tests passed, 7 skipped** across Chromium, Firefox and WebKit.
No runner changes are included. Markdown lint and diff whitespace checks also
passed after updating this record.

## Rook compatibility result

The plan reserved this item for "Rook's current streaming model does not represent
structured tool-call arguments." The inspection on 2026-10-06 confirmed a stricter
finding: Rook's OpenAI adapter does not represent tool calls at all during
streaming.

Evidence from the current source:

- `crates/infrastructure/providers-openai/src/provider.rs:151-161` builds the
  outbound `StreamChunk` from the OpenAI delta's `content` only; the parsed
  `tool_calls` field is dropped.
- `docs/providers.md:52-55` records the same gap from the user-facing side:
  `stream()` is marked ❌ not yet implemented.
- The agent's adapter (`crates/agent/runtime/src/model.rs:67-122`, `SseParser` at
  `:137-286`) does send `tools` in the request and assembles fragmented
  `tool_calls` deltas into `ModelDelta::ToolCall`. It expects the OpenAI-shaped
  response with `choices[0].delta.tool_calls` and `finish_reason: "tool_calls"`.

End-to-end consequence: any prompt that requires a tool call against a Rook
gateway reaches the `SseParser.finish()` step with no assembled calls, the
`stream ended without a finish marker` or `missing tool identity` error fires,
and the turn aborts. Plain-text turns may still appear to work because the
content-only delta path is intact, but the agent's MVP workflow is tool-driven
and cannot complete against Rook today.

Per ADR-0005 ("Do not advertise that integration as working until the complete
tool-call round trip has been tested"), Rook is recorded as **not supported** as
a model backend. Closing this item did not require running Rook; a live request
would only confirm the same code-level limitation. Supporting Rook would need a
scoped change in `crates/infrastructure/providers-openai/src/provider.rs` plus
tests that exercise the tool-call delta path. That work is out of scope for
the personal MVP and is not scheduled here.

The offline mock and local protocol fixtures validate implementation mechanics;
they do not establish quality of model reasoning or compatibility with every endpoint.

## Linux daily-use record

The agent is ready to be exercised against a real model on a Linux host. The
manual workflow lives in [linux-daily-use.md](linux-daily-use.md); the
companion `scripts/agent-linux-smoke.sh` verifies the local install (toolchain,
build, deterministic suite, offline mock session) and appends one line to
`linux-smoke.log` on success. It does not contact a model.

Append each completed session here in the shape documented in the checklist:

- YYYY-MM-DD host=<hostname> rustc=<version> endpoint=<provider/model>
  steps=<which steps completed 1–7> result=<ok|degraded|failed>
  notes=<one-line summary or "see issue #N">

(Empty — first session pending.)
