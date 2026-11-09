# Tasks: Effective Workspace Permissions and Platform Parity

## Review Workload Forecast

| Field | Value |
|---|---|
| Review budget | 400 changed lines |
| Estimated workload | High: native sandbox backends, adversarial OS tests, grant storage, filesystem, MCP, TUI, CI, and docs |
| Chain strategy | `feature-branch-chain`; trunk `main`; parent/base `integration/huginn-autonomous-workspace-permissions`; branch `feature/huginn-workspace-autonomy-policy`; position 1 |
| Work-unit balance | Dependency-ordered slices: policy; sandbox/isolation proof; gated storage/grants; filesystem/MCP/TUI; final CI/docs. Each slice includes focused verification. |

Decision needed before apply: Yes
Chained PRs recommended: Yes
400-line budget risk: High

Before any macOS sandbox implementation or macOS parity claim, the user must choose one option: (A) approve a temporary `sandbox-exec(1)` exception despite deprecation and removal risk; (B) fail closed or disable shell on macOS until a supported mechanism exists, with no three-OS parity; or (C) investigate/redesign for signed App Sandbox packaging and workspace access, a larger change with unproven feasibility. Apple marks `sandbox-exec(1)` and `sandbox_init(3)` deprecated; the investigated dynamic options rely only on deprecated APIs. Until the user chooses, the macOS sandbox task is blocked. Preserve fail-closed behavior, keep grants inactive and unloaded until real isolation probes pass independently on each OS, and do not weaken `unsafe_code = deny` or introduce YOLO, Exit, or other permissive modes.

## Phase 1: Policy and one-operation decisions — completed

- [x] 1.1 RED: Test typed policy, profile outcomes, managed restrictions, trust provenance, and bounded escalation.
- [x] 1.2 GREEN: Add effective-policy resolution and restrictive defaults in `permission_policy.rs` and `apps/huginn/src/config.rs`.
- [x] 1.3 GREEN: Pass immutable policy through interactive/headless composition in `apps/huginn/src/main.rs`.
- [x] 1.4 RED: Test that prompt-eligible out-of-profile native writes reach the prompt while managed denial remains final.
- [x] 1.5 GREEN: Correct `HostPolicy::approve_for` and policy/action mapping without moving policy resolution into presentation.
- [x] 1.6 REFACTOR: Verify policy, composition, HostPolicy, TUI behavior, and unchanged sandbox selection.
- [x] 1.7 RED: Test distinct global/user rules, workspace grants, precedence, user prohibitions, and managed restrictions.
- [x] 1.8 GREEN: Implement policy authorities and precedence in `permission_policy.rs` and `authorization.rs`.
- [x] 1.9 REFACTOR: Verify policy provenance and that behavioral instructions do not affect host authorization.
- [x] 1.10 RED: Characterize operation-only `Allow once`, no durable state, and zero execution on `Deny`.
- [x] 1.11 GREEN: Preserve operation-scoped `Allow once` and denial in application/approval contracts.
- [x] 1.12 REFACTOR: Verify decisions do not widen scope or change the `Restricted` sandbox profile.
- [x] 1.13 EVALUATE: Assess `private-fs` 0.2.1 as a possible user-state candidate. It compiles in external scratch with Rust 1.89 (crate MSRV 1.88; Huginn package 1.89; workspace 1.81; pinned toolchain 1.99), but its security contract excludes same-effective-UID/`TokenUser` attackers and shell shares that identity. With no sandbox yet, outcome: **NO GO for active authorization today**. Adding this crate cannot authorize persistent grants. Re-evaluate only after sandbox gates.

## Phase 2: Native sandbox and isolation proof — slice 2

- [ ] 2.1 DECISION GATE — BLOCKED: Obtain the user's explicit choice of (A) temporary deprecated `sandbox-exec(1)` exception despite deprecation/removal risk; (B) fail closed or disable macOS shell until a supported mechanism exists, accepting no three-OS parity; or (C) investigate/redesign for signed App Sandbox packaging/workspace access, a larger change with feasibility unproven. Do not implement or claim a macOS sandbox before this choice; `sandbox_init(3)` is also deprecated and is not an investigated non-deprecated alternative.
- [ ] 2.2 RED: After the decision gate, add adversarial shell probes on each target OS for denial of read, list/enumerate, create, modify, and replace access to future grant state and every ancestor needed to access or replace it. The macOS probe/backend remains blocked until option A or C is explicitly selected; option B means no macOS sandbox implementation or parity claim.
- [ ] 2.3 GREEN: After platform mechanism decisions, implement only selected OS-specific sandbox backends and fail-closed pre-spawn enforcement in `crates/huginn/runtime/src/`; missing or failed isolation must produce zero shell spawns. No unsandboxed fallback.
- [ ] 2.4 GREEN: Add blocking Windows Huginn test CI coverage in `.github/workflows/ci.yml`; run real sandbox probes and Huginn tests on each supported OS. Do not treat compilation or metadata checks as evidence.
- [ ] 2.5 REFACTOR: Record per-OS runtime evidence for every denial operation and backend failure; keep grants dormant and no persistent choice available unless that OS passes every gate. Do not claim three-OS parity when macOS is disabled, unsupported, or unverified.

