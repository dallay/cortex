# Proposal: Autonomous Workspace Permissions and Platform Parity

## Intent

Provide Huginn with least-privilege, user-controlled authorization for host effects, including a predictable approval flow for workspace file writes. A user must be able to deny an effect, allow it once, or persistently allow it for the current workspace within a narrow capability/action/resource scope. After a user persistently grants matching workspace file writes, later matching writes in that workspace must not prompt again.

The change must keep authorization separate from process sandboxing and agent behavioral instructions. It must also preserve native filesystem boundaries, administrator control, granular MCP authorization, three-OS parity goals, and accessible mouse and approval interactions. Current code does not yet provide the requested persistent workspace grant or the required verified policy sources.

## Scope

### In Scope

- Provide three effect decisions: deny, allow once, and allow persistently for the current workspace. A persistent grant MUST be limited to the canonical workspace root and the narrow capability, action, and resource it covers. Matching authorized writes in that workspace MUST NOT prompt again.
- Store persistent workspace grants only in a user-owned configuration or registry keyed by canonical root. Project-controlled files MUST NOT create, alter, or persist trusted permission state.
- Support configurable global/user policy rules and administrator rules as separate authorities. Administrator rules MUST only restrict and MUST override user grants when they conflict. Invalid or unverifiable administrator rules MUST fail closed.
- Preserve least privilege and `OnRequest` approval behavior. Define how a denied-by-profile effect can become eligible for a prompt without weakening administrator restrictions or silently expanding a grant.
- Keep authorization independent from sandbox selection and enforcement. Approval MUST NOT disable or broaden a sandbox. Preserve native filesystem path, symlink, capability, and resource checks at each effect boundary.
- Keep shell authorization and MCP authorization granular and separate. Do not let native file grants authorize shell or MCP effects; define MCP decisions at the server/tool/action/resource scope where supported.
- Preserve behavioral instructions as distinct from host capability authorization. Instructions such as “do not list environment variables” guide agent behavior but MUST NOT grant or replace host permissions; this change addresses host capability permissions only.
- Preserve and verify sandbox enforcement across macOS, Linux, and Windows. Do not claim platform parity based only on shared interfaces, compilation, stubs, or fail-closed behavior.
- Preserve mouse and keyboard interaction in TUI menus and approval flows. Approval UX MUST clearly show the requested action and resource, provide safe denial/cancellation, and make the three decisions understandable and reviewable.
- Reconcile existing tests with effective policy behavior. In particular, distinguish a safe default choice in a displayed prompt from whether the current `Restricted` policy should reach that prompt.

### Out of Scope

- YOLO, Exit, full-access, or other permissive authorization modes. Do not expand or expose the existing `FullAccess` profile as part of this change.
- Persisting grants in project-controlled files, broad workspace trust, or grants that implicitly include other workspaces, resources, actions, shell execution, or MCP calls.
- Treating behavioral instructions as authorization or process isolation.
- Unsandboxed execution fallback or any claim that approval itself provides sandboxing.

## Approach

Resolve effective authorization from distinct configurable global/user rules, user-owned canonical-root workspace grants, and administrator restrictions. Enforce administrator restrictions before honoring a grant. Treat missing, invalid, or unverifiable authority data as a denial. Keep grant identity bounded to the canonical workspace root and explicit capability/action/resource, and revalidate the requested effect against current policy and native filesystem boundaries before execution.

Use `OnRequest` to present the three user decisions only when policy permits an approval request. A successful persistent workspace decision is stored outside the project and suppresses later prompts only for matching operations. Maintain a separate authorization path for granular MCP effects. Keep process sandbox choice and enforcement independent from all approval decisions, and verify the declared restrictions on each target OS.

