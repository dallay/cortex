# Delta Specification: Autonomous Workspace Permissions

## Scope and Contract Replacement

This specification explicitly replaces incompatible requirements from the previous contract in this file, especially: closed autonomy limited to ordinary native operations, mandatory approval for every shell command and MCP call, shell described as unsandboxed, and session grants that exclude shell and MCP. The two contracts MUST NOT be implemented in parallel. The independent requirements for mouse and keyboard interaction in all TUI menus and for approval UX remain in force as specified here.

Platform parity MUST NOT be declared until Huginn's capability is implemented and verified on each supported operating system.

## ADDED Requirements

### Requirement: User-Owned Project Trust Registry

Huginn MUST load project trust only from a user-owned trust registry. Each trust entry MUST be keyed by the canonical project root, and the registry MUST establish trust provenance independently of project-controlled files or metadata. Trust is descriptive state and MUST NOT equal or imply permission to perform any effect. A workspace-scoped write grant is a specific user authorization, not trust or blanket trust. Huginn MUST validate registry ownership and protection against unauthorized modification on macOS, Linux, and Windows before using an entry. Project files, workspace location, and project-controlled configuration MUST NOT create, elevate, or persist trust. A project without a valid registry entry MUST be treated as untrusted. If registry contents, ownership, protection, or canonical project identity cannot be verified, Huginn MUST fail closed and MUST NOT grant trust.

#### Scenario: Trust entry matches canonical project root

- GIVEN the user-owned registry contains a valid trust entry for a project's canonical root
- WHEN Huginn resolves that project's trust state
- THEN it obtains trust only from that registry entry
- AND it records the registry as the trust provenance

#### Scenario: Project files attempt to grant trust

- GIVEN a project has no trust entry in the user-owned registry
- AND project-controlled files or configuration declare that the project is trusted
- WHEN Huginn resolves the project's trust state
- THEN Huginn treats the project as untrusted
- AND project-controlled data does not create or persist a trust entry

#### Scenario: No trust registry entry exists

- GIVEN the user-owned registry has no entry for the project's canonical root
- WHEN Huginn resolves the project's trust state
- THEN Huginn treats the project as untrusted
- AND it applies the explicit untrusted-project policy

#### Scenario: Registry is invalid or unreadable

- GIVEN the trust registry is malformed, unreadable, or cannot be verified as user-owned and protected
- WHEN Huginn resolves the project's trust state
- THEN Huginn does not grant trust
- AND it fails closed with an observable trust-state failure

#### Scenario: Registry ownership and protection on macOS

- GIVEN Huginn reads a project-trust registry on macOS
- WHEN it validates the registry before using an entry
- THEN it accepts the registry only if its ownership and protection establish that it is user-owned and not modifiable by project-controlled data or unauthorized principals
- AND it rejects an unverifiable registry without granting trust

#### Scenario: Registry ownership and protection on Linux

- GIVEN Huginn reads a project-trust registry on Linux
- WHEN it validates the registry before using an entry
- THEN it accepts the registry only if its ownership and protection establish that it is user-owned and not modifiable by project-controlled data or unauthorized principals
- AND it rejects an unverifiable registry without granting trust

#### Scenario: Registry ownership and protection on Windows

- GIVEN Huginn reads a project-trust registry on Windows
- WHEN it validates the registry before using an entry
- THEN it accepts the registry only if its ownership and protection establish that it is user-owned and not modifiable by project-controlled data or unauthorized principals
- AND it rejects an unverifiable registry without granting trust

### Requirement: Protected Managed Restrictions

Huginn MUST load managed restrictions only from protected system configuration. Before applying managed configuration, Huginn MUST verify on macOS, Linux, and Windows that its ownership and protection establish system or administrator control and prevent modification by project-controlled data or an unprivileged user. Managed configuration MUST be restriction-only: it MAY reduce permissions granted by user policy, but MUST NOT grant trust, weaken a restriction, or otherwise increase effective authority. A valid administrator restriction MUST override a weaker user policy. If managed configuration is present but malformed, unreadable, or unverifiable, Huginn MUST fail closed and MUST NOT execute permission-governed effects under a weaker policy.

#### Scenario: Managed configuration ownership and protection on macOS

