# Huginn implementation specification

## Crate boundaries

| Crate | Responsibility |
| --- | --- |
| `huginn-core` | Typed ports, events, session model and service composition kernel |
| `huginn-runtime` | Standard loop, HTTP/SSE, native tools, SQLite, instructions and MCP |
| `huginn-presentation` | Replaceable presentation contract, Ratatui driver, UI state and scoped contributions |
| `huginn` | Configuration, composition root, terminal approvals and CLI |

The runtime and presentation depend on the core; the CLI composes them. None depends on Rook.
Ports use async traits and typed errors. ModelProvider yields a stream of deltas;
AgentLoop accepts a session, prompt, approval policy, event sink and cancellation
token. The standard loop is registered through LoopService, so another native
implementation can replace it without changing the supervisor.

> **Naming note (2026-10-10):** Crate names in this table are the canonical
> Huginn names. The previous `agent-core` / `agent-runtime` / `cortex-agent`
> crates are the same code under their new names; see
> [ADR-0011](adr/0011-huginn-product-naming.md).

## Ratatui-first interactive frontend contract (ADR-0010)

This section governs **new** interactive behavior. The first vertical slice is implemented in `crates/huginn/presentation`. Terminal eligibility is not a terminal-compatibility certification; the broader smoke/resize matrix remains in [DALLAY-665](https://linear.app/dallay/issue/DALLAY-665).

- **Presentation plugin:** Ratatui + Crossterm + Tokio run behind a first-party plugin/service boundary. The composition root selects one terminal owner before initializing input. No Ratatui types in `huginn-core` or `huginn-runtime`.
- **Default selection:** `chat` and interactive `resume` select Ratatui when both stdin/stdout are TTYs, `TERM` is set and non-dumb, and the initial size is at least 20×8. Unsupported initialization fails with a recovery hint, never silently switches readers. `--line-mode` selects the explicit recovery adapter, including piped recovery input. `run`, `resume --prompt`, `sessions` and `doctor` do not open a presentation. `--json` preserves the existing line/NDJSON adapter and never opens Ratatui or emits ANSI UI output.
- **Plugin model:** all non-kernel features, including view/prompt functionality, commands, tool-result renderers and status contributions, register through kernel-managed services. Contributions are declarative and host-rendered; registration and cleanup belong to the plugin generation. User-authored plugin extensibility is a roadmap requirement, but a public dynamic ABI is not implied by the first built-in implementation.
- **Focus/input:** host owns raw mode, stdin, keymap context, modal priority and terminal restoration. The legacy `Input::new()` is never constructed for TUI mode. One Tokio driver uses bounded, zero-timeout Crossterm polling at 33 ms intervals; no competing EventStream reader runs during stock inline cursor queries. Modal paste/repeat events are ignored, queued input is drained before display, and a fresh unpredictable eight-hex-digit challenge is armed only after drawing. Enter with a missing/wrong code denies, as does Esc. No answer grants nothing.
- **Approvals:** Both adapters implement the existing `ApprovalPolicy` only and derive numbering plus `Effect:` summaries from the shared `huginn-core::approval` model, so semantics cannot drift. Approval views show the complete authorized action/diff, distinguish pending vs decided, bind to the request ID and reject stale confirmations; only policy grants authority. Plugin views are never authorities. The line adapter confirms with one-time yes/no; the TUI modal requires typing a fresh unpredictable code.
- **Streaming state:** `EventSink::emit` remains synchronous; it synchronously projects typed events into TUI-owned state with short critical sections and emits a coalesced dirty-frame signal. Token deltas fold into the mutable `tail`; the retained event log keeps only non-text authoritative transitions, so there is no unbounded token-to-frame queue. Renderers read snapshots. Persisted session messages and permissions are authoritative, unfinished tokens are ephemeral, and tool/approval transitions must not be silently dropped. If this contract proves inadequate, evolve `EventSink` explicitly with tests rather than hide overload losses.
- **Scrollback:** preserve native history semantics. Keep mutable Markdown blocks/tables/code fences in a streaming tail until safe to commit; once committed, do not duplicate them on resize. PTY tests must cover cursor query races and Windows/SSH/tmux claims separately.
- **Lifecycle:** `presentation.ratatui` requires the typed `agent:loop@1` capability and provides `agent:presentation@1` and `agent:ui-contributions@1`. `PresentationService` wraps a replaceable trait. A Supervisor-owned cleanup task stops and waits for active drivers on unload. Connection destruction synchronously restores raw mode/paste/cursor under a terminal-I/O lock; failed initialization and panics also restore. No alternate screen is entered.

### Initial keymap and contribution surface

- Enter submits when idle. Alt+Enter, Shift+Enter (where encoded by the terminal), Ctrl+J or bracketed paste insert newlines. Left/Right/Home/End/Up/Down, Backspace and Delete edit UTF-8 text. The three-line visible editor scrolls to its cursor; it is editable but cannot submit a second turn while a model is running.
- Esc cancels an active turn or denies a modal. Ctrl+C cancels an active turn (including MCP startup/compaction) and exits when idle. `/quit` and `/exit` exit. A modal has priority over the composer; Up/Down/PgUp/PgDn scroll its wrapped full action/diff, Home/End jump to the ends.
- `/compact` and `/summarize` use the registered loop and a host-owned confirmation modal. The normal effect `--allow` grants remain invocation-scoped. Resume projects saved messages and warns about interrupted effects; nothing is automatically replayed.
- `presentation.conversation` registers `/help` and a tool-result *status* contribution through a lease bound to `PluginContext::generation()` and its cancellation token: a success heading, a failure heading rendered with error severity, plus the latest-active override rule. Duplicate/reserved command names are rejected. Cancellation immediately hides registrations and lease destruction removes them. This initial surface returns declarative text, not callbacks, focusable widgets, key handlers, terminal access or approval authority. Per-tool typed result views (diff, structured output) are explicit follow-up under DALLAY-666.
- Completed authoritative session messages are inserted once into stock `Viewport::Inline` history. In-flight Markdown remains a mutable tail (basic heading/fence coloring, not a complete Markdown engine). Terminal escape/control bytes are filtered. Large previews remain scrollable without truncating payloads. Comprehensive scrollback/rapid-resize, SSH/tmux and native-terminal certification remain DALLAY-665 work.

## Services and effects

Services use explicit `name@major` identifiers, one provider per service and one
scope. Plugins declare provided and required services. Activation resolves only
declared dependencies, stages registrations, verifies the manifest and publishes
atomically. Failed activation cancels owned tasks and discards staged registrations.
Missing dependencies block; ambiguity and cycles fail explicitly. Removal stops
transitive consumers before their provider. Cleanup and deactivation have deadlines;
diagnostics preserve generation, state and errors. Native plugins are trusted and
must register owned tasks through PluginContext. Hot reload and a public SDK are
outside this contract.

Tools expose definitions, prepare a payload and optional approval preview, then
execute. The loop persists the assistant tool call before execution and persists
approval outcomes before effects. Calls execute sequentially. Permission policy
runs outside model/provider code and cannot be changed by repository text. The
interactive presentation shows the complete approval action and preview without
truncating diffs, then requests a one-time confirmation; explicit `--allow`
grants apply only to that CLI invocation. Approval previews must identify the
target/location, requested change or arguments, and relevant external-effect or
sandboxing risk. MCP tool previews identify the server and remote tool, show the
arguments sent, and warn that effects depend on the server; MCP startup previews
identify command, arguments, working directory, environment names, host privileges
and lack of sandboxing. Native effect denial must occur before mutation.
The terminal's exact action grants last only for the current invocation.

The line-adapter policy numbers each approval within a turn. The header reads
`Approval for <action> (request #N this turn):`, followed by a single
`Action: <action>` line and, when the preview starts with a known prefix, a
single `Effect: <one-line summary>` line above the full preview. The counter
resets to 0 at the start of each new turn so each turn's first approval is
numbered one. The prompt text is
`Approve this exact change? [y/N]`.

The chat loop also accepts `/compact` (with `/summarize` as a deprecated
alias) to compact all complete finalized turns in the current session. The line adapter
prints `Compact session now? Older history will be summarized; originals stay
in the database. [y/N]` and reuses the same `Input` reader used for approvals.
After confirmation it calls the registered loop's `compact(session, output,
cancel)` capability, so Ctrl+C reaches the active summary request and JSON mode
receives the persisted `Event::Compacted` event. Manual and automatic paths use
the same summary request, budget validation, result validation, persistence,
and event-emission helper; automatic compaction still preserves the newest
complete turn when trimming to fit the configured context.

## HTTP and event contract

OpenAI-compatible Chat Completions POSTs to `base_url/chat/completions` with tools,
streaming and usage enabled. Requests use rustls, configured deadlines and no
redirects. URLs cannot embed credentials, query strings or fragments. Errors omit
provider response bodies and credential values. SSE parsing supports fragmented
UTF-8 and function arguments. Tools are dispatched only after a valid finish marker
and complete object arguments. Invalid or truncated streams fail the turn.

JSON events use a `type` discriminator: turn_started, text, tool_started,
approval_requested, approval_resolved, tool_finished, usage, compacted,
turn_finished and error. Event variants and fields are defined in `huginn-core`.
Text is incremental; approval and tool events include their identifiers.
Text deltas are emitted directly to the live sink, not appended to persisted events.

## Persistence and context

SQLite schema version 1 uses `sessions(id TEXT PRIMARY KEY, data TEXT NOT NULL,
updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)`. `data` is a serialized Session
with original messages, events, workspace, separate summary and exclusive summary
boundary. Each update is an atomic UPSERT. Future migrations must be explicit;
unknown higher user_version values are rejected. Cross-process advisory lock files
protect an actively resumed session and release automatically after a crash.

Completed assistant responses are saved as messages before any tool execution;
resume and context assembly use those messages, not replayed text events. Text
delta count does not increase session saves. Partial text from incomplete, failed,
cancelled or crashed turns is live-only and intentionally lost on reload. Existing
stored text events remain readable; new streams do not persist them. Usage,
turn/error, compaction, tool and approval persistence boundaries are unchanged,
including saving approval outcomes before effects and tool results after execution.

An unfinished tool call is paired with an unknown-completion recovery result rather
than replayed. Stored summaries must end on a user-turn boundary. Context assembly
includes current instructions, the summary and unsummarized original messages.
At 80% of the conservative budget, older complete turns are summarized by the same
provider with tools disabled. The last two user turns remain verbatim. Summary
failure or overflow stops with original history intact. Serialized byte counts,
including tool definitions and output reservation, are conservative token estimates.

## Native and MCP limits

- UTF-8 file reads and writes: 1 MiB per file; complete approval diffs: 64 KiB.
- Tool output: 64 KiB with a visible truncation marker; search: 100 matches.
- File listing: 100 entries with a visible limit marker; select a narrower path for more.
- Traversal: 20,000 entries, skipping generated/dependency folders and directory symlinks.
- Read tools resolve containment; new writes require an existing contained parent.
- Writes compare prepared paths and original content, then use a temporary file and
  atomic rename. On Unix, existing destination permissions are read before opening
  the temporary file and used as its creation mode; new files use mode `0666`, allowing
  the kernel to apply the process umask. Non-Unix creation behavior follows platform
  defaults. Concurrent hostile filesystem mutations are outside the trust model.
- Native tool execution dispatches each operation to a focused handler to keep the
  orchestration path readable; blocking workspace traversal runs on Tokio's blocking pool.

```mermaid
flowchart TD
    A[Dispatch native operation] --> B{Operation kind}
    B -- read --> C[Read bounded file range]
    B -- list/search --> D[Traverse workspace on blocking pool]
    B -- write/edit --> E[Validate approved content and destination]
    E --> F{Destination exists?}
    F -- Yes --> G[Read and retain existing permissions]
    F -- No, Unix --> H[Select creation mode 0666]
    F -- No, non-Unix --> I[Use platform-default creation permissions]
    G --> J[Create temporary file with existing mode]
    H --> K[Create temporary file; kernel applies umask]
    I --> L[Create temporary file]
    J --> M[Write contents, sync, then atomic rename]
    K --> M
    L --> M
