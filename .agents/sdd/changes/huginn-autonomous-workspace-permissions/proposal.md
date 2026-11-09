# Proposal: Autonomous Workspace Permissions and Platform Parity

## Intent

Replace conflicting permission rules with an effective, least-privilege model for autonomous workspace operations. Authorization and process sandboxing must remain separate controls: approval may authorize a bounded escalation, but it must not disable or broaden the sandbox. The confirmed platform goal is functional parity across macOS, Linux, and Windows, supported by verified enforcement on each platform rather than by a shared interface alone.

The change also defines granular MCP approval, fail-closed behavior at every uncertain permission boundary, and preserves mouse interaction in TUI menus and the approval experience as independent user-facing requirements.

## Scope

### In Scope

- Resolve effective approval policy, profiles, project trust, and managed restrictions with explicit authority provenance. Project trust MUST come only from a user-owned registry keyed by the canonical project root; files inside a project MUST NOT be able to grant trust to that project.
- Load managed restrictions only from protected system configuration. Managed configuration may restrict effective policy but MUST NOT grant additional authority. Invalid or unverifiable managed configuration MUST fail closed.
- Keep authorization and sandbox enforcement distinct. Run shell operations under the selected OS sandbox; allow only bounded escalation, and reject execution if the required sandbox cannot be enforced.
- Apply effective filesystem capabilities, roots, and exceptions to native file operations, with validation and revalidation at the effect boundary.
- Provide granular MCP policy by server and tool, including `auto`, `prompt`, `writes`, and `approve` modes, conservative handling of missing or incomplete annotations, explicit override precedence, and any documented full-access exception. Do not represent MCP annotations or approvals as process isolation.
- Implement and verify the required sandbox capabilities on macOS, Linux, and Windows before claiming parity. Keep shell execution unavailable on any platform where enforcement is missing, unsupported, or unsuccessful.
- Fail closed on invalid or unverifiable trust-registry and managed-configuration state, invalid configuration, unknown profiles or classifications, unverifiable resources, unavailable approval, stale requests, and sandbox failures. Return an observable outcome without executing the effect.
- Preserve mouse and keyboard interaction across selectable TUI menus, including correct hit testing, focus, and resize behavior. Preserve a reviewable approval experience with clear action, resource, risk, preview, and safe denial or cancellation states.
- Update relevant specifications, design, implementation tasks, documentation, and deterministic tests for policy, platform enforcement, MCP, and TUI behavior.

### Out of Scope

- Unsandboxed shell fallback, implicit approval, unrestricted grants based only on trust or workspace location, or using shell isolation as a substitute for native filesystem authorization.
- Claims that MCP servers inherit shell isolation without separately verified enforcement.
- Declaring platform parity from a common abstraction, compile checks, stubs, or fail-closed behavior alone.
- Removing or folding the TUI mouse and approval UX work into backend-only changes.

## Approach

Resolve an immutable effective policy from user configuration, a user-owned project-trust registry keyed by canonical project root, and protected system configuration. Project files and workspace location cannot establish trust. Managed restrictions can only reduce the user's effective permissions; reject invalid or unverifiable managed configuration and fail closed. Keep these authority sources explicit in policy provenance. Keep the authorization decision (allow, deny, or request a bounded approval) separate from the process profile and its enforcement. For shell, validate the requested profile against a platform-specific sandbox backend before process creation; approval must not change that profile. For native file operations, enforce filesystem capabilities and verified roots independently of shell sandboxing. Resolve MCP policy per call using server/tool overrides and conservative metadata handling, without implying server isolation. Verify equivalent observable restrictions on macOS, Linux, and Windows, and retain mouse/keyboard TUI interaction and approval UX as independently testable deliverables.

