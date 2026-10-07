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

- Real-model smoke test: use a tool-capable Chat Completions endpoint, approve a
  small code edit and its tests, then restart and resume the session.
- Linux daily use: deterministic coding and MCP fixtures passed; repeat the
  interactive workflow with a real model on a Linux host. **Checklist and
  smoke harness are ready** — see "Linux daily-use record" below. Five `ok`
  sessions across at least two repositories are the closing bar.
- ~~Rook: verify structured streaming tool-call arguments before declaring
  support.~~ **Closed 2026-10-06 — not supported as a backend.**
  See "Rook compatibility result" below.
- Public packaging and release tags follow the personal MVP.

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
