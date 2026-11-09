# Tasks: Effective Workspace Permissions and Platform Parity

## Review Workload Forecast

| Field | Value |
|---|---|
| Review budget | 400 changed lines; high-risk multi-surface security change |
| Estimated workload | High; three OS backends, policy, filesystem, MCP, TUI, CI/docs, and new dependency evaluation |
| Decision needed before apply | No; complete macOS/Linux/Windows parity remains the target |
| Chained PRs recommended | Yes |
| 400-line budget risk | High |
| Chain strategy | `feature-branch-chain`; integration base `integration/huginn-autonomous-workspace-permissions`, trunk `main` |
| Work-unit balance | Six dependency-ordered, reviewable deliverables; each carries behavior, tests, and relevant documentation. New `private-fs` evaluation increases review load; forecast remains High. |

Decision needed before apply: No
Chained PRs recommended: Yes
400-line budget risk: High

Each OS backend requires real enforcement evidence; common interfaces, fakes, compilation, and fail-closed stubs are not parity. Fail closed before effects whenever required policy or enforcement is unavailable. Preserve `unsafe_code = deny`; add no unsafe wrapper or lint relaxation.

### Feature-branch-chain metadata

| Position | `trunk` | `parent_branch` | `base` | `branch` | Tracking |
|---|---|---|---|---|---|
| 1 | `main` | `integration/huginn-autonomous-workspace-permissions` | `integration/huginn-autonomous-workspace-permissions` | `feature/huginn-workspace-autonomy-policy` (checked-out branch) | Local plan; no external issue |
| 2 | `main` | `feature/huginn-workspace-autonomy-policy` | same as parent | `feature/huginn-cross-platform-sandbox` | Local plan |
| 3 | `main` | `feature/huginn-cross-platform-sandbox` | same as parent | `feature/huginn-native-filesystem` | Local plan |
| 4 | `main` | `feature/huginn-native-filesystem` | same as parent | `feature/huginn-mcp-policy` | Local plan |
| 5 | `main` | `feature/huginn-mcp-policy` | same as parent | `feature/huginn-tui-mouse-menus` | Local plan |
| 6 | `main` | `feature/huginn-tui-mouse-menus` | same as parent | `feature/huginn-approval-ux` | Local plan |

## Phase 1: Policy and trust foundation — slice 1

- [x] 1.1 RED: Add application tests for policy precedence, `OnRequest`, explicit trust provenance, managed restrictions, missing approver, and bounded escalation. RED exposed missing policy types and an `Always` path that allowed an action without approval.
- [x] 1.2 GREEN: Implement typed policy settings, restrictive effective-policy resolution, explicit trust provenance, managed denial precedence, and bounded escalation in `crates/huginn/application/src/permission_policy.rs`.
- [x] 1.3 GREEN: Add restrictive config defaults and mapping in `apps/huginn/src/config.rs`; pass the immutable policy through shared interactive/headless composition in `apps/huginn/src/main.rs`.
- [ ] 1.4 RED: Add failing `HostPolicy` tests proving effective deny/approval decisions precede allowlists, prompts, and session grants; prove policy errors fail closed.
- [ ] 1.5 GREEN: Make `HostPolicy::approve_for` enforce the injected policy before existing authorization paths; do not enable effects whose trust or managed source is unverified.
- [ ] 1.6 REFACTOR: Rerun policy/composition/HostPolicy suites; verify approval cannot weaken isolation and policy errors never become implicit allow.
- [ ] 1.7 EVALUATE: Review `private-fs` **0.2.1 only** for security/audit status, version, license, targets, API/handle guarantees, and compatibility; resolve Huginn MSRV 1.89 versus workspace MSRV 1.81 using actual resolver/toolchain builds. Treat it as unapproved; preserve `unsafe_code = deny` without wrappers or lint changes.
- [ ] 1.8 EVALUATE: Build focused private-root and user-registry fixtures/probes for secure root acquisition, ownership/ACL/path checks, ancestor handling, no-follow/reparse rejection, canonical identity, and atomic registry read/write; run integration tests on macOS, Linux, and Windows. If any audit, MSRV, fixture, or OS gate fails, stop evaluation, do not adopt, and fail closed; choose no substitute without separate review.
- [ ] 1.9 GREEN: Only after successful, reviewed registry evaluation, integrate the approved adapter at `apps/huginn/src/config.rs`; keep registry outside project/config overrides, user-owned, atomically persisted, and reject fallback roots. Otherwise keep trust unavailable and fail closed.
- [ ] 1.10 BLOCKED: Do not implement machine-managed source loading until a secure end-to-end verifier exists for system/admin ownership, ACL/DACL, and same-handle no-symlink/reparse traversal on macOS, Linux, and Windows. `win-authorization-sys` alone is insufficient; macOS ACL handle inspection remains unresolved. Do not treat verifier failure as absence; keep affected effects fail-closed.