- GIVEN Huginn reads managed restrictions on macOS
- WHEN it validates the configuration before policy resolution
- THEN it accepts the configuration only if its ownership and protection establish system or administrator control and prevent unauthorized modification
- AND it rejects unverifiable configuration and fails closed

#### Scenario: Managed configuration ownership and protection on Linux

- GIVEN Huginn reads managed restrictions on Linux
- WHEN it validates the configuration before policy resolution
- THEN it accepts the configuration only if its ownership and protection establish system or administrator control and prevent unauthorized modification
- AND it rejects unverifiable configuration and fails closed

#### Scenario: Managed configuration ownership and protection on Windows

- GIVEN Huginn reads managed restrictions on Windows
- WHEN it validates the configuration before policy resolution
- THEN it accepts the configuration only if its ownership and protection establish system or administrator control and prevent unauthorized modification
- AND it rejects unverifiable configuration and fails closed

#### Scenario: Administrator restriction overrides weaker user policy

- GIVEN a valid managed restriction is more restrictive than the user's policy
- WHEN Huginn resolves effective permissions
- THEN the managed restriction takes precedence
- AND the effective permissions do not exceed the managed restriction

#### Scenario: Managed configuration cannot grant authority

- GIVEN managed configuration proposes a permission or trust grant that would increase authority beyond user policy
- WHEN Huginn resolves effective policy
- THEN Huginn does not apply that grant
- AND managed configuration can only preserve or reduce effective permissions

#### Scenario: Malformed, unreadable, or unverifiable managed configuration

- GIVEN managed configuration is present but malformed, unreadable, or cannot be verified as protected system configuration
- WHEN Huginn resolves policy or reaches an effect boundary
- THEN it fails closed and does not execute the permission-governed effect
- AND it returns an observable configuration failure

## MODIFIED Requirements

### Requirement: Effective Policy, Profiles, and Project Trust

Huginn MUST resolve authorization and sandboxing as independent controls from configurable global/user rules, user-authorized workspace grants, the selected profile, and managed restrictions. Global/user rules and explicit workspace grants are distinct authorities: a rule defines policy, while a grant authorizes only its explicitly bounded effect scope. Managed restrictions MUST override conflicting user rules and grants and MUST only restrict. The approval policy MUST support `OnRequest` for bounded approval of out-of-policy effects when policy permits; it MUST NOT require approval for every shell command as a universal rule. Defaults MUST be explicit and context-specific. An empty or unspecified escalation default MUST NOT silently reject an otherwise prompt-eligible operation: Huginn MUST offer bounded approval when policy permits. Managed denial, invalid policy, or unavailable policy remains a final non-execution outcome.

“Trusted” and “untrusted” MUST describe an explicit project trust state, not a permission grant. Huginn MUST explicitly define verifiable trust boundaries, their provenance, and the mapping of those states to profiles and policies. Huginn MUST NOT automatically assume a project is trusted because of its path, workspace, opening, content, or a model declaration. Huginn MUST NOT equate `trusted` with universal approval, unrestricted access, or the absence of sandboxing. If Huginn cannot verify the state or resolve the mapping, it MUST apply the configured restrictive profile or fail closed.

#### Scenario: On-demand approval within policy

- GIVEN the effective configuration selects `OnRequest` and the command fits within the effective sandbox and profile
- WHEN execution of the command is requested
- THEN the system does not request approval solely because the operation uses shell
- AND it executes the command with the sandbox required by the profile

#### Scenario: Out-of-policy effect is offered for bounded approval

- GIVEN an operation is outside the effective user policy
- WHEN the approval policy is `OnRequest`
- THEN the system offers a bounded approval request if policy permits and a valid approver is available
- AND it does not execute the operation unless the user gives a valid authorization
- AND denial, cancellation, or unavailable approval prevents execution

#### Scenario: Empty escalation default does not silently reject

- GIVEN an effect is outside user policy and no escalation choice is configured
- AND no managed restriction denies the effect
- WHEN Huginn resolves the effect
- THEN it offers bounded approval if policy permits and an approver is available
- AND it does not silently reject the effect solely because the escalation default is empty

#### Scenario: Managed denial or invalid policy is final

