---
applyTo: "apps/huginn/**,crates/huginn/**"
---

# Huginn — Product instructions

These instructions extend `.agents/AGENTS.md` for `apps/huginn` and
`crates/huginn`. Global invariants remain mandatory. All paths below are relative
to the repository root.

## Architecture and contracts

- `huginn-core` owns domain contracts; `huginn-application` orchestrates them;
  `huginn-runtime` implements I/O adapters; `huginn-presentation` implements UI.
  `apps/huginn` is the composition root. Keep plugin contracts in `plugin-api`
  and the reference implementation in `reference-extension`; do not model these
  crates as a chain that makes inner layers depend on outer layers.
- `just huginn-architecture` is a blocking gate for core and application.
  Preserve dependency allowlists in
  `.github/scripts/check-huginn-architecture.py`, including dev, optional and
  target-specific dependencies. Review any allowlist addition deliberately.
- Core and application must not use `std`/`tokio` filesystem, process or network
  I/O, or import adapters such as reqwest, rusqlite, rmcp, ratatui or crossterm.
- Read `docs/huginn/architecture.md`, `docs/huginn/README.md` and the relevant
  ADRs in `docs/huginn/adr/` before changing contracts. Reuse established ports,
  lifecycle management and dependency injection.

## Effects and trust boundaries

- `HostPolicy` owns approval of effects (ADR-0007). No tool, plugin,
  instruction file, model output or tool metadata can grant authorization.
- Fail closed on cancellation, denial or unavailable presentation. Keep action
  preparation, reviewed previews, authorization and execution distinct; recheck
  edited content after approval to reject stale proposals.
- Preserve workspace path and symlink boundaries for native tools. Approval of
  shell commands or MCP invocations does not make their processes sandboxed.
- Treat MCP output, external commands and plugin schemas as untrusted
  (ADR-0006). Server startup permission is distinct from invocation permission.
- Preserve session history and tool-call/result pairing through compaction and
  recovery (ADR-0008); interrupted effects must not replay automatically on resume.
  Never persist provider credentials in session history.

## TUI and validation

- Keep the ratatui TUI keyboard-first, with visible focus, understandable effect
  previews and approvals, and clear loading, empty, error and denied states.
- Test cancellation, denied effects, unavailable presentation, path escapes and
  stale approval proposals as relevant; use deterministic fakes for inner layers.
- Run `cargo test -p <affected-huginn-package>` and `just huginn-architecture`.
  Verify affected CLI/TUI interactions using the appropriate terminal workflow;
  consult `docs/huginn/validation.md` for recorded evidence and platform limits.
