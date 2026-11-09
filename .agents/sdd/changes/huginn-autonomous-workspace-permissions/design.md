# Design: Autonomous Workspace Permissions and Platform Parity

## Technical approach

Implement explicit, least-privilege host authorization in Huginn's existing core/application/runtime/presentation boundaries. Resolve user rules, explicit user-owned workspace grants, and verified administrator restrictions into one immutable policy at the application composition boundary. Keep behavior instructions, host authorization, native filesystem checks, process sandboxing, and MCP authorization separate.

Eligible effects expose `Allow once`, `Allow for this workspace`, and `Deny`. The persistent choice applies only to file writes for the canonical workspace root and the narrow capability/action/resource scope presented. Store it in verified user-owned state outside the project. Revalidate current policy, grant scope, resource identity, roots, stale content, and sandbox requirements at the effect boundary. Matching writes do not prompt again; a mismatch is resolved again by current policy and may prompt where escalation is permitted.

Managed policy remains a separate, restriction-only authority. A valid managed denial, invalid policy, failed source verification, invalid resource, or unavailable approver is final and prevents execution. A normal `Restricted` profile decision outside the profile is not itself a managed denial: with `OnRequest`, Huginn must reach the user decision when no explicit restriction bars escalation. An empty optional escalation list must not suppress that ordinary prompt. Approval authorizes only its reviewed effect scope and never changes or bypasses the required sandbox.

The present managed-source verifier is unresolved. `private-fs` 0.2.1 is only a conditionally reviewed candidate for secure per-user grant state, not an adopted dependency or proof of safety. Do not represent the verifier or three-OS parity as implemented. Final parity is blocked until each OS's actual enforcement and state-verification requirements pass real-platform tests.

## Architecture decisions

### Decision: Persist explicit scoped grants in user-owned state

**Choice:** Add a dedicated registry adapter at the `apps/huginn` configuration/composition boundary. Registry records are created only after the user explicitly selects `Allow for this workspace`. Key each record by canonical workspace root and explicit capability, action, and resource scope. Keep registry state separate from trust state and general policy. Provide user-visible listing, editing, and revocation; revocation takes effect before subsequent effect authorization. Reject project files, project metadata, model output, instructions, and workspace location as grant authorities.

**Alternatives considered:** Project-local grants, broad workspace trust, implicit persistence after `Allow once`, or persistence in generic policy settings.

**Rationale:** User-owned state preserves explicit choice and prevents repository-controlled content from minting or widening host authority. A narrow record permits exact matching without turning workspace membership into trust.

### Decision: Treat `private-fs` 0.2.1 as a gated candidate only

**Choice:** Evaluate `private-fs` 0.2.1 only for the per-user grant registry. Do not adopt it unless API/security review, compatibility checks, and real macOS/Linux/Windows tests prove it meets the exact state boundary. Do not use it for administrator-owned configuration. Verify compatibility with the workspace and Huginn Rust toolchain constraints before any dependency change. Its stated security review is conditional; it is not approval.

**Alternatives considered:** Adopt immediately based on its API description, use it for both user and managed state, or infer protection from path placement and metadata alone.

**Rationale:** The dependency's current evidence does not establish the complete guarantees required here, particularly for managed roots and all platform paths. Consumer-side unsafe code or weakened lint rules are not acceptable substitutes.

### Decision: Keep managed restrictions independent and restriction-only

**Choice:** Load administrator rules from a protected system source through a separately reviewed verifier. Accept only validated restrictions. Managed rules may reduce effective permission, must override conflicting user grants, and must never grant trust or authority. Treat present-but-invalid, unreadable, or unverifiable managed state as an observable fail-closed error; never interpret verifier failure as verified absence.

**Alternatives considered:** Load managed policy from user/project configuration, merge all sources as equivalent rules, or silently ignore invalid fields.

**Rationale:** Distinct provenance and restriction-only composition prevent user grants or untrusted state from overriding administrator controls. The verifier, protected locations, and end-to-end platform coverage remain unresolved and block final parity.

### Decision: Separate policy eligibility from presentation and preserve the normal decision