```
- Shell uses POSIX sh, null stdin, PATH-only environment and supervised process groups.
  Timeout, cancellation and dropped execution kill the group; stdout/stderr are drained
  with bounded retained output. Nonzero exit status is returned to the model.
- SSE events: 1 MiB; at most 64 calls, each argument string at most 256 KiB.
- Instructions: 64 KiB per document and 128 KiB combined, reloaded each iteration.
- MCP uses official rmcp 3.5.1, pinned for Rust 1.89, local stdio only.
- MCP discovery: 128 tools per server, 16 KiB per schema. Names are
  `mcp_SERVER_INDEX`; approval identity includes the remote tool name.
- MCP launch uses only configured literal environment values and named env_from
  sources. Startup and each call need approval. SDK transport closure and process
  groups bound teardown; failed or timed-out calls may have unknown external effects.

## Acceptance evidence

Separate deterministic tests exercise kernel lifecycle/rollback, edit approval and
stale diffs, shell execution and descendant cleanup, path containment, interruption
and resume, scoped instructions, compaction, HTTP and MCP transports, and CLI JSON.
Public endpoints and Rook have not been asserted compatible. macOS deterministic
checks do not establish Linux daily-use acceptance. Record full gate results in
[validation](validation.md); real-model and Linux checks remain manual acceptance.
