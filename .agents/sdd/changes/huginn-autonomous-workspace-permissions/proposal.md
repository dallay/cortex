# Proposal: Autonomous Workspace Permissions and Platform Parity

## Intent

Provide Huginn with least-privilege, user-controlled authorization for host effects. Users must be able to deny an effect, allow it once, or persistently allow it for the current workspace within a narrow capability/action/resource scope. Matching authorized writes in that workspace must not prompt again.

Persistent grants remain a required outcome, but they must not become an authorization authority that same-user shell processes can read or alter. Huginn shell processes share the user's UID on macOS/Linux or `TokenUser` on Windows and can access user files unless sandbox enforcement prevents it. Therefore, prove sandbox isolation first; only then activate persistent grant storage. Keep authorization separate from sandbox enforcement and behavioral instructions.

## Scope

### In Scope

- Provide deny, allow once, and allow persistently for the current workspace. Bound persistent grants to the canonical workspace root and narrow capability/action/resource. Matching writes must not prompt again.
- Retain persistent grants in user-owned state; project-controlled files must not create, alter, or persist trusted permission state. Do not activate or rely on this store until sandbox isolation has been proven.
- First implement and verify sandbox restrictions that deny shell read, write, replacement, and enumeration access to grant state on macOS, Linux, and Windows. Require real-platform adversarial tests for each OS before persistent grants become active.
- Treat `private-fs` 0.2.1 as unsuitable as a standalone authorization safeguard: its security contract excludes same-principal attackers. Compilation or file ownership checks alone do not prove shell isolation.
- Support configurable global/user policy and administrator rules as separate authorities. Administrator rules must only restrict and override conflicting user grants. Invalid or unverifiable administrator rules must fail closed.
- Preserve least privilege and `OnRequest` behavior. Define prompt eligibility for profile-denied effects without weakening administrator restrictions or silently expanding a grant.
- Keep authorization independent from sandbox selection and enforcement. Approval must not disable or broaden a sandbox. Preserve native filesystem path, symlink, capability, and resource checks at each effect boundary.
- Keep shell authorization and MCP authorization granular and separate. Native file grants must not authorize shell or MCP effects. Keep MCP decisions at the server/tool/action/resource scope where supported.
- Keep behavioral instructions distinct from host capability authorization. Instructions such as “do not list environment variables” guide agent behavior but must not grant or replace host permissions.
- Require Huginn Windows test and clippy parity gates; current Windows CI excludes Huginn. Do not claim parity from shared interfaces, compilation, stubs, or fail-closed behavior alone.
- Preserve mouse and keyboard approval flows, clear action/resource details, safe denial/cancellation, and understandable decisions. Reconcile the `Restricted` prompt-eligibility behavior with existing TUI expectations.

### Out of Scope

- YOLO, Exit, full-access, or other permissive authorization modes. Do not expand or expose the existing `FullAccess` profile.
- Project-controlled grant storage, broad workspace trust, or grants that include other workspaces, resources, actions, shell execution, or MCP calls implicitly.
- Treating behavioral instructions as authorization or process isolation.
- Unsandboxed execution fallback or any claim that approval itself provides sandboxing.

## Approach

Order delivery by security dependency. First establish sandbox controls and demonstrate on each supported OS that shell processes cannot read, write, replace, or enumerate the future grant state. Add real-platform adversarial tests and enable equivalent Huginn Windows CI coverage. Persistent grants remain inactive until these gates pass. A private user-owned store is not sufficient protection while same-principal shell processes can access it; `private-fs` 0.2.1 must not serve as a standalone authorization authority.

After isolation is proven, add persistent user-owned grants keyed by canonical workspace root and explicit capability/action/resource. Resolve effective authorization from distinct global/user rules, grants, and administrator restrictions; apply administrator restrictions before honoring grants. Fail closed on missing, invalid, or unverifiable authority data, and revalidate every effect against current policy and native filesystem boundaries. Use `OnRequest` for the three decisions only when policy permits prompting. Keep MCP authorization separate and preserve independent sandbox selection/enforcement.