**Choice:** The application policy produces distinct outcomes for allowed, prompt-eligible, and final-denial effects, with reasons/provenance sufficient for the host and UI. Under `OnRequest`, an out-of-profile capability is prompt-eligible unless an explicit effective restriction (including a managed denial) prohibits it. An absent or empty optional escalation list does not turn this normal user decision into a pre-prompt denial. A configured restrictive rule may prohibit escalation. `Always` retains its configured approval semantics. When the approver is missing, revoked, or fails, do not execute.

**Alternatives considered:** Require every out-of-profile capability to appear in a non-empty allowlist, interpret `Restricted` as an unconditional pre-prompt denial, or let the UI decide whether policy permits escalation.

**Rationale:** Policy determines whether the choice is available; presentation collects but does not create authority. This resolves prompt eligibility without weakening explicit user or managed restrictions.

### Decision: Enforce an ordered host authorization sequence

**Choice:** For every effect, use this sequence:

1. Prepare and classify the exact capability, action, and resource; resolve/canonicalize the workspace and resource. Reject invalid or unresolvable identity.
2. Load the current immutable effective policy and required authority state. Invalid/unavailable policy or unverifiable required managed state fails closed.
3. Apply managed restrictions first. A managed denial is final and cannot be prompted around.
4. Revalidate native roots, symlinks, resource identity, operation content, and other effect-specific constraints. A stale or out-of-root request is rejected or prepared for fresh review; no grant can bypass these checks.
5. Resolve the user policy. Honor an explicit persistent grant only if canonical root and every capability/action/resource dimension match, the grant registry is verified, and current policy still permits it. Do not prompt for a matching valid grant.
6. If the effect is in policy, continue under the selected sandbox and native boundary checks. If it is outside user policy but `OnRequest` permits escalation, request approval from an available valid approver—even when the optional escalation list is empty. If effective policy explicitly prohibits escalation, deny without prompting.
7. Present only eligible choices: `Deny` and `Allow once` for an eligible effect; include `Allow for this workspace` only for supported native file-write actions with a verified canonical workspace and registry. Record only the explicit persistent choice. A denial, cancellation, timeout, unavailable approver, or persistence failure never becomes an allow.
8. Revalidate policy, scope, resource, content, and sandbox immediately before effect. Execute only the approved operation under the unchanged required sandbox. A change after review requires fresh review/authorization or denial.

This ordering is independent of whether the effect originates in shell, native file tools, or MCP; each family retains its own scope and enforcement rules. Workspace grants never authorize shell or MCP.

**Alternatives considered:** Prompt before managed policy, treat approval as profile elevation, reuse one broad grant across effects, or rely on preparation-time checks alone.

**Rationale:** The ordering makes authority precedence explicit and prevents user choices from bypassing managed denial, sandbox enforcement, roots, stale-resource validation, or approver availability.

### Decision: Preserve platform-specific sandbox enforcement and block parity claims

**Choice:** Maintain a platform-neutral observable profile contract with native macOS, Linux, and Windows enforcement implementations. Each backend must prove the required filesystem/process restrictions on its target OS and reject unsupported or failed enforcement before spawn. A shared interface, compile success, fail-closed stub, or equivalent profile name is not parity evidence. Do not provide an unsandboxed fallback. Three-OS parity remains blocked until all three real-platform gates pass.

**Alternatives considered:** Treat common interfaces as parity, ship unsupported backends as final behavior, or allow approval to bypass isolation.

**Rationale:** Authorization and isolation solve different problems. Native enforcement evidence is required for each OS, and approval must not widen the baseline sandbox.

### Decision: Keep MCP and behavior instructions separate

**Choice:** Preserve MCP's own server/tool/action/resource policy and existing `auto`, `prompt`, `writes`, and `approve` semantics. Do not inherit native file-write grants into MCP. Keep behavioral instructions (including instructions about environment-variable enumeration) as agent guidance, not host capability authorization. Preserve independent MCP server-startup and invocation controls.

**Alternatives considered:** Share workspace grants with MCP, infer MCP security from annotations, or use behavioral instructions as permission controls.

**Rationale:** These mechanisms have distinct trust boundaries and scopes. Combining them would create unintended authorization and sandbox claims.

### Decision: Preserve complete TUI interaction and review

