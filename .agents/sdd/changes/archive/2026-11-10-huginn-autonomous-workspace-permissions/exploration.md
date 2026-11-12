# Exploration: macOS workspace sandbox for Huginn shell execution

## Executive summary

The user explicitly rejected the App Sandbox POC and authorized “COPIA COMO LO HACE CODEX”. Close and abandon the signed App Sandbox prototype; its files and results are not evidence of sandbox enforcement. The relevant Codex pattern is a dynamically generated Seatbelt (SBPL) deny-default profile passed to the absolute `/usr/bin/sandbox-exec`, wrapping the actual shell command. This is technically compatible with the current Rust `run_shell` boundary without a new Rust dependency or a signing/bundling requirement, but Apple deprecates `sandbox-exec`/Seatbelt interfaces. Treat this as an opt-in, macOS-only backend with clear deprecation risk, fail-closed startup, no persistent grants, and separate from approval policy. This architecture pivot invalidates the old proposal/spec/design/tasks and requires explore → propose → spec → design → tasks again before apply.

## Current state and evidence

### Huginn execution path

- `crates/huginn/runtime/src/tools.rs::run_shell` launches `/bin/sh -c <command>` with workspace `current_dir`, clears environment then restores only the host `PATH`, null stdin, piped stdout/stderr, and no inherited terminal. It wraps the child with `process_wrap::tokio::{ProcessGroup::leader, KillOnDrop}`. Cancellation and timeout select around `wait`; `start_kill` is then called and output readers are bounded and drained. `process-wrap` is already a runtime dependency. Sandbox wrapping must preserve this lifecycle and apply to the shell and all descendants.
- A minimal invocation shape is `/usr/bin/sandbox-exec -p <generated-profile> /bin/sh -c <command>` (prefer a securely created profile file or a supported stdin/profile mechanism after validation; never construct shell text by interpolating unescaped user paths). Set cwd and the explicitly selected environment on the sandbox-exec process so they are inherited by the shell. Any failure to generate/validate the profile, locate/execute sandbox-exec, or install the profile MUST return an error; it MUST NOT retry unsandboxed.
- Current shell tool metadata explicitly says “not sandboxed”; `apps/huginn/config.example.toml` states “No sandbox is provided.” `apps/huginn/src/config.rs` has permission profile/approval settings but no sandbox setting. Permission approval and technical enforcement are distinct: approving a command authorizes this operation; only OS policy constrains its filesystem/network effects. Neither approval nor `Guardrails` proves isolation.
- `crates/huginn/runtime/Cargo.toml` already has `process-wrap`, Tokio, `libc`; macOS target dependencies exist only for keyring. Adding a Rust sandbox dependency is unnecessary for this backend. The implementation can live behind `#[cfg(target_os = "macos")]`, with a platform-neutral interface/fail-closed unsupported implementation if needed. Ordinary source builds on all platforms should remain possible without requiring sandbox-exec to exist at build time.

### Codex primary-source comparison (current `openai/codex` main)