Implementation is blocked until a safe user-owned grant registry and a machine-managed source for administrator rules have been proven trustworthy and verifiable across macOS, Linux, and Windows. The current implementation has only session-scoped in-memory approvals, resolves `Restricted` writes to denial before prompting unless explicitly escalated, and has no user-owned persistent workspace-grant store or administrator policy source in the examined configuration path. Do not present the requested behavior or cross-platform verification as existing functionality.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/huginn/application/src/authorization.rs` | Modified | Replace session-only approval semantics with bounded one-time and persistent workspace decisions; enforce current policy before effects. |
| `crates/huginn/application/src/permission_policy.rs` | Modified | Resolve user/global policy separately from administrator restrictions and establish explicit precedence and prompt eligibility. |
| `apps/huginn/src/config.rs` and `apps/huginn/src/main.rs` | Modified | Load and compose user rules, the user-owned grant registry, and administrator rules without trusting project-controlled state. |
| `crates/huginn/presentation/src/terminal.rs` and approval state | Modified | Present deny, allow once, and allow for this workspace; preserve mouse/keyboard operation and safe review/cancel states. |
| Huginn native filesystem and sandbox adapters | Modified | Preserve effect-level path/resource checks and independently enforce platform sandbox boundaries. |
| `crates/huginn/runtime/src/mcp.rs` | Modified | Maintain separate, granular MCP authorization without inheriting native workspace grants. |
| `apps/huginn/tests/tui.rs` and Huginn policy/runtime tests | Modified | Verify decisions, no-repeat matching writes, authority precedence, boundary failures, UI behavior, and platform restrictions. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| A grant key or canonical-root check covers a different workspace or resource than intended. | High | Bind grants to canonical root and explicit capability/action/resource; revalidate at each effect boundary and test path/symlink edge cases. |
| A project-controlled file or unsafe registry grants permissions. | High | Keep grants in user-owned state, reject project-sourced grant data, and block implementation until registry ownership and integrity are proven across all target OSes. |
| Administrator rules are bypassed by user grants or are accepted from an unverifiable source. | High | Enforce restrictions after user rules/grants in precedence, fail closed, and block implementation pending verified machine-managed source design on all target OSes. |
| `Restricted` denies writes before the approval UI can present the requested choices. | High | Specify prompt eligibility and any bounded escalation explicitly; keep administrator denials final and reconcile the existing TUI test with policy behavior. |
| Approval is mistaken for sandboxing or a native file grant leaks into shell/MCP. | High | Keep authorization, sandbox enforcement, native filesystem permissions, and MCP decisions as separate controls with boundary tests. |
| Sandbox enforcement differs across operating systems. | High | Require real platform evidence before claiming parity; fail closed where enforcement or required authority verification is unavailable. |
| Approval choices are unclear or mouse interaction regresses. | Medium | Test keyboard and mouse flows, decision labels, focus/hit testing, resource details, and denial/cancellation states. |

## Rollback Plan

Revert affected policy, persistence, and presentation slices independently. If registry ownership, administrator source verification, or a platform enforcement check fails, disable persistent grants or the affected capability/platform and return an observable denial; do not fall back to project state, session-only behavior presented as persistent, or unsandboxed execution. Remove grants that exceed the effective policy. Rollback cannot reverse effects that already occurred.

## Dependencies

- Prove a safe user-owned workspace-grant registry and canonical-root identity on macOS, Linux, and Windows.
- Prove a machine-managed administrator-rule source and its ownership, integrity, and restriction-only semantics on all three operating systems.
- Resolve the `Restricted`/`OnRequest` prompt-eligibility contract without weakening least privilege or administrator restrictions.
- Provide platform-capable environments for tests that demonstrate real sandbox and filesystem restrictions.
- Reconcile current specifications, design, task breakdown, and UI test expectations with this proposal before implementation.

## Success Criteria

- [ ] Each eligible effect offers deny, allow once, or allow persistently for the current workspace, with the persistent decision bounded to canonical root and narrow capability/action/resource.
- [ ] After a persistent workspace write grant, subsequent matching writes in that workspace do not prompt again; non-matching writes and other workspaces remain unauthorized unless separately approved.
- [ ] Persistent grants exist only in verified user-owned state; project-controlled files cannot grant or modify permissions.
- [ ] Configurable global/user rules and administrator rules remain distinct; administrator rules only restrict and override conflicting grants.
- [ ] Registry and administrator-rule source integrity and verification are demonstrated on macOS, Linux, and Windows before implementation is considered unblocked.
- [ ] `OnRequest`, least privilege, and fail-closed behavior remain intact; no permissive mode is introduced.
- [ ] Approval does not alter sandbox selection, and native filesystem boundaries remain independently enforced on all three operating systems before parity is claimed.
- [ ] MCP remains granular and separate from native workspace grants; behavioral instructions do not act as host permissions.
- [ ] Mouse and keyboard approval flows are usable, reviewable, and tested, including safe denial and cancellation.