- GIVEN a managed restriction denies an effect, or the applicable policy is invalid or unavailable
- WHEN Huginn resolves the effect
- THEN Huginn does not offer a user grant that could override the restriction or policy failure
- AND it does not execute the effect

#### Scenario: Trust is not inferred from the path

- GIVEN a project is within the workspace but has no explicit, verifiable trust state
- WHEN Huginn resolves its profile and permissions
- THEN Huginn does not automatically classify it as trusted
- AND it applies the explicit trust mapping or fails closed

#### Scenario: Managed restriction takes precedence

- GIVEN an administrator or effective configuration imposes a profile or policy more restrictive than the user default
- WHEN Huginn resolves the policy
- THEN it uses the effective restriction
- AND it does not replace that decision with a universal default

### Requirement: Bounded Effect Authorization Decisions

For an effect that policy permits the user to authorize, Huginn MUST present the decisions `Allow once`, `Allow for this workspace`, and `Deny`. `Allow once` MUST authorize only the presented operation and MUST NOT persist. `Allow for this workspace` MUST be an explicit user authorization stored only in user-owned configuration or a user-owned registry, keyed by the canonical workspace root and a narrow capability/action/resource scope. Huginn MUST validate ownership, protection, and canonical workspace identity before honoring a stored grant on macOS, Linux, and Windows. Project-controlled files or configuration MUST NOT create, modify, or persist this grant. Subsequent matching operations within that workspace scope MUST NOT prompt again. An operation outside any part of the grant scope MUST be resolved by current policy and, when policy permits, offered as a separate bounded approval. `Deny` MUST block the presented operation and MUST NOT be converted to implicit allow. Managed restrictions MUST override every user grant and MUST only restrict. Grants MUST NOT remove or broaden the baseline OS sandbox. Behavioral instructions, including instructions not to enumerate environment variables, are not host permission grants and are outside this change. No permissive YOLO, Exit, or full-access mode is in scope.

#### Scenario: Allow once is operation-scoped

- GIVEN an effect is eligible for user approval
- WHEN the user selects `Allow once`
- THEN Huginn authorizes only the presented operation
- AND a later matching operation requires a new decision unless separately authorized by policy
- AND no persistent workspace grant is created

#### Scenario: Persistent workspace grant suppresses matching write prompts

- GIVEN the user explicitly selects `Allow for this workspace` for a file-write operation
- AND the grant is valid for the canonical workspace root and the narrow capability/action/resource scope
- WHEN Huginn later requests a matching file write in that workspace
- THEN Huginn authorizes the write without prompting again, subject to current policy and sandbox enforcement
- AND the grant remains in user-owned state and is not stored in project-controlled data

#### Scenario: Workspace grant does not cross privilege scope

- GIVEN a user grant authorizes file writes for one canonical workspace and a defined resource scope
- WHEN an operation targets another workspace, another resource, another action, or another capability
- THEN Huginn does not apply that grant to the operation
- AND it applies current policy and offers bounded approval only when policy permits

#### Scenario: Project-controlled data cannot create a workspace grant

- GIVEN project-controlled files request or contain a persistent permission grant
- WHEN Huginn resolves authorization for the project
- THEN Huginn ignores that data as a grant authority
- AND only a valid user-authorized grant in user-owned state can authorize matching workspace effects

#### Scenario: Workspace grant registry is unverifiable

- GIVEN a stored workspace grant is malformed, unreadable, or its ownership, protection, or canonical root cannot be verified on the current operating system
- WHEN Huginn resolves an operation that would match the grant
- THEN Huginn does not honor the grant
- AND it fails closed with an observable authorization outcome

#### Scenario: Denial remains denial

- GIVEN the user selects `Deny` for a presented effect
- WHEN Huginn processes the decision
- THEN Huginn does not execute that operation
- AND it does not treat the denial as an allow-once or persistent grant

#### Scenario: Managed restriction overrides a workspace grant

- GIVEN a valid user-owned workspace grant matches an operation
- AND a managed restriction denies that operation
- WHEN Huginn resolves the operation
- THEN the managed restriction prevents execution
- AND the user grant does not weaken or override the restriction

#### Scenario: Behavioral instruction is not host authorization