## Phase 3: Re-evaluate storage, then gated grants — slice 3

- [ ] 3.1 EVALUATE: After Phase 2 passes on the target OS, re-evaluate `private-fs` 0.2.1 API, threat boundary, license, targets, and package/workspace toolchain compatibility; select no store unless real-OS store security tests can pass independently.
- [ ] 3.2 RED: After isolation and store gates pass, test that missing/failed isolation does not open or honor storage, malformed/unavailable state fails closed, and project files cannot create or modify grants.
- [ ] 3.3 GREEN: Implement the user-owned store in `apps/huginn/src/config.rs` and gate its load/write and persistent UI choice on verified current-OS isolation; preserve observable fail-closed outcomes.
- [ ] 3.4 RED: Test exact canonical root + capability + action + resource matching, matching writes suppressing prompts, and mismatches prompting or denying under current policy.
- [ ] 3.5 GREEN: Implement exact grant identity and effect-time policy/isolation revalidation in `crates/huginn/application/src/authorization.rs` and `apps/huginn/src/main.rs`.
- [ ] 3.6 RED: Test listing/editing/revocation taking effect before the next authorization, including malformed and unavailable stores.
- [ ] 3.7 GREEN: Implement explicit grant management and immediate revocation; persistence failure never becomes authorization.
- [ ] 3.8 REFACTOR: Verify storage remains dormant before gates and project-controlled data never becomes an authority.

## Phase 4: Native filesystem boundaries — slice 4

- [ ] 4.1 RED: Test native read/list/search/write/edit roots, traversal, symlinks, stale resources, and zero effects on denial in Huginn runtime/application tests.
- [ ] 4.2 GREEN: Enforce and revalidate path, capability, and resource policy at native effect boundaries in `crates/huginn/runtime/src/tools.rs`; grants never bypass checks or route builtins through shell.
- [ ] 4.3 REFACTOR: Rerun boundary and platform suites; changed requests require fresh review or denial.

## Phase 5: MCP and TUI — slices 5–6

- [ ] 5.1 RED: Test MCP `auto`/`prompt`/`writes`/`approve`, annotation failures, server/tool precedence, startup/call approval, and no native-grant inheritance in `crates/huginn/runtime/src/mcp.rs`.
- [ ] 5.2 GREEN: Implement independent granular MCP decisions in `mcp.rs` and `composition.rs`; verify no shell-sandbox isolation claim.
- [ ] 5.3 RED: Test approval action/resource/risk/preview, permitted labels, keyboard/mouse equivalence, hit testing, resize, incomplete review, cancellation, revocation, and no-op feedback in `crates/huginn/presentation/` and `apps/huginn/tests/tui.rs`.
- [ ] 5.4 GREEN: Present only host-authorized choices; preserve review, safe denial/cancellation, focus, and deterministic terminal behavior.
- [ ] 5.5 REFACTOR: Run deterministic PTY tests without sleeps or a second event reader.

## Phase 6: Final gates and documentation

- [ ] 6.1 BLOCKER: Keep protected managed-source verification explicitly unresolved; never treat unverifiable policy as absent or mark this verifier complete.
- [ ] 6.2 Update `apps/huginn/config.example.toml` and `docs/huginn/` with policy/grant scope, revocation, sandbox gates, platform evidence, MCP separation, and unresolved managed verification; make no unsupported parity claim.
- [ ] 6.3 Run affected tests, PTY suites, `just huginn-architecture`, fmt, relevant clippy, dependency/lockfile checks, and real OS probes; missing platform evidence is blocked, not passed.
- [ ] 6.4 Add `docs/huginn/adr/0018-effective-workspace-permissions-and-platform-sandboxing.md`, superseding conflicting ADR-0006/0007 decisions without editing historical ADRs.

## Gates retained

- Sandbox/store work is not complete. Persistent grants require real per-OS proof that shell cannot read/list/create/modify/replace the state or its ancestors, plus independent store security evidence.
- Managed-source verification remains unresolved and blocks any effect requiring verified managed absence and all final parity/release claims.
- Keep native file-boundary, MCP, and TUI authorization separate. No project grant authority, unsandboxed fallback, `unsafe`, YOLO, or Exit mode.