Current Huginn approvals are session-scoped and in-memory. `Restricted` writes are denied before prompting unless explicitly escalated. The existing TUI test expects a displayed prompt, which must be reconciled with that policy path. Current Windows CI excludes Huginn test/clippy commands. Do not present any requested persistent behavior or platform verification as existing functionality.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| Huginn sandbox adapters and platform tests | Modified | Prove shell cannot read, write, replace, or enumerate grant state before enabling persistence; cover macOS, Linux, and Windows. |
| Huginn CI configuration | Modified | Add Windows Huginn test and clippy parity gates required for the security boundary. |
| `crates/huginn/application/src/authorization.rs` | Modified | Add bounded one-time and persistent workspace decisions only after isolation is proven; enforce current policy before effects. |
| `crates/huginn/application/src/permission_policy.rs` | Modified | Resolve global/user policy separately from restriction-only administrator rules and define prompt eligibility. |
| `apps/huginn/src/config.rs` and `apps/huginn/src/main.rs` | Modified | Load and compose user rules, user-owned grants, and administrator rules without trusting project state. |
| `crates/huginn/presentation/src/terminal.rs` and approval state | Modified | Present the three decisions and preserve mouse/keyboard operation and safe review/cancel states. |
| Huginn native filesystem adapters | Modified | Preserve effect-level path/resource checks independently of authorization and process isolation. |
| `crates/huginn/runtime/src/mcp.rs` | Modified | Maintain granular MCP authorization without inheriting native workspace grants. |
| `apps/huginn/tests/tui.rs` and Huginn policy/runtime tests | Modified | Verify policy, grant behavior, authority precedence, sandbox denial boundary, UI behavior, and platform restrictions. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Same-principal shell code reads or alters authorization state. | High | Keep persistence inactive until sandbox denial of read/write/replace/enumerate is proven with real-platform adversarial tests on all three OSes. |
| A grant key or canonical-root check covers an unintended workspace/resource. | High | Bind grants to canonical root and explicit capability/action/resource; revalidate at each effect boundary and test path/symlink cases. |
| Project state or an unsafe registry grants permissions. | High | Keep grants user-owned, reject project-sourced grant data, and gate activation on verified isolation. |
| Administrator rules are bypassed or accepted from an unverifiable source. | High | Keep administrator policy restriction-only, enforce its precedence, fail closed, and verify its source on all target OSes. |
| `Restricted` denies writes before the approval UI can present choices. | High | Specify prompt eligibility and bounded escalation; keep administrator denials final and reconcile the TUI test. |
| Windows CI continues to omit Huginn tests. | High | Add Windows Huginn test/clippy gates and require real Windows evidence before parity claims or grant activation. |
| Approval is mistaken for sandboxing or native grants leak into shell/MCP. | High | Keep authorization, sandboxing, native filesystem checks, and MCP decisions separate and test each boundary. |
| Approval choices or mouse interaction regress. | Medium | Test keyboard/mouse flows, labels, focus/hit testing, resource details, and denial/cancellation. |

## Rollback Plan

Revert sandbox, persistence, policy, and presentation slices independently. If any platform fails the isolation or authority verification gates, keep persistent grants inactive and return an observable denial for affected capabilities; do not fall back to project state, mislabel session-only behavior as persistent, or run unsandboxed. Remove grants that exceed effective policy. Rollback cannot reverse effects that already occurred.

## Dependencies

- Implement and prove sandbox denial of shell read/write/replace/enumerate access to grant state on macOS, Linux, and Windows before activating persistent grants.
- Provide real-platform adversarial test environments for all three OSes and add Huginn Windows CI test/clippy coverage, currently excluded.
- Prove a user-owned grant store and canonical-root identity after the isolation prerequisite; do not treat `private-fs` 0.2.1 as a standalone safeguard.
- Prove a machine-managed administrator-rule source with ownership, integrity, and restriction-only semantics on all target OSes.
- Resolve the `Restricted`/`OnRequest` prompt-eligibility contract without weakening least privilege or administrator restrictions.
- Reconcile current specifications, design, task breakdown, and UI test expectations before implementation.

## Success Criteria

- [ ] Sandbox tests on macOS, Linux, and Windows demonstrate that a shell process cannot read, write, replace, or enumerate grant state; persistent grants remain inactive until all platform gates pass.
- [ ] Huginn Windows test and clippy CI gates run and pass; parity is supported by real-platform evidence, not compilation alone.
- [ ] After isolation is proven, eligible effects offer deny, allow once, or allow persistently for the current workspace, bounded to canonical root and narrow capability/action/resource.
- [ ] After a persistent workspace write grant, matching writes in that workspace do not prompt again; non-matching writes and other workspaces remain unauthorized unless separately approved.
- [ ] Persistent grants exist only in user-owned state; project-controlled files and same-principal shell processes cannot grant or modify permissions.
- [ ] Global/user rules and administrator rules remain distinct; administrator rules only restrict and override conflicting grants.
- [ ] `OnRequest`, least privilege, and fail-closed behavior remain intact; no permissive mode is introduced.
- [ ] Approval does not alter sandbox selection; native filesystem boundaries remain independently enforced on all three OSes.
- [ ] MCP remains granular and separate from native workspace grants; behavioral instructions do not act as host permissions.
- [ ] Mouse and keyboard approval flows are usable and tested, including safe denial and cancellation.