- `codex-rs/sandboxing/src/seatbelt.rs` uses the fixed absolute `/usr/bin/sandbox-exec` to avoid PATH injection. It resolves filesystem policy roots, normalizes paths, generates filesystem rules, joins base/network/platform rules, and builds command arguments. It does not treat a generic profile as sufficient: policy generation and path normalization are substantive security code.
- `codex-rs/sandboxing/src/seatbelt_base_policy.sbpl` starts with `(deny default)`, then selectively permits process exec/fork and same-sandbox signalling/process info, `/dev/null`, constrained system queries, selected Mach lookups, and pseudo-TTY operations. This illustrates required runtime allowances, not a permission list to copy wholesale.
- `codex-rs/sandboxing/src/seatbelt_network_policy.sbpl` contains narrowly selected local platform services, not a blanket socket allowance. Codex composes network policy dynamically in `seatbelt.rs`; its proxy, Unix socket, network-allowance, and managed-network handling is not needed for Huginn's minimal offline profile.
- Codex constructs file read/write rules separately, supports exclusions and protected ancestors, and has explicit symlink normalization and glob-to-regex translation. This demonstrates why copying only a short allow-subpath rule would be unsafe. Codex is a reference implementation, not a platform stability guarantee or permission to import its broad defaults.
- Primary sources: [Codex Seatbelt generator](https://github.com/openai/codex/blob/main/codex-rs/sandboxing/src/seatbelt.rs), [base SBPL](https://github.com/openai/codex/blob/main/codex-rs/sandboxing/src/seatbelt_base_policy.sbpl), [network SBPL](https://github.com/openai/codex/blob/main/codex-rs/sandboxing/src/seatbelt_network_policy.sbpl). The inspected source was current `main` on 2026-11-09; pin a commit in later design evidence because `main` moves.
- Apple marks `sandbox-exec(1)` as deprecated, and the Apple `sandbox_init(3)` interface is deprecated. This approach relies on an unsupported/deprecated macOS facility and is vulnerable to OS-version behavior drift. Do not imply Apple endorsement, permanence, or equivalent coverage across macOS versions. Validate using the target macOS runner and clearly document support boundaries.

### Minimal profile direction (not a finished SBPL profile)

- Start deny-default. Allow only necessary process launch/fork and same-sandbox signalling for shell descendants; required runtime reads (system libraries, dyld, shell executable, locale/runtime resources) and narrowly required temp paths; workspace read/write; standard streams and only the minimum OS services proven necessary. Keep profile writable roots separate from read roots. Default network/socket access to denied; any later network exception requires an explicit policy decision and tests. Do not copy Codex's `/Applications`, broad system preferences, proxy exceptions, full-disk paths, or optional language-runtime permissions without demonstrating Huginn needs them.
- Path rules must handle SBPL literals/regex escaping, non-ASCII and metacharacter-containing paths, canonical roots, symlinks, aliases, missing roots, and rename races. `canonicalize` alone does not eliminate TOCTOU or symlink races. A profile based on pathnames may not provide descriptor/FD-level confinement for descriptors inherited by the child; validate the environment and ensure no unintended FDs are inherited. Process sandbox inheritance must be tested across `fork`/`exec`, shell subprocesses, and attempted escape via launching alternate interpreters/binaries.
- Avoid exposing a profile-file write/read race: use restrictive ownership/mode and a safe unique temporary file or another validated profile-input mechanism, remove it reliably, and deny the command access to unrelated host files. Ensure temp access does not accidentally reopen the filesystem broadly.
- No grants are persisted by this proposed sandbox backend. This exploration does not authorize persistent workspace permission grants or a grant registry. Do not equate “sandbox enabled” with an approval or with an `Allow for workspace` choice.

## Affected areas

- `crates/huginn/runtime/src/tools.rs` — shell wrapping, environment/cwd/stdio, cancellation, timeout and process-group lifecycle.
- `crates/huginn/runtime/Cargo.toml` — currently sufficient dependencies; no additional dependency appears necessary.
- `apps/huginn/src/config.rs`, CLI configuration and `apps/huginn/config.example.toml` — an explicit macOS-only enable setting would need validation and a default decision; currently none exists.
- `crates/huginn/runtime/tests/` — OS integration/adversarial tests for filesystem, network, descendants, timeout/cancellation and fail-closed launcher failures.
- `docs/huginn/README.md`, `docs/huginn/architecture.md`, ADR-0006/ADR-0007 and the superseded change artifacts — distinguish approval from OS enforcement and document deprecated macOS facility and platform coverage.
- `.github/workflows/ci.yml` — `test-multi` runs the full Rust workspace test suite on `macos-latest` on push to main/develop; this is the actual existing CI macOS test runner, but it is not a PR runner today. `build-darwin` only builds Rook targets. CI therefore needs a deliberate change in a later phase if adversarial tests are to block PRs. Local macOS tests can provide fast iteration; tests must skip only when sandbox-exec is unavailable with an explicit unsupported result, not report a pass.

## Approaches

1. **Codex-style Seatbelt profile behind explicit opt-in (recommended for the authorized macOS scope)** — generate a narrow deny-default profile and invoke the fixed absolute sandbox-exec around the current shell process group. Keep build-time dependency-free and fail closed when enabled.
    - Pros: Closely matches requested reference architecture; applies to shell descendants; works with existing CLI/TUI child-process architecture; no signing, app bundle, or new crate dependency.
    - Cons: Apple-deprecated interface; SBPL/path construction is security-sensitive; macOS drift and profile compatibility; some commands/runtime services may fail; does not cover non-macOS.
    - Effort: High.

2. **Signed App Sandbox host/helper POC** — old direction, explicitly rejected by the user. Close/abandon it; do not continue its prototype or count its output as sandbox proof.

3. **No sandbox / policy-only checks** — retain approval and shell guardrails without claiming process confinement.
    - Pros: No platform fragility and preserves existing source-build behavior.
    - Cons: Does not meet “copy as Codex” or constrain arbitrary approved shell commands.
    - Effort: Low, but not the requested architecture.

## Recommendation

Take the Codex-style, macOS-only Seatbelt route as the next SDD design subject, not as permission to implement yet. Add a configuration setting only after product acceptance; recommendation is explicit opt-in with default disabled, because `sandbox-exec` is deprecated and sandboxing can break existing commands. When enabled, absence/failure must block command execution and never fall back. Keep sandbox enforcement separate from HostPolicy approval, do not persist grants, and state that Linux/Windows enforcement is unresolved and outside this macOS-focused change. Revisit the minimum runtime allowlist with adversarial tests; do not transplant Codex's broad profile.

The SBPL path language, canonicalization/symlink semantics, inherited descriptors, fork/exec inheritance, interactive behavior, standard streams, command launch, environment, and OS-version compatibility are acceptance-critical. Huginn's current command is non-interactive (`stdin=null`), but the profile still needs enough TTY/runtime permissions only if tests or future execution modes require them. Preserve timeout/cancellation descendant cleanup under the sandbox process group. Add an adversarial macOS test matrix: workspace read/write succeeds; outside-home, sibling workspace, config/credential files and arbitrary temp locations are denied except designated temp; network TCP/UDP and Unix sockets denied; shell grandchildren and alternate execs remain confined; symlink escapes/renames and metacharacter paths do not widen access; cancellation/timeout kills descendants; malformed/failed profile and missing/failing sandbox-exec never run the shell unsandboxed.

## Risks and open decisions

- `sandbox-exec` is deprecated by Apple. MacOS major-version support policy and the willingness to depend on a deprecated security mechanism require explicit acceptance; if deprecated APIs are unacceptable, stop and select another backend rather than pretending this is supported.
- Users must decide whether the setting is opt-in/default-off (recommended) or mandatory/default-on on macOS. Default-on changes source CLI behavior; default-off means the product does not claim all shell commands are sandboxed.
- The current Huginn `NetworkSandboxPolicy` equivalent is absent. Recommend network deny by default inside the profile; do not infer that existing approval or guardrails already impose network isolation.
- SBPL path escaping and dynamic profile generation can introduce policy injection or overbroad rules. Treat generated profiles as security-critical and test unusual paths and profile parser failures.
- Canonicalization does not make path-based policy race-free; symlink aliasing, rename/replace races, inherited open FDs, environment-driven proxy behavior and descriptor passing require review and tests.
- Child process behavior, signal handling and process groups must be tested under `sandbox-exec`; process-wrap cleanup currently wraps `/bin/sh` directly, and wrapping another launcher can alter group leader semantics.
- macOS CI coverage is on push-only `test-multi` (`macos-latest`), not pull requests. Required PR enforcement needs workflow scope change in a later phase; local-only integration tests are insufficient as sole security evidence.
- Linux and Windows remain unresolved: Linux might use a separate Landlock/seccomp/Bubblewrap strategy, Windows a distinct restricted-token/AppContainer/job-object strategy, but neither has been researched or verified here. Do not claim cross-platform isolation or persistent-grant protection.
- The signed App Sandbox POC artifacts in `prototype-macos/` were removed on 2026-11-10 under explicit user authorization, after the pivot to the Seatbelt backend. They were never evidence of sandbox enforcement; do not cite prototype evidence as proof and do not restore the artifacts.
- Existing SDD proposal/spec/design/tasks describe a signed App Sandbox feasibility POC and are architecturally obsolete. Do not apply them. Re-run explore/propose/spec/design/tasks, preserving still-relevant authorization requirements while superseding the old approach.

## Ready for proposal

Yes, for a revised macOS-only Codex-style Seatbelt proposal, conditional on acceptance of the deprecated Apple interface and explicit opt-in/default policy. The user has already authorized the architecture choice but has not answered the acceptance-critical opt-in/default question. If the orchestrator considers that choice necessary to acceptance, pause and ask; otherwise propose default-off and record that assumption for explicit review. No production code, tests, config, docs or POC files were changed in this phase. No test suite was run; this was repository/source research only.

## Sources

- Codex (OpenAI), current Seatbelt profile generator: https://github.com/openai/codex/blob/main/codex-rs/sandboxing/src/seatbelt.rs
- Codex (OpenAI), deny-default base SBPL: https://github.com/openai/codex/blob/main/codex-rs/sandboxing/src/seatbelt_base_policy.sbpl
- Codex (OpenAI), network SBPL: https://github.com/openai/codex/blob/main/codex-rs/sandboxing/src/seatbelt_network_policy.sbpl
- Apple manual pages on macOS: `sandbox-exec(1)` and `sandbox_init(3)` (both identify the interface as deprecated).
- Repository: `crates/huginn/runtime/src/tools.rs`, `crates/huginn/runtime/Cargo.toml`, `apps/huginn/src/config.rs`, `apps/huginn/config.example.toml`, `.github/workflows/ci.yml`, `docs/huginn/architecture.md`, `docs/huginn/adr/0006-native-tools-and-trusted-mcp-stdio.md`, `docs/huginn/adr/0007-read-freely-approve-effects.md`.
