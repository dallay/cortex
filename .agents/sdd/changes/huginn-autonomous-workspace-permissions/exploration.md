# Exploration: Autonomous workspace permissions

## Executive summary

The required approval contract is explicit: `Allow once`, `Allow for this workspace`, and `Deny`. A workspace grant must be persisted in a user-owned source and keyed by the canonical workspace root plus a narrow capability/action. Once granted, covered writes must not prompt again. User/global rules and administrator rules remain separate; administrator rules may restrict and override grants but cannot expand them. Project files cannot create trusted permission state. Permission approval does not provide sandboxing, and grants remain bounded by capability and resource.

Current Huginn does not implement that contract. `HostPolicy` stores only volatile, session-scoped grants, while the effective policy can deny an action before presenting a prompt. The current default is `Restricted` with no configured escalation capabilities. The existing integration test that waits for `Reject` must be reconciled with this fail-closed authorization path; seeing `Reject` in a modal is not evidence that a denied capability should prompt.

## Current implementation evidence

### Authorization and prompt flow

- `crates/huginn/application/src/permission_policy.rs` defines `Restricted`, `Workspace`, and `FullAccess` profiles and `OnRequest` and `Always` approval modes. `EffectivePolicy::resolve` initializes `escalation_capabilities` as an empty set. `Restricted` allows only `FileRead`; `Workspace` allows `FileRead` and `FileWrite`.
- `EffectivePolicy::decide` returns `Denied` for a capability outside the selected profile unless `OnRequest`, an approver is available, and the capability is explicitly present in the escalation set. Managed denied capabilities take precedence over profiles and escalations.
- `crates/huginn/application/src/authorization.rs` maps `native.write_file` and `native.edit_file` to `FileWrite`. `HostPolicy::approve_for` evaluates the effective policy before constructing or calling the prompt. `AuthorizationOutcome::Denied` immediately returns `ApprovalDecision::Reject`; therefore the prompt receives no request and cannot offer `Allow once` or a persistent workspace choice. An allowlist does not bypass this check.
- Under the default `Restricted` profile and empty escalation set, a write is therefore rejected before a prompt. This is a deliberate consequence of the current policy implementation, not a TUI rendering issue.
- When a prompt is reachable, `HostPolicy` currently offers `Reject`, `ApproveOnce`, and, only for eligible native file writes with a non-sensitive absolute canonical path and a session ID, `ApproveForSession`. The remembered key is `(session_id, action, canonical_path)` in an in-memory `HashSet`. It is neither persistent nor workspace-scoped, and does not satisfy the requested contract.
- `apps/huginn/src/config.rs` exposes only profile and approval-mode values in `[permissions]`. Defaults are `Restricted` and `OnRequest`; `Config::effective_policy` currently resolves with no managed restrictions and default untrusted project trust. No user-owned persistent workspace-grant store or administrator policy source is present in this configuration path.
- The application composition in `apps/huginn/src/main.rs` installs `HostPolicy` with the resolved effective policy in both line and TUI paths. The test at `apps/huginn/tests/tui.rs:422-499` waits for the write approval modal, expects `Reject` to be selected by default, and presses Escape to deny. This test documents safe default selection for a displayed prompt, but it does not prove that a `Restricted` write ought to prompt. It appears inconsistent with the current `Restricted` + empty-escalation policy path, which rejects before `request_decision`; verify or update this integration contract in later phases rather than assuming either behavior from the test alone.
- The TUI presents `Reject`, `Approve once`, and `Allow (session)` in `crates/huginn/presentation/src/terminal.rs`. The persistent choice will require a distinct decision contract and UI label; it must not be implemented by relabeling the existing session grant.

### Required permission semantics and scope boundaries

- Provide exactly the requested user choices: `Allow once`, `Allow for this workspace`, and `Deny`. A persistent grant belongs in user-owned configuration/state, keyed by the canonical workspace root and the narrow capability/action it covers. Do not key it only by session, a broad profile, or a project-controlled file.
- After an eligible persistent workspace write grant, do not prompt again for writes covered by that grant. A grant must not silently include other actions, resources, workspaces, shell execution, or MCP calls.
- Keep configurable user/global rules distinct from administrator rules. Resolve administrator restrictions so they can restrict and override user grants. Do not let a user grant, project trust, or project-controlled file override an administrator denial.
- Do not add YOLO, Exit, full-access, or other permissive modes as part of this scope. The existing `FullAccess` profile is a code fact, not authorization to include or expand such a mode in this change.
- Behavioral instructions such as “do not list environment variables” are not host capability authorization. They are outside this change. Host authorization must continue to enforce actual capability/resource boundaries regardless of instruction text.
- Approval authorizes a bounded operation; it does not sandbox the process. Preserve existing native filesystem path/symlink checks and platform sandbox behavior. Project files, model output, tool metadata, and MCP content must not self-grant or persist trusted permissions.

### Sandbox, platform, and MCP evidence retained

- Huginn's instructions state that approval of shell or MCP does not sandbox the process and that native workspace path and symlink boundaries must be preserved. Authorization and execution isolation remain separate controls.
- The previously inspected executor and platform code establish separate sandbox implementations for macOS, Linux, and Windows. Linux has Bubblewrap and a configurable legacy Landlock path; macOS and Windows use separate enforcement paths. Filesystem execution fails if the requested sandbox cannot be selected. This supports retaining the cross-platform requirement, but not claiming equivalent enforcement without per-platform verification.
- Native file operations use executor filesystem context and policy; they are not interchangeable with shell or MCP operations.
- MCP currently has its own approval path. `crates/huginn/runtime/src/mcp.rs` prepares approval for each invocation; this does not establish process sandbox inheritance. Keep MCP policy distinct from native filesystem workspace grants unless a later specification explicitly defines a narrowly scoped MCP grant.

## Affected areas

- `crates/huginn/application/src/authorization.rs` — current pre-prompt denial, decision choices, and in-memory session grant key.
- `crates/huginn/application/src/permission_policy.rs` — default profiles, escalation set, and managed-denial precedence.
- `apps/huginn/src/config.rs` — current user permission configuration and missing administrator/persistent grant loading.
- `apps/huginn/src/main.rs` — composition of the policy in line and TUI modes.
- `crates/huginn/presentation/src/terminal.rs` and approval state — current labels and decision selection contract.
- `apps/huginn/tests/tui.rs` — expected prompt and default-denial behavior that must be checked against effective policy.
- Runtime filesystem and sandbox adapters on macOS, Linux, and Windows — preserve bounded resource checks and independent isolation during authorization changes.
- `crates/huginn/runtime/src/mcp.rs` — separate authorization boundary; do not accidentally apply file grants to MCP.

## Approaches

1. **Add a user-owned workspace-grant store alongside effective policy (recommended)** — retain profile and administrator-policy resolution, and consult a separately persisted grant keyed by canonical workspace root plus action/capability and bounded resource. Resolve administrator restrictions before honoring the grant. Offer the persistent choice only when the operation is eligible and the effective policy permits asking.
   - Pros: Matches the clarified semantics; separates global configuration, workspace decisions, and administrator restrictions; avoids project-controlled trust state.
   - Cons: Requires safe persistence, canonical workspace identity, migration/validation, and explicit policy-versus-grant precedence.
   - Effort: Medium to High.

2. **Expand the existing session grant into a persistent workspace grant** — replace the session key and in-memory store with a persistent user-owned store and workspace-root key.
   - Pros: Reuses the current prompt and native-file eligibility concept.
   - Cons: Risk of carrying forward session-specific assumptions; requires broader capability/resource validation and managed-rule precedence before persistence and reuse.
   - Effort: Medium.

## Recommendation

Prefer a separate user-owned workspace-grant store integrated with the effective authorization decision, rather than treating remembered approvals as profile settings or extending the session set without a distinct policy boundary. First resolve how the new `Allow for this workspace` choice interacts with `Restricted`: the current policy rejects writes before prompting unless a finite escalation is configured. Preserve fail-closed behavior for administrator denials and unavailable prompts. Canonicalize and bind the grant to the exact workspace and narrow action/resource; revalidate at use time. Keep shell/MCP, behavior instructions, and sandbox enforcement out of the grant unless separately specified. Reconcile the TUI integration test with this decision before implementation.

## Risks

- A broad or non-canonical key could cause a workspace grant to authorize writes in another workspace or to an unintended resource.
- Persisting grants in project-controlled files would permit self-granting; the store must remain user-owned.
- A grant lookup before administrator restrictions could bypass managed denials.
- The current default policy and the TUI test's expected prompt are inconsistent; implementation based on only one side could leave a misleading or failing test.
- Storing grants as global profile configuration could broaden permission beyond the selected workspace.
- Approval could be mistaken for isolation and allow out-of-bound process access unless the existing sandbox and native filesystem checks remain independent.
- Cross-platform sandbox mechanisms differ. Functional equivalence requires separate evidence for macOS, Linux, and Windows.
- Applying a native file grant to MCP, shell, or behavior instructions would widen the approved scope.

## Ready for proposal

Yes, after the proposal records the clarified three-choice UX, persistent user-owned workspace key, administrator-over-user precedence, the `Restricted`/escalation prompt question, and the test mismatch. The proposal must exclude permissive modes and behavior-rule authorization, and must preserve sandbox/resource boundaries and distinct MCP treatment.