- GIVEN an agent behavioral instruction says not to enumerate environment variables
- WHEN Huginn resolves host permissions
- THEN the instruction does not create, replace, or revoke a host permission grant

### Requirement: Shell and Native File Operations Under Effective Isolation and Policy

Huginn MUST execute shell within the process or OS sandbox required by the selected profile. Authorization of an operation and process isolation MUST remain independent: approving an ordinary escalation MUST NOT disable or broaden the sandbox. `OnRequest` MUST allow escalation of the operation that exceeds policy, not removal of general isolation.

Native file tools MUST apply the effective filesystem policy of the profile, including applicable read and write capabilities, roots, and exceptions. They MUST NOT be treated as shell or reduced to a single `write_file` operation; authorization of a native operation and shell sandboxing are distinct controls. A file operation not authorized by its capabilities or roots MUST be denied or escalated according to the effective policy. A path that cannot be verified MUST fail closed.

The sandbox MUST be enforced by an OS-specific backend with verifiable capabilities and limits. Huginn MUST NOT claim equivalence across Linux, macOS, and Windows merely because they share a profile or interface name. Huginn MUST implement and verify the required capability on each of Linux, macOS, and Windows before declaring parity across those platforms. If the required backend is missing, unavailable, unable to enforce isolation, or the policy resolves to no sandbox, Huginn MUST reject execution and MUST NOT run shell without isolation as a fallback.

#### Scenario: Shell permitted by sandbox

- GIVEN a command whose operation is permitted by the effective profile
- WHEN Huginn executes it
- THEN the command runs in the OS sandbox corresponding to that profile
- AND a working location within the workspace does not, by itself, broaden permissions

#### Scenario: Approval does not disable sandboxing

- GIVEN a shell operation exceeds an ordinary capability and the user approves it
- WHEN Huginn continues execution
- THEN it retains the sandbox required by the profile
- AND approval does not grant unrestricted access or remove limits outside the escalation

#### Scenario: Sandbox backend is missing or cannot enforce policy

- GIVEN the operating system has no supported backend or the backend cannot enforce the required policy
- WHEN shell execution is requested
- THEN Huginn returns an observable error or result indicating that sandboxing is unavailable
- AND it does not start the process or fall back to execution without a sandbox

#### Scenario: Native operation is outside permitted roots

- GIVEN a native file tool requests a resource outside the effective roots or capabilities
- WHEN Huginn evaluates the operation
- THEN the operation is denied or escalated according to the effective policy
- AND the operation is not considered authorized merely because it is available in the tool catalog

#### Scenario: Resource verification fails

- GIVEN a path, symlink, root, or exception cannot be verified safely
- WHEN Huginn evaluates or revalidates a file operation
- THEN Huginn does not perform the effect
- AND it returns an observable failure without broadening access

### Requirement: Approval and Sandboxing Are Independent Decisions

Approval MUST authorize only the escalation or action, and scope, presented and permitted by the effective policy. Approval MUST NOT disable sandboxing or convert the profile, session, or project to unrestricted access. The absence, cancellation, denial, timeout, or error of an approver MUST NOT become implicit approval. This rule applies to shell and native tools; MCP calls remain subject to the MCP-specific policy requirement.

#### Scenario: Approval is limited to the presented request

- GIVEN a request presents a specific action and resource
- WHEN the user approves that request
- THEN only the approved scope is authorized, subject to the active profile and sandbox
- AND a different action or resource requires a separate decision under its policy

#### Scenario: Approver is unavailable

- GIVEN an operation requires approval and no valid approver is available
- WHEN Huginn attempts to process it
- THEN the operation is not executed
- AND the result identifies that approval is unavailable

### Requirement: Granular MCP Policy by Server and Tool

Huginn MUST provide the observable semantics of MCP modes `auto`, `prompt`, `writes`, and `approve`, with effective resolution configurable per server and tool and with explicit override precedence. The decision MUST be based on available tool annotations, without treating those annotations as proof of security or isolation:

- `auto`: MUST request approval unless annotations establish read-only behavior and indicate no destructive or open-world effects; if annotations are missing or incomplete, MUST request approval.
- `prompt`: MUST request approval for every call.
- `writes`: MUST request approval unless `read_only_hint=true`.
- `approve`: MUST NOT request approval for an MCP call under this configured mode.