**Choice:** Keep mouse and keyboard operation for all interactive TUI menus and approval flows. Show the requested action, exact resource, risk, and complete required preview. Expose only host-authorized decisions. Preserve focus, hit testing, resize behavior, cancellation, safe denial, and explicit no-op feedback.

**Alternatives considered:** Keyboard-only menus, click-to-approve shortcuts, or assuming a rendered prompt proves complete review.

**Rationale:** Input parity and complete review are separate requirements from the authorization algorithm; neither may bypass policy or approve stale requests.

## Data flow

```text
User policy + verified user grant registry + managed source status
  -> apps/huginn composition loads and validates independent authorities
  -> application resolves immutable EffectivePolicy and provenance
  -> effect preparation identifies capability/action/resource and canonical roots
  -> HostPolicy applies managed restrictions, current policy, and exact grant match
       | final denial / invalid state -> observable non-execution
       | permitted matching grant -> continue without prompt
       | prompt-eligible and approver available -> TUI collects eligible choice
       | otherwise -> observable denial/unavailable-approver outcome
  -> immediately before effect: revalidate policy, grant, roots, resource, content
  -> runtime native file / sandboxed shell adapter enforces independent OS boundary
  -> effect executes only within the approved scope and required sandbox

MCP -> separate server/tool policy -> scoped MCP approval -> MCP runtime
     (never inherits native workspace grants or shell sandbox claims)
```

Persist only an explicit `Allow for this workspace` user choice. Registry failure does not convert that choice to `Allow once` unless the user separately selects `Allow once`; report that no persistent grant was created. Registry editing/revocation uses the same verified user-owned storage path. A grant match suppresses later prompts only after current managed policy, user policy, and effect-boundary checks succeed.

## Interfaces / contracts

Names below are conceptual; reconcile them with existing types during implementation.

- **`EffectivePolicy` (application):** immutable user policy, verified managed restrictions/status, approval mode, allowed/prompt-eligible/denied outcome, provenance, and profile. Represent final managed denial separately from ordinary out-of-profile prompt eligibility.
- **`WorkspaceGrant` (application contract):** canonical workspace root plus narrow capability, action, resource scope, and explicit user-choice provenance. No trust or generic “workspace trusted” field. Exact equality or a documented safe containment relation must be defined for resource matching; lexical path-prefix matching is insufficient.
- **`WorkspaceGrantStore` (apps/config adapter):** verified user-owned load, atomic explicit write, enumerate/edit/revoke. Project-controlled inputs are never accepted as grant writes. Reject malformed state, unsafe paths, links/reparse points, ownership/protection failures, or unresolved canonical roots. Storage implementation is unresolved pending candidate evaluation.
- **`ManagedPolicySource` (apps/config adapter):** distinct outcomes for verified absence, verified restriction-only configuration, and verification/read/parse error. Its cross-platform protected source verifier and locations are unresolved. Until verified, do not claim successful absence or policy loading where the source is required.
- **`HostPolicy` (application):** apply the ordered authorization sequence and return a structured decision. Presentation receives allowed decisions but does not own grants or policy.
- **`ApprovalDecision`:** user-visible decisions include `Allow once`, `Allow for this workspace`, and `Deny`. Map to existing enum/API only after ensuring the persistent choice cannot be confused with session remembrance. `Allow once` is one prepared operation and is never persisted.
- **`SandboxBackend` (runtime):** target-specific enforcement contract. Must not expose an unsandboxed fallback or accept authorization decisions as a reason to widen the selected profile.
- **MCP policy:** retain separate server/tool-level mode and scope contracts. Native workspace grants are not valid MCP grants.

## File changes

