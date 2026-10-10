# ADR-0010: Ratatui-first, plugin-first interactive terminal architecture

- Status: Accepted
- Date: 2026-10-09
- Product: Cortex agent (provisional name)
- Decision basis: Explicit product direction: build a Pi-inspired extensible agent harness with Ratatui from the outset of the next interactive implementation phase.
- Supersedes: ADR-0004's decision to defer TUI and use a line-oriented CLI as the long-term primary interactive experience. Refines ADR-0003's timing of UI/plugin extension investment; does not replace its composition or lifecycle guarantees.

## Context

The original CLI-first MVP in ADR-0004 already exists. Its line-oriented input/output implementation demonstrated the agent loop, sessions, approvals and MCP integration, but using this UI as the foundation for further interactive features would couple future plugin contributions to an unsuitable presentation surface.

The product differentiator is composability inspired by Pi and Cordis: everything outside the small host/kernel boundary should be a replaceable capability. A UI chosen late would make the plugin contract, prompt composer, command routing, tool rendering and permission UX expensive to retrofit. Codex and Grok Build demonstrate Ratatui-based Rust coding-agent terminals; both also maintain terminal-specific code for inline behavior. Goose demonstrates keeping plain CLI/headless modes separate. These are references, not code to copy.

## Decision

### 1. Select Ratatui now, not conditionally

Use **Ratatui + Crossterm + Tokio** as the interactive TUI stack. Start implementing the Ratatui frontend now; do not build an interim line-editor or bespoke widget framework as the new primary interface. Inline, conversation-first rendering inspired by Pi is the default product target. A fullscreen dashboard remains a separate optional mode.

The normal interactive `chat` and interactive `resume` paths will select the TUI when attached to a supported TTY once the TUI reaches its acceptance gate. A deliberate line-mode override is retained for diagnostics, accessibility, constrained terminals and recovery. Noninteractive `run`, `--json`, `doctor`, and `sessions` preserve stable machine-readable/CLI semantics. Until the new frontend ships, existing CLI behavior is historical/current implementation, not evidence that the TUI decision is deferred.

### 2. Plugin-first boundaries and a minimal core

The kernel remains responsible only for service discovery/resolution, versioned contracts, registration, generation identity, dependency ordering, effect/task ownership and cleanup. Security/approval authority remains at the trusted host boundary. Concrete model providers, native and MCP tools, sessions, agent loops, commands, conversation views, tool renderers, status panels and other user-facing functionality must be replaceable capabilities registered through the existing plugin/service-composition mechanism.

The Ratatui frontend is a **first-party presentation plugin** with its own crate/module boundary; its terminal driver owns the process TTY and is never embedded in `agent-core` or `agent-runtime`. The line-oriented frontend is another presentation adapter, not a second agent loop. A small application composition root selects exactly one presentation owner per interactive process.

Built-in plugins must exercise the same versioned capability contracts intended for subsequent third-party authoring. User-authored plugins and a usable extension/developer experience are explicit roadmap requirements, not an indefinitely deferred side concern. This ADR does **not** promise immediate hot reload, dynamic Rust ABI stability, WASM sandboxing or arbitrary external native code execution; those need their own versioned loading/security decisions. A public SDK's final wire ABI is not frozen by this ADR.

### 3. Host-controlled UI contribution surface

Expose a scoped UI contribution service from the presentation plugin for owned registrations, e.g. commands, keymap actions, status items, tool-result renderers, contextual widgets, selection/confirmation dialogs and notifications. Begin with internal Rust contracts; design version boundaries for later author-written plugins. Registrations have plugin generation IDs and are removed automatically during deactivation.

Plugins produce **declarative host-rendered contributions**. They cannot write arbitrary terminal escape sequences, seize `stdin`, create unmediated key handlers, bypass permissions or modify `ApprovalPolicy`. Host-owned layout and rendering enforce focus, capabilities and lifecycle. Plugin failure must not leave a stale view or orphaned handler.