Deliver and verify real sandbox enforcement separately on macOS, Linux, and Windows. A platform may be reported as supported only after tests on that platform demonstrate the declared restrictions. Missing or failed enforcement must prevent process creation. Replan implementation slices around effective policy, the three platform backends, native filesystem authorization, MCP policy, TUI mouse support, approval UX, and platform evidence/documentation. Keep mouse support and approval UX independently testable and releasable where practical.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `.agents/sdd/changes/huginn-autonomous-workspace-permissions/` | Modified | Reconcile specifications, design, and tasks with the confirmed cross-platform goal and separated controls. |
| Huginn application policy and configuration | Modified | Resolve effective profiles, approval policy, trust provenance, managed restrictions, and bounded escalation. |
| Huginn project trust and managed configuration | Modified | Store project trust in a user-owned registry keyed by canonical project root; read managed restrictions from protected system configuration and permit them only to restrict effective policy. |
| Huginn runtime shell and filesystem tools | Modified | Enforce OS sandboxing before process creation and apply independent native filesystem authorization. |
| Huginn runtime MCP integration | Modified | Apply granular per-server and per-tool approval modes and explicit precedence. |
| Huginn presentation and TUI | Modified | Preserve mouse and keyboard interaction in menus and improve review, approval, denial, and cancellation feedback. |
| Huginn tests and documentation | Modified | Verify behavior independently on macOS, Linux, and Windows; document only capabilities supported by evidence. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| A backend cannot enforce the required profile on one or more target platforms. | High | Require real-platform enforcement evidence; fail closed and do not claim parity where evidence is absent. |
| Approval is mistaken for a sandbox change, allowing an escalation to broaden unrelated access. | High | Keep authorization outcomes and process profiles separate; test that approval never removes sandbox restrictions. |
| Project-controlled data or an unverified registry path grants project trust, or managed configuration grants authority. | High | Accept trust only from the user-owned canonical-root registry; accept managed policy only from protected system configuration, enforce restriction-only semantics, and fail closed when either source is invalid or unverifiable. |
| Missing or misleading MCP annotations result in unsafe automatic execution or excessive prompts. | Medium | Use the specified mode semantics, conservative treatment of unknown metadata, explicit overrides, and observable decisions. |
| Native file tools remain outside shell isolation and bypass filesystem policy. | High | Enforce capabilities and verified roots at each native effect boundary and test adversarial paths and stale state. |
| Cross-platform security work exceeds one reviewable delivery unit or hides regressions in TUI behavior. | High | Use dependency-ordered, independently verifiable slices; retain separate mouse and approval UX work. |

## Rollback Plan

Revert affected slices independently. If a platform backend cannot enforce the selected policy, disable shell execution on that platform and return an observable error; do not restore an unsandboxed fallback. If policy or MCP behavior regresses, restore the restrictive decision path and remove grants that exceed the effective policy. Preserve the TUI mouse and approval UX requirements during backend rollback. Rollback cannot reverse effects that already occurred.

## Dependencies

- Select and review suitable sandbox backends for macOS, Linux, and Windows, including their enforceable capabilities and host requirements.
- Define effective profile, override, managed-restriction, and trust-provenance resolution before implementing enforcement, including canonical project-root identity, registry ownership, protected managed configuration, and restriction-only semantics.
- Reconcile the existing specifications, design, and task breakdown with the confirmed platform goal and independently deliverable TUI work.
- Provide platform-capable environments for real enforcement tests; missing evidence blocks a support or parity claim.

## Success Criteria

- [ ] Authorization and sandboxing are represented and enforced as separate controls; approval does not disable or broaden the sandbox.
- [ ] Project trust is sourced only from a user-owned registry keyed by canonical project root; project files cannot grant trust.
- [ ] Managed restrictions come only from protected system configuration, can only restrict effective policy, and fail closed when invalid or unverifiable.
- [ ] Shell execution is sandboxed on macOS, Linux, and Windows, with platform-specific tests demonstrating the declared enforcement before parity is claimed.
- [ ] Missing or unsuccessful enforcement prevents process creation and returns an observable failure.
- [ ] Native filesystem operations enforce the effective capabilities and verified roots independently of shell execution.
- [ ] MCP implements the four approval modes with explicit server/tool precedence and conservative handling of unknown annotations, without claiming isolation.
- [ ] Unavailable approval, invalid policy, unverifiable resources, and stale requests fail closed.
- [ ] TUI menus preserve mouse and keyboard interaction, and approval UX preserves complete review, clear feedback, and safe denial or cancellation.
- [ ] Documentation and specifications state platform support only where real enforcement evidence exists.