| File | Action | Description |
|---|---|---|
| `crates/huginn/application/src/permission_policy.rs` | Modify | Separate allowed, prompt-eligible, and final-denial outcomes; ensure empty optional escalation configuration does not suppress normal `OnRequest` user decisions; preserve managed restrictions. |
| `crates/huginn/application/src/authorization.rs` | Modify | Implement ordered host checks, explicit persistent grant matching, distinct user decisions, denial/revalidation behavior, and no-repeat matching writes. |
| `apps/huginn/src/config.rs` | Modify | Resolve separate user policy and managed source; define safe user-state path without treating temporary fallback or project state as trusted storage. |
| `apps/huginn/src/main.rs` | Modify | Compose verified policy and grant-store adapters consistently for interactive and headless execution; fail closed on unresolved required authority. |
| `apps/huginn/src/` | Add/modify as justified | Implement user-owned workspace-grant storage and management UI/API. Evaluate `private-fs` 0.2.1 conditionally for this registry only; no adoption before security, compatibility, and real-OS test gates. |
| Managed-policy adapter in `apps/huginn/src/` | Add only after design gate | Protected-source verifier and per-OS locations remain unresolved; do not describe this adapter as implemented until reviewed end-to-end verification exists. |
| `crates/huginn/runtime/src/tools.rs` and native filesystem adapters | Modify | Preserve path, root, symlink, stale-resource, and effect-boundary checks; consume authorization without allowing grants to bypass enforcement. |
| Huginn runtime sandbox modules and target-specific manifests | Modify/add as justified | Implement native Linux, macOS, and Windows enforcement; block parity until real OS integration gates pass. |
| `crates/huginn/runtime/src/mcp.rs` | Modify only as needed | Preserve separate granular MCP modes and scope; do not inherit workspace file grants. |
| `crates/huginn/presentation/src/approval.rs`, TUI menu/event code | Modify | Present exact eligible decisions, complete review, safe denial/cancellation, and preserve mouse and keyboard operation. |
| `apps/huginn/tests/`, `crates/huginn/application/tests/`, `crates/huginn/runtime/tests/`, `crates/huginn/presentation/tests/` | Modify/add | Cover grant storage/scope, policy order, prompt eligibility, stale checks, MCP independence, platform enforcement, and TUI interactions. |
| `.github/workflows/ci.yml` | Modify | Add blocking Huginn runtime/security gates for Linux, macOS, and Windows; platform parity cannot be claimed from build-only jobs. |
| `apps/huginn/config.example.toml`, `docs/huginn/` | Modify | Document profiles, authority precedence, persistent grant scope/revocation, unresolved managed-source status, sandbox behavior, and separate MCP policy. |

Exact modules and dependency choices are implementation work. Do not add a dependency until its threat model, maintenance, licenses, target support, toolchain compatibility, and safe API fit are verified.

## Testing strategy

| Layer | What to test | Approach |
|---|---|---|
| Policy unit | User rules, managed restriction-only precedence, `OnRequest`, empty escalation list, explicit prohibition, prompt eligibility vs final denial | Pure deterministic matrix; prove `Restricted` plus empty optional escalation list reaches a valid user prompt under `OnRequest`, while managed denial does not. |
| Grant-store security | User ownership/protection, canonical root, exact scope, malformed state, atomic updates, edit/revocation, link/reparse and ancestor failures | Inject verifier for deterministic cases plus real-OS tests. Persist only an explicit user choice; project data cannot write grants. |
| Candidate dependency gate | `private-fs` 0.2.1 API fit, ownership/ACL/mode checks, ancestor and link handling, atomic update, target and toolchain compatibility | Focused probes on macOS, Linux, and Windows before adoption. Require security review. Failure means do not adopt; it does not justify weakened checks. |
| Host authorization integration | Exact ordered checks, valid matching grants suppress prompts, non-matches prompt/deny by policy, all three decisions, unavailable approver, invalid policy, persistence failure | Fake store and prompt. Assert grants cannot bypass managed denial, roots, sandbox, stale validation, or approval availability. |
| Native filesystem integration | Read/write/edit/list/search boundaries, canonical roots, symlink/path traversal, stale resource/content, grant match and mismatch | Temporary fixtures and zero-effect assertions; revalidate immediately before mutation. |
| Sandbox per OS | Effective filesystem/process restrictions, backend absence/failure, approval with unchanged sandbox, zero spawn on failure | Real Linux/macOS/Windows runtime probes against the same observable fixtures. Compile-only and fake-backend tests do not establish parity. |
| Managed source | Protected path acquisition, owner/ACL, ancestor verification, verified absence vs invalid source, restriction-only parsing | Remains a blocking unresolved gate. Until a complete reviewed verifier and real-OS tests exist, assert fail-closed outcomes and do not claim implementation. |
| MCP | `auto`, `prompt`, `writes`, `approve`, per-server/tool precedence, missing/adversarial annotations, persistence scope | Independent policy matrix and fakes; assert no native grant inheritance or sandbox equivalence claims. |
| TUI | Mouse/keyboard parity, hit testing, focus, resize, complete preview, eligible choices, deny/cancel, revocation feedback/no-op | Deterministic terminal events and PTY tests without arbitrary sleeps. |
| Architecture/CI | Dependency direction, target gates, blocking PR coverage | Run Huginn architecture and relevant crate suites; Windows jobs must exercise Huginn, not only cross-compile it. |