### 4. One terminal owner, one authority for approvals

At bootstrap, select the TTY owner before constructing input sources. Never run the existing detached `Input::new()` line reader concurrently with Crossterm's event stream. Terminal initialization/restoration is RAII-guarded for errors, cancellation, panics and subprocess handoff. Interactive policies use the existing `ApprovalPolicy`, fail closed, preserve full approval payloads, and require fresh confirmation for each authorized effect. Untrusted text/paste/input buffered before a modal cannot grant permission.

Use a central focus/modal stack and context-aware keymap with deterministic precedence. A plugin can propose actions but the host arbitrates conflicts. Document the exact keymap in the implementation spec and cover it with integration tests.

### 5. Event projection and streaming

Keep `agent-core` free of Ratatui types. Project the existing `EventSink::emit(&self, Event)` contract into TUI-owned state using cheap synchronous updates and a coalesced dirty-frame notification. Completed messages and effect/approval transitions follow authoritative session semantics; in-flight text is an ephemeral mutable tail. A stalled renderer cannot silently drop critical transitions or indefinitely block model execution. Markdown, large tables and code fences are held mutable until safe to commit to terminal scrollback. Use incremental rendering or caches and measure throughput.

### 6. Terminal-specific adaptation is evidence-driven

Start with stock Ratatui APIs. If real PTY and terminal tests expose reproducible inline/resize/cursor/scrollback defects, first isolate the smallest terminal adapter or upstream fix. A custom terminal or fork is allowed **only when needed**, with its own regression tests, ownership and license/provenance record. This is not grounds to postpone using Ratatui. No copying of Codex or Grok implementation without a separate dependency/provenance review.

Initial supported targets are macOS and Linux. Cover native terminals, SSH and tmux explicitly before claiming compatibility; Windows and specialized image protocols require a later acceptance gate.

## Alternatives considered

- Continue the line-oriented CLI until a future redesign: rejected; shifts plugin and UI architectural debt forward.
- Write a custom Rust TUI framework from scratch: rejected; duplicates mature Ratatui abstractions.
- Switch to an opinionated UI framework first: rejected for this phase; Ratatui gives direct control and has agent-tool precedents.
- Adopt Codex/Grok's custom terminal directly: rejected absent evidence and per-file provenance review.
- Make every plugin a dynamically loaded Rust library immediately: rejected; stable runtime loading/security is orthogonal to plugin-first component boundaries.

## Consequences

- Ratatui is a committed design choice; correctness gates control rollout and adapter fixes, not framework selection.
- Interactive app composition must be refactored away from the current global line-oriented stdin path.
- A separately maintained TUI module and UI contribution contract are introduced early.
- Default UI selection must never change machine-readable output and must allow explicit fallback.
- Plugin extensibility must be visible in initial feature acceptance: at least one replaceable built-in command or tool renderer is installed and removed through the plugin lifecycle.
- Existing CLI/model/approval/session regression tests must remain green.

## Follow-up and evidence

Implement a vertically complete Ratatui TUI plugin before adding sophisticated dashboards. Prove: streaming and stable native scrollback; unique committed entries on resize; responsive keyboard and focus; safe approval/rejection; terminal restoration; plugin contribution mount/unmount; noninteractive mode compatibility; and session resume.

Track stock-Ratatui inline issues #2666 (closed, regression still required) and #2640 (cursor position query race, still open at research time). Use version-pinned PTY tests, not conjecture, to decide workarounds. Enforce explicit authorization in the shared approval policy, not in view callbacks.

Related records: [ADR-0003](0003-minimal-service-composition-kernel.md), [ADR-0004](0004-personal-coding-agent-with-cli.md), [ADR-0007](0007-read-freely-approve-effects.md), [implementation specification](../implementation-specification.md), [implementation plan](../implementation-plan.md).
