# Huginn

Huginn is the local Rust coding assistant and a product alongside Rook. The
service kernel, OpenAI-compatible streaming adapter, native tools, SQLite
sessions, scoped `AGENTS.md` context, compaction and trusted MCP stdio
client are implemented. Public distribution remains outside this MVP.

## Install and start

Use the repository's Rust 1.89 toolchain on macOS or Linux:

```bash
cargo install --path apps/huginn --locked
huginn --provider mock --workspace /path/to/repository run "list files"
```

From the checkout, use `just huginn-run` or `cargo run -p huginn --` followed
by the same arguments. `mock` is an offline diagnostic provider with a
read-only `list files` example; it does not perform real coding reasoning.

For a model, set `HUGINN_BASE_URL` (including `/v1`), `HUGINN_MODEL`, and
`HUGINN_API_KEY` in the invoking shell, then run:

```bash
huginn --workspace /path/to/repository chat
huginn --workspace /path/to/repository --json run "Explain the project"
huginn sessions
huginn resume SESSION_UUID
huginn resume SESSION_UUID --prompt "Continue the task"
huginn doctor
```

`doctor` validates configuration and reports services without contacting the
model or launching MCP. It currently requires configured credentials to be
present. `--db` selects another session database; `--config` selects a
TOML file.

## Configuration and permissions

See [config.example.toml](../../apps/huginn/config.example.toml) for every
setting. The default location follows the OS configuration directory: on
macOS, `~/Library/Application Support/cortex/huginn/config.toml`; on Linux,
`~/.config/cortex/huginn/config.toml` (or the XDG override). If a new
install runs on a host that only has the legacy
`~/.../cortex/agent/config.toml`, Huginn reads it once (read-only) and
prints a warning suggesting the migration; it never overwrites the legacy
file. CLI options override the file; environment variables provide
defaults for the endpoint and model. API keys are loaded from a named
environment variable. Set `api_key_env = ""` for a local endpoint
without authentication.

Reads are limited to resolved paths within the workspace. Edits show their
complete diff and require approval; changed files require a new diff.
Shell and every MCP call require approval too. Ctrl+C interrupts a turn;
`/quit` ends a conversation. MCP startup asks separately. Only configured
environment variables are passed to MCP processes; native shell inherits
PATH only. Shell and MCP have full host access under the user's account
and must be trusted.

One-shot mode denies effects unless explicitly granted for that invocation:

```bash
huginn --allow native.edit_file,native.shell run "Fix the bug and run its tests"
```

Grants match exact action names, have no wildcard, and authorize all
occurrences of that action during this invocation. Available native
effects are `native.write_file`, `native.edit_file`, and `native.shell`.
MCP actions use `mcp.SERVER.start` and `mcp.SERVER.REMOTE_TOOL`. Grants
are never restored from a session. `--json` emits one typed event per
stdout line; diagnostics use stderr.

### Environment variable fallback

`AGENT_BASE_URL`, `AGENT_MODEL`, and `AGENT_API_KEY` are still read as a
fallback when the canonical `HUGINN_*` variables are unset. The fallback
exists so users who already exported the old names keep working after
the rename; new installations should set the `HUGINN_*` variables
directly.

## Durable context and recovery

Sessions store messages, calls, results, approval events and interruption
state in SQLite. Originals survive compaction. New databases use schema
version 1; newer unknown versions fail explicitly. One process may hold
a session at a time. Provider credentials are not stored. Repository
content, prompts and tool output are part of history and may themselves
contain sensitive text.

Interrupted calls receive a result indicating unknown completion when
resumed; they are never automatically replayed. Inspect external effects
before requesting a replacement action. Compaction keeps the last two user
turns and complete tool pairs, summarizes older messages with the same
provider, and stops if it fails. Context accounting uses conservative
serialized bytes rather than a model tokenizer.

Root and nested AGENTS.md documents are loaded with directory scope
labels on each iteration. Symlinks must resolve inside the workspace;
generated/dependency folders are excluded. Instructions are context and
cannot grant permissions.

## Cross-process session locking

Huginn acquires an advisory lock file before mutating a session. The lock filename remains `cortex-agent-session-<uuid>.lock` during the
compatibility window. Both old and new executables acquire the same lock;
using separate names would allow two versions to mutate one SQLite session
at once. Huginn does not delete, copy or rewrite legacy SQLite databases
through a plain file copy.

## Development and architecture

- [Implementation specification](implementation-specification.md) — contracts and limits.
- [Architecture decisions](adr/README.md) — accepted ADRs.
- [Implementation plan](implementation-plan.md) — milestones and acceptance boundaries.
- [Linux daily-use checklist](linux-daily-use.md) — manual acceptance workflow on Linux.
- [Validation record](validation.md) — automated and manual evidence; status of open items.
- `just huginn-test` runs deterministic kernel, coding, HTTP, MCP and CLI fixtures.
- `./scripts/huginn-linux-smoke.sh` verifies the local install on a Linux host without contacting a model.
- `just ci-local` runs the complete workspace gate, including Rook and Docker E2E.

The Huginn crates have no dependency on Rook's domain or provider
abstractions. Rook is not a supported backend as of 2026-10-06 — see
[validation.md § Rook compatibility result](validation.md#rook-compatibility-result).
Real-model validation and a Linux daily-use run must be recorded before
declaring the personal MVP fully accepted. MCP transport fixtures
require Python 3 at `/usr/bin/python3` on the supported platforms.

Obsidian's rust-agent-harness-research remains the research source. The
repository is the canonical source of implementation specifications and
ADRs.