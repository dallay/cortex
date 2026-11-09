# Tasks: Effective Workspace Permissions and Platform Parity

## Review Workload Forecast

| Field | Value |
|---|---|
| Review budget | 400 changed lines |
| Estimated workload | High: policy, persistence, filesystem, MCP, TUI, three native OS backends, CI, and docs |
| Decision needed before apply | No; continue the approved feature-branch chain |
| Chained PRs recommended | Yes |
| 400-line budget risk | High |
| Chain strategy | `feature-branch-chain`; current branch `feature/huginn-workspace-autonomy-policy`, base `integration/huginn-autonomous-workspace-permissions` |
| Work-unit balance | Dependency-ordered behavior slices; each includes tests and relevant docs. Keep each slice independently verifiable and rollbackable. |

Decision needed before apply: No
Chained PRs recommended: Yes
400-line budget risk: High

Do not claim platform parity without real enforcement evidence on Linux, macOS, and Windows. Preserve `unsafe_code = deny`; no unsafe wrapper or lint relaxation. `private-fs` 0.2.1 remains an unevaluated candidate, not an adopted dependency.

## Phase 1: Policy foundations and workspace grants — current slice

- [x] 1.1 RED: Add application tests for typed policy settings, profile outcomes, managed restrictions, trust provenance, and bounded escalation; the initial tests exposed missing policy types and an unsafe `Always` path.
- [x] 1.2 GREEN: Add typed effective-policy resolution and restrictive defaults in `crates/huginn/application/src/permission_policy.rs` and `apps/huginn/src/config.rs`.
- [x] 1.3 GREEN: Pass immutable policy through interactive/headless composition in `apps/huginn/src/main.rs`.
- [x] 1.4 RED: Add a failing `HostPolicy` regression test proving ordinary out-of-profile native writes under `OnRequest` reach the prompt (including empty escalation configuration); managed denial remains final. Current TUI regression: write action is rejected before UI prompt.
- [x] 1.5 GREEN: Fix `HostPolicy::approve_for` and policy/action mapping so prompt eligibility is not pre-denied; keep policy resolution outside presentation and preserve the managed-denial precedence.
- [x] 1.6 REFACTOR: Run policy, composition, HostPolicy, and TUI tests; prove no allowlist/session shortcut or approval changes the selected sandbox.
- [ ] 1.7 RED: Test separate global/user rules and workspace grants, deterministic precedence, user prohibitions, and managed restrictions overriding grants.
- [ ] 1.8 GREEN: Implement distinct policy authorities and precedence in `permission_policy.rs` and `authorization.rs`.
- [ ] 1.9 REFACTOR: Verify policy provenance and that behavioral instructions do not affect host authorization.
- [ ] 1.10 RED: Test `Allow once` for one operation only, no durable state, and `Deny` for zero execution.
- [ ] 1.11 GREEN: Implement operation-scoped `Allow once` and denial in application/approval contracts.
- [ ] 1.12 REFACTOR: Verify neither decision widens scope or changes the sandbox.
- [ ] 1.13 RED: Test durable grant identity by canonical root/capability/action/resource; matching later writes prompt zero times; mismatched path/action/capability/workspace prompts or denies under policy.
- [ ] 1.14 GREEN: Implement `Allow for this workspace` and a user-owned store in `apps/huginn/src/`; accept only exact current-policy matches. Never read project grants.
- [ ] 1.15 REFACTOR: Verify narrow scope, prompt suppression, and no shell/MCP inheritance.
- [ ] 1.16 RED: Test grant listing/edit/revocation, immediate revocation, malformed/unavailable storage, and persistence-failure feedback.
- [ ] 1.17 GREEN: Implement explicit grant management and observable failures; failed persistence never becomes an allow.
- [ ] 1.18 REFACTOR: Verify edited/revoked state is applied before the next authorization decision.
- [ ] 1.19 EVALUATE: Assess `private-fs` 0.2.1 only as a user-state candidate: security, API, license, targets, and toolchain compatibility. No adoption without review and real-platform proof; preserve `unsafe_code = deny`.
- [ ] 1.20 RED: Test exact action/resource revalidation, canonical roots, symlink/stale-content changes, native-boundary enforcement, and unchanged sandbox.
- [ ] 1.21 GREEN: Enforce the tested sequence in `authorization.rs` and native adapters; grants cannot bypass effect checks.
- [ ] 1.22 REFACTOR: Rerun boundary tests and confirm changed requests require fresh review or denial.
- [ ] 1.23 BLOCKED: Keep managed-source verification unresolved. Never treat unverifiable policy as absent; fail closed until protected-source verification passes on all three OSes.

## Phase 2: Native sandbox and shell — slice 2

- [ ] 2.1 RED: Test zero spawns when a backend is absent, incompatible, or fails; add real enforcement probes for each OS and prove approval does not change the profile.
- [ ] 2.2 GREEN: Implement native macOS, Linux, and Windows backends and enforce before spawn in `crates/huginn/runtime/src/tools.rs`; preserve limits and provide no unsandboxed fallback.
- [ ] 2.3 REFACTOR: Review backend dependencies/security and rerun contract/failure tests; do not claim unsupported OS parity.

## Phase 3: Native filesystem — slice 3

- [ ] 3.1 RED: Test read/list/search/write/edit roots, capabilities, traversal, symlink/stale-state changes, matching and nonmatching grants, and zero effects on denial.
- [ ] 3.2 GREEN: Enforce and revalidate policy at native effect boundaries in `crates/huginn/runtime/src/tools.rs`; never authorize by tool name or route builtins through shell.
- [ ] 3.3 REFACTOR: Consolidate checks without weakening boundaries; rerun application/runtime suites.

## Phase 4: Granular MCP — slice 4

- [ ] 4.1 RED: Test `auto`/`prompt`/`writes`/`approve`, annotation failures, server/tool precedence, startup versus call approval, and no native-grant inheritance.
- [ ] 4.2 GREEN: Implement separate policy in `crates/huginn/runtime/src/mcp.rs` and `composition.rs`; retain existing scoped session semantics.
- [ ] 4.3 REFACTOR: Verify policy bounds and ensure no MCP isolation claim relies on shell sandboxing.

## Phase 5: Mouse support — slice 5

- [ ] 5.1 RED: Test mouse/keyboard equivalence, hit testing, focus, scrolling, overlays, resize, and protected/stale actions across selectable menus.
- [ ] 5.2 GREEN: Implement visible-geometry hit testing and restore mouse capture on exit, error, and cancellation.
- [ ] 5.3 REFACTOR: Run deterministic PTY regressions without sleeps or a second event reader.

## Phase 6: Approval UX — slice 6

- [ ] 6.1 RED: Test exact action/resource/risk/preview, all three labels, narrow/resized terminals, incomplete review, missing/revoked approver, denial, cancellation, and no-op feedback.
- [ ] 6.2 GREEN: Present only host-authorized choices and clear non-execution/persistence-failure feedback; presentation must not grant authority.
- [ ] 6.3 REFACTOR: Verify keyboard/Escape, focus, mouse, review completeness, and deterministic PTY states.

## Phase 7: Gates, documentation, and superseding ADR

- [ ] 7.1 Update `apps/huginn/config.example.toml` and `docs/huginn/` for rule precedence, grant scope/revocation, profiles, MCP, sandbox limits, and unresolved managed verification; make no unverified parity claim.
- [ ] 7.2 Add blocking CI tests on macOS/Linux/Windows for real backend enforcement and fail-closed zero-spawn behavior, including Huginn tests on Windows.
- [ ] 7.3 Run affected tests, PTY suites, `just huginn-architecture`, fmt, relevant clippy, dependency/lockfile checks, and real-OS probes. Missing runners are blocked evidence, not passes.
- [ ] 7.4 Add `docs/huginn/adr/0018-effective-workspace-permissions-and-platform-sandboxing.md`, superseding conflicting ADR-0006/0007 decisions without editing historical ADRs.

## Gates retained

- User-owned grant storage remains unavailable until ownership, protection, canonical identity, atomic updates, revocation, and all three OS tests pass. Any candidate dependency is conditional and must not be described as adopted before approval.
- Managed-source verification remains unresolved and blocks effects that require verified managed absence, as well as final parity claims.
- Each native OS sandbox is a separate deliverable requiring actual enforcement evidence. Fail closed on missing policy, verifier, approver, resource validation, or backend; do not fall back to project state or unsandboxed execution.