Remembered or persistent approvals, if offered, MUST respect and not exceed the effective server and tool policy and scope. No permissive full-access mode is introduced by this change.

MCP modes, annotations, MCP approvals, and the shell sandbox MUST remain conceptually separate. Huginn MUST NOT describe MCP as isolated by the shell sandbox, claim an MCP process inherits it, or present annotations as sandboxing.

#### Scenario: Auto mode with exclusively read-only annotations

- GIVEN an MCP call in `auto` mode with verifiable annotations indicating read-only behavior and no destructive or open-world effects
- WHEN Huginn evaluates the call
- THEN it does not request approval for that call

#### Scenario: Auto mode with unknown or risky metadata

- GIVEN an MCP call in `auto` mode with missing or incomplete annotations, or with destructive or open-world effects
- WHEN Huginn evaluates the call
- THEN it requests approval before execution
- AND it does not interpret missing annotations as permission to read

#### Scenario: Prompt mode and writes mode

- GIVEN an MCP call in `prompt` mode, or in `writes` mode without `read_only_hint=true`
- WHEN the call is requested
- THEN Huginn requests approval
- AND it does not execute the call until it receives a valid decision

#### Scenario: Read-only call in writes mode and call in approve mode

- GIVEN a call with `read_only_hint=true` in `writes` mode, or a call configured in `approve` mode
- WHEN Huginn evaluates the call
- THEN it does not request per-call MCP approval under the semantics of that mode
- AND it does not present that decision as isolation of the MCP server

#### Scenario: Tool override within a server

- GIVEN an effective policy or override exists for a specific tool
- WHEN Huginn resolves the call together with the server policy
- THEN it applies the configured precedence deterministically
- AND the resulting decision does not exceed effective authority

#### Scenario: MCP is not isolated by the shell sandbox

- GIVEN an MCP server runs alongside sandboxed shell
- WHEN Huginn communicates or evaluates its security boundaries
- THEN it does not attribute shell isolation to the server without specific evidence
- AND it keeps MCP authorization separate from sandbox policy

### Requirement: Mouse and Keyboard Interaction in All TUI Menus

All selectable interactive TUI menus MUST support selection by mouse and keyboard, including settings, provider and model selection, workflows, plugin dialogs, help, approval, and any selector or completion presented as a menu. Input MUST use hit testing consistent with visible content, preserve focus and keyboard navigation, and define the result of clicks on rows, buttons, scrollable areas, empty space, borders, and outside the menu. Resize MUST recalculate interaction geometry. A click MUST NOT bypass mandatory review, accidentally confirm a protected action, or approve a stale request. This requirement remains in force independently of the policy and sandbox work.

#### Scenario: Equivalent selection

- GIVEN a visible interactive menu with a selectable option
- WHEN the user activates the option with a primary click or the available keyboard path
- THEN both inputs select the same option except for controls that require protected confirmation
- AND keyboard navigation, focus, and cancellation remain available

#### Scenario: Click outside a target

- GIVEN the user clicks outside a valid row, button, or target
- WHEN the menu processes the event
- THEN it does not activate an option or confirm or approve an action
- AND it keeps selection and focus in a valid state

#### Scenario: Resize and hit testing

- GIVEN an open menu is resized
- WHEN it processes the next input
- THEN click targets match the current visible geometry
- AND an empty position does not select the target that previously occupied that position

#### Scenario: Protected action requires review

- GIVEN a request requires review of a diff, command, or other detail before approval
- WHEN the user clicks before completing the review or on a stale request
- THEN Huginn does not approve the request or execute the effect
- AND it preserves the option to review or cancel

### Requirement: Approval UX, Review, and No-Op Feedback

The approval dialog MUST present the action, resource, risk, and complete preview required for the effect class with a readable hierarchy. It MUST expose only decisions permitted by the host. Review state MUST distinguish an incomplete or truncated preview from one reviewed in full, especially for shell; a narrow terminal or resize MUST NOT hide content while falsely indicating that it was reviewed. If presentation or the approver is absent, revoked, or unable to produce a valid decision, the flow MUST report that the effect will not execute and offer a safe exit. Denial, cancellation, or a decision with no effect MUST provide visible feedback and a coherent view and review state, without indicating execution success. This requirement remains an independent deliverable from sandboxing.