## Migration / rollout

No project-file migration is allowed. Do not import or infer grants from project configuration, workspace trust, model state, or existing allowlists. Existing session-scoped decisions are not persistent workspace grants and must not be silently promoted. Any new registry begins with no grants; users create each grant through an explicit choice. Provide review, edit, and revocation before enabling persistent grants broadly.

Rollout gates:

1. Resolve and test exact grant identity and resource matching, registry ownership/protection, atomic update, and user-facing revocation. Evaluate `private-fs` 0.2.1 only as a conditional candidate for this per-user store; do not adopt without review, toolchain compatibility, and real-platform tests.
2. Define protected managed locations and a complete safe verifier for macOS, Linux, and Windows. This remains unresolved; keep the final parity gate blocked and do not represent managed verification as implemented.
3. Implement host policy and the ordered authorization sequence, including normal `OnRequest` prompt eligibility for out-of-profile effects when no explicit restriction bars escalation.
4. Implement and independently verify each OS sandbox and native filesystem boundary. Do not use approval as an isolation fallback.
5. Complete TUI mouse/keyboard review and separate MCP contract tests. Add blocking per-OS CI gates.
6. Enable persistent grants only after storage and policy gates pass. Release or declare three-OS parity only after all required real-platform verifier and enforcement gates pass.

If any source, registry, approver, resource, or backend becomes unverifiable, disable the affected authorization path and return an observable denial/error. Do not fall back to project state, a broader grant, or unsandboxed execution. Revocation must prevent subsequent matching operations; it cannot reverse effects already completed.

## Open questions

- Does `private-fs` 0.2.1 satisfy the per-user grant registry's exact security and API requirements on all three OSes, and can it meet the repository's toolchain constraints? It remains a conditionally reviewed candidate pending evidence.
- Which protected per-OS locations and safe, reviewed APIs can verify the complete managed-source path, owner/ACL, and ancestor chain? This is unresolved and blocks final parity.
- What exact resource-scope matching relation is safe for directory-scoped writes while preventing prefix, symlink, and canonicalization bypasses? Define this before storage format implementation.
- Which native sandbox mechanisms and supported profile dimensions meet the same observable contract on macOS, Linux, and Windows? Each requires real platform evidence.
- Which existing settings/API surfaces will expose grant listing, editing, and revocation without introducing project-controlled authorization inputs?

## Risks

- An overly broad or unstable grant key can authorize a different workspace/resource than the user selected. Use canonical identity, narrow scope, effect-boundary revalidation, and adversarial path tests.
- A user-owned registry that is not securely acquired, verified, and atomically updated can be modified or raced. Keep the candidate evaluation and real-OS tests as adoption gates.
- Managed-source verification is unresolved. Treating unverifiable state as absent can bypass administrator restrictions; this blocks final parity and must remain visible.
- Prompt eligibility can regress into a pre-prompt deny for `Restricted` effects if an empty optional escalation list is interpreted as prohibition. Test the exact policy/UI sequence; explicit managed/user denials remain final.
- Persistent grants may be mistaken for project trust or general access. Keep them separate, explicit, narrowly scoped, editable, and revocable.
- A grant or approval could be incorrectly treated as bypassing sandbox, roots, stale-resource checks, or approver availability. Enforce and test each independently.
- Platform sandbox behavior, registry protection, and managed source may differ. Do not claim three-OS parity until all real-OS gates pass.
- Mouse support or incomplete previews could make approval ambiguous or bypass review. Verify hit testing, keyboard paths, resize, review completeness, and safe no-op feedback.