## Phase 2: Native sandbox and shell — slice 2

- [ ] 2.1 RED: Test zero spawns for absent, incompatible, and failing backends; approval must not change the profile. Add black-box enforcement probes for declared capabilities on all three OSes.
- [ ] 2.2 GREEN: Implement native macOS, Linux, and Windows backends and select/enforce them before spawn in `crates/huginn/runtime/src/tools.rs`; preserve timeout, cancellation, output limits, and descendant termination. No unsandboxed fallback.
- [ ] 2.3 REFACTOR: Review target dependencies, security, license, and `unsafe_code = deny`; rerun backend contract and failure tests. Do not claim an OS supported without its real enforcement evidence.

## Phase 3: Native filesystem policy — slice 3

- [ ] 3.1 RED: Test read/list/search/write/edit and registered tools for roots, capabilities, traversal, symlink changes, stale state, and verification failure; assert zero effects on denial.
- [ ] 3.2 GREEN: Enforce and immediately revalidate effective policy at native effect boundaries in `crates/huginn/runtime/src/tools.rs`; do not authorize by tool name or route builtins through shell.
- [ ] 3.3 REFACTOR: Consolidate checks without weakening effect boundaries; rerun focused application/runtime suites.

## Phase 4: Granular MCP policy — slice 4

- [ ] 4.1 RED: Test `auto`/`prompt`/`writes`/`approve`, missing/adversarial annotations, tool-server-default precedence, explicit full-access, and startup versus call approval.
- [ ] 4.2 RED: Test session remembrance identity/scope, cross-scope isolation, session expiry, and no persistent change; test narrow persistent approval, safe config edit/reload, ineligible sources, and write/reload failure.
- [ ] 4.3 GREEN: Implement policy and scoped session remembrance in `crates/huginn/runtime/src/mcp.rs` and `composition.rs`; implement only explicit eligible persistent amendments. Unknown `auto` metadata prompts; persistence failure reports unchanged settings and retains session choice only if explicitly selected.
- [ ] 4.4 REFACTOR: Verify integration and ensure remembered/persistent decisions never exceed policy; do not claim MCP isolation from shell sandboxing.

## Phase 5: Mouse support in all menus — slice 5 (independent)

- [ ] 5.1 RED: Test mouse/keyboard equivalence, hit-testing, outside clicks, focus, scrolling, overlays, resize, and protected/stale actions across selectable menus.
- [ ] 5.2 GREEN: Implement hit-testing against visible geometry and restore terminal mouse capture on exit, error, and cancellation.
- [ ] 5.3 REFACTOR: Run deterministic PTY regressions without sleeps or a second event reader.

## Phase 6: Approval UX — slice 6 (independent)

- [ ] 6.1 RED: Test action/resource/risk/full preview, narrow/resized terminals, pending review, premature clicks, missing/revoked approver, denial, cancellation, and no-op; assert no unauthorized execution.
- [ ] 6.2 GREEN: Present complete review, host-authorized choices, and clear non-execution feedback; presentation must not grant authority.
- [ ] 6.3 REFACTOR: Verify keyboard/Escape, focus, mouse, and deterministic PTY states without sleeps.

## Phase 7: Platform gates, docs, and superseding ADR

- [ ] 7.1 Update `apps/huginn/config.example.toml` and `docs/huginn/` for profiles, defaults, MCP modes, platform capabilities, and limitations; make no unverified parity claim.
- [ ] 7.2 Add blocking PR CI tests on macOS/Linux/Windows for real backend enforcement and unavailable/failing zero-spawn behavior; include Huginn crates on Windows.
- [ ] 7.3 Run affected Huginn tests, integration/PTY suites, `just huginn-architecture`, fmt, relevant clippy, dependency/lockfile checks, and real-OS probes. Missing runners/capabilities are blocked evidence, not passes; release requires all three OS gates.
- [ ] 7.4 Add `docs/huginn/adr/0018-effective-workspace-permissions-and-platform-sandboxing.md`, explicitly superseding conflicting ADR-0006/0007 decisions without editing historical ADRs.

## Technical blockers

- Machine-managed configuration and effects requiring its verified absence remain blocked until the complete safe three-OS verifier and real-OS tests exist. `win-authorization-sys` supplied-handle inspection is insufficient; safe macOS ACL handle inspection is unresolved.
- User trust remains unavailable unless `private-fs` 0.2.1 passes its audit/version/MSRV evaluation, user-registry fixture tests, all three OS integration tests, and security review. Any failure stops adoption; no unsafe wrapper or lint relaxation is permitted.
- Any missing/failing sandbox backend or OS evidence blocks that OS's support/parity claim and the final release; shell remains fail-closed.