#### Scenario: Complete, reviewable request

- GIVEN a valid approval request
- WHEN the dialog is presented
- THEN it shows the required complete action, resource, risk, and preview
- AND it shows only the decisions permitted by the host
- AND it enables approval only after the required review is complete

#### Scenario: Preview in a narrow or resized terminal

- GIVEN the preview does not fit completely in the available space
- WHEN the dialog renders or the terminal is resized
- THEN the user can navigate and review the complete preview
- AND the dialog identifies pending content without prematurely indicating that review is complete

#### Scenario: Approver is absent or revoked

- GIVEN no approval channel or approver is available, or the approver is revoked before deciding
- WHEN an effect requires approval
- THEN the flow reports that the effect will not execute because approval is unavailable
- AND it offers denial, cancellation, or a safe return without executing the effect

#### Scenario: Denial, cancellation, or no-op

- GIVEN the user denies, cancels, or selects a decision that does not execute the effect
- WHEN the dialog closes
- THEN the result identifies denial, cancellation, or no-op
- AND the view and review state reflect that the effect did not execute
- AND the flow does not report execution success

### Requirement: Denial and Fail-Closed Behavior at Permission Boundaries

Timeout, missing UI or approver, configuration error, unknown profile, unverifiable trust state, insufficient classification, authorization or revalidation error, sandbox failure, and missing backend MUST NOT become implicit approval. Before any execution, Huginn MUST confirm policy, scope, validity, and the required sandbox. If it cannot confirm any of these, it MUST prevent the effect and return an observable state. Cancellation and denial MUST remain available by keyboard, including Escape when compatible with the dialog.

#### Scenario: Denial, timeout, or loss of UI

- GIVEN a pending approval request
- WHEN the user denies or cancels it, its deadline expires, or the channel is lost before a valid decision
- THEN the effect is not executed
- AND the result preserves the reason without interpreting it as approval

#### Scenario: Invalid configuration or classification

- GIVEN Huginn cannot resolve the required profile, policy, or classification
- WHEN execution reaches its boundary
- THEN it fails closed without execution
- AND it returns an observable invalid-configuration or invalid-classification state

#### Scenario: Revalidation detects a change

- GIVEN a path, symlink, content, workspace, or scope changes after the request is prepared
- WHEN Huginn revalidates before execution
- THEN it stops the effect and requests new review or approval when applicable, or denies the operation
- AND it does not execute the stale proposal

### Requirement: Platform-Specific and Observable-Outcome Verification

Tests MUST separately verify profile and trust-state resolution, approval decisions, workspace grant persistence and scope, filesystem policies, escalation, sandbox enforcement by OS backend, fail-closed behavior, and MCP modes. Support or equivalence MUST NOT be declared for a platform without specific evidence that its backend enforces the required capabilities. This verification MUST address Linux, macOS, and Windows individually before parity across them is declared. TUI tests MUST verify mouse and keyboard input, hit testing, focus, resize, complete review, approval states, and no-op behavior. Tests MUST be deterministic and MUST NOT depend on `sleep` or arbitrary timing.

#### Scenario: Supported backend and unavailable backend

- GIVEN platforms with an available backend and a platform or backend that is missing or incapable
- WHEN shell execution tests run
- THEN the available backend demonstrates enforcement of the declared policy
- AND the missing or incapable case demonstrates denial without starting an unsandboxed process

#### Scenario: MCP modes and adversarial metadata

- GIVEN all four modes, overrides, and complete and missing annotations
- WHEN MCP call decisions are tested
- THEN each case produces the outcome defined by the MCP requirement
- AND no test claims that the server is sandboxed by the shell sandbox

#### Scenario: TUI and approval tests

- GIVEN interactive menus and approval dialogs
- WHEN tests exercise mouse and keyboard input, outside clicks, cancellation, resize, and narrow terminals
- THEN they verify hit testing, focus, review, no-op feedback, and preservation of denial and cancellation
- AND they do not depend on nondeterministic delays
