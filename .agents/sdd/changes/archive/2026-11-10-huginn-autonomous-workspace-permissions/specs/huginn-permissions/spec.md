<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:5504633c91fe35636f47df8846e193e5dfc3a1c5f5a08ce1d252d3b48ebbf224 -->

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:255efca5789786ff78ad02486fd9b8af6ee7fafdeccf9ce89176d709f1208792 -->

# Delta Specification: Autonomous workspace permissions

## MODIFIED Requirements

### Requirement: Shell command isolation on macOS

On macOS, Huginn MUST run `run_shell` with the active operating system isolation by default. The policy MUST deny by default, allow only the access needed to the workspace and the resources required to run the shell, and deny network access. The shell process and its descendants MUST remain subject to the same policy. If isolation is not available or cannot be applied, Huginn MUST prevent execution and MUST NOT retry it without isolation.

The policy MUST be generated dynamically in SBPL format and enforced through `/usr/bin/sandbox-exec`. Untrusted path data MUST NOT allow widening the policy.

The mechanism depends on Apple's Seatbelt interfaces, which are deprecated. Huginn MUST communicate this limitation and MUST NOT present their availability or future behavior as guaranteed nor as endorsed by Apple. The behavior must be verified separately on the supported macOS versions. This requirement does not declare isolation support for Linux or Windows.

#### Scenario: Shell allowed inside the workspace

- GIVEN `run_shell` requests an operation within the effective workspace permissions
- WHEN Huginn runs the command on macOS
- THEN the command runs under the active restrictive policy
- AND the processes descending from the shell stay confined by that policy

#### Scenario: Access outside the workspace denied

- GIVEN a command attempts to read or modify a file outside the workspace and outside the runtime read exceptions under `/bin` and `/usr/lib` and the immediate parent's metadata
- WHEN the shell process or one of its descendants requests the access
- THEN the operating system denies the access
- AND the command gains no additional permissions by changing its working directory

#### Scenario: Manipulated paths do not widen permissions

- GIVEN the workspace or a requested path contains metacharacters, sequences with meaning in SBPL, or symbolic links to the outside
- WHEN Huginn generates and applies the policy
- THEN the policy grants no access outside the authorized paths
- AND Huginn denies execution if it cannot determine the scope safely

#### Scenario: Network and sockets denied

- GIVEN a command or descendant attempts to open a network connection or a socket
- WHEN the operation is evaluated under the shell policy
- THEN the access is denied

#### Scenario: Descendant attempts to evade the policy

- GIVEN a command starts a descendant or an alternative execution
- WHEN that process attempts to access files outside the authorized paths (workspace, reads under `/bin` and `/usr/lib` and the immediate parent's metadata) or the network
- THEN the operating system denies the access under the same policy

#### Scenario: Launcher or policy failure

- GIVEN `/usr/bin/sandbox-exec` is not available, or the policy cannot be generated, validated, or applied
- WHEN Huginn receives a `run_shell` request
- THEN it does not start the command
- AND it returns an observable failure result
- AND it does not attempt to run the command without isolation

#### Scenario: Deprecated Seatbelt interface

- GIVEN a macOS version changes, restricts, or removes `sandbox-exec` or a required Seatbelt interface
- WHEN Huginn cannot confirm and apply the policy
- THEN Huginn reports that isolation is not available
- AND it does not run the command without isolation as an automatic alternative

#### Scenario: No parity is declared on other platforms

- GIVEN `run_shell` is requested on Linux or Windows
- WHEN Huginn applies this change
- THEN this requirement does not attribute isolation to that execution
- AND Huginn does not declare that the behavior of that platform is equivalent to macOS

### Requirement: Approval, policy, and isolation are independent controls

Huginn MUST preserve the phase 1 policy and approval semantics. User and global rules, managed restrictions, and approval decisions determine whether an operation is authorized; they neither grant nor widen the operating system isolation. Managed restrictions prevail and can only restrict. `Deny` MUST prevent the operation, and `Allow once` MUST authorize only the presented operation. Huginn MUST NOT introduce persistent grants, nor load, create, or honor them. Huginn MUST NOT introduce YOLO, Exit, nor new full-access modes. The existing `full-access` authorization profile remains available; it does not disable nor widen the mandatory `run_shell` sandbox on macOS.

#### Scenario: Ordinary approval does not remove the isolation

- GIVEN a shell operation requires approval and the user grants it
- WHEN Huginn runs the operation
- THEN the approval is limited to the authorized operation
- AND the command remains subject to the active isolation and the effective policy
- AND the ordinary `HostPolicy` approval is not interpreted as separate consent to run without sandbox
- AND Huginn offers no configuration, exclusion, or bypass to run `run_shell` without sandbox

#### Scenario: Managed restriction prevails

- GIVEN an operation allowed by a user rule is denied by a managed restriction
- WHEN Huginn resolves the operation
- THEN Huginn does not run it
- AND the user approval does not override the managed restriction

#### Scenario: Denial and one-time approval

- GIVEN the user chooses `Deny` or `Allow once` for an operation
- WHEN Huginn processes the decision
- THEN `Deny` prevents that operation and does not become an authorization
- AND `Allow once` does not authorize later operations nor create persistent state

#### Scenario: No persistent grants

- GIVEN a project file, configuration, or previous state contains a persistent grant request or data
- WHEN Huginn resolves permissions for an operation
- THEN it does not create, load, or honor a persistent grant from that data

### Requirement: Shell isolation does not cover native tools nor MCP

Huginn MUST limit the guarantee of this isolation to the processes started by `run_shell` and its descendants. Native file tools MUST remain subject to their own policy and authorization controls. MCP calls MUST keep their specific approval policy. Huginn MUST NOT assert that those operations are confined by the shell sandbox.

#### Scenario: Native tool out of scope

- GIVEN a native tool requests a file operation
- WHEN Huginn authorizes or denies that operation
- THEN it applies the native tool's own controls
- AND it does not use the `run_shell` sandbox as proof of authorization or confinement

#### Scenario: MCP call out of scope

- GIVEN an MCP server receives a call
- WHEN Huginn applies the MCP approval policy
- THEN it keeps the server specific and tool specific rules
- AND it does not attribute the `run_shell` isolation to the call

### Requirement: `run_shell` lifecycle

Applying the isolation MUST preserve the existing `run_shell` lifecycle guarantees: selected environment, working directory, input and output, timeout, cancellation, and process group cleanup. The launcher and the policy MUST NOT allow cancellation or timeout to leave active descendant processes outside Huginn's control.

On every controlled exit that returns as `Result` (success, non-zero exit, rejection, or spawn error, timeout, and cancellation), the runtime MUST attempt the explicit cleanup of the launcher resources and MUST propagate any cleanup error. If the shell was able to run, a cleanup error MUST NOT assert that the shell did not run. If the preparation, creation, or writing of resources fails before starting the launcher, Huginn MUST NOT start the shell; it MUST attempt the explicit cleanup of the resources already created and MUST make observable both the primary error and any cleanup error, without hiding the latter.

If the future or its owner is abruptly dropped and only `Drop` runs, the cleanup MUST be best effort. The contract MUST NOT guarantee the removal of the resource if the operating system fails. Writing resources MUST complete through `write_all`. Closing `std::fs::File` is not an independently required scenario: its error is not observable through the safe API of that type. This limitation does not reduce the launcher security requirements.

**Rationale:** `std::fs::File` does not expose a close error through its safe API, so requiring or testing that error independently would not be verifiable. The runtime can require `write_all`, explicit cleanup on controlled exits, and propagation of observable errors. The close observability limitation does not authorize omitting cleanup, relaxing the launcher, or running without sandbox.

#### Scenario: Execution preserves the selected parameters

- GIVEN a valid request specifies environment, working directory, and input
- WHEN Huginn runs `run_shell` under isolation
- THEN the process receives those parameters as requested
- AND Huginn delivers the output according to the existing `run_shell` contract

#### Scenario: Timeout or cancellation cleans up descendants

- GIVEN a command creates one or more descendant processes
- WHEN the execution reaches the timeout or is canceled
- THEN Huginn stops the shell process and cleans its process group according to the existing guarantees
- AND no descendant keeps running as a result of an incomplete cleanup

#### Scenario: Explicit cleanup on controlled exits

- GIVEN the launcher has been prepared and `run_shell` ends with success, non-zero exit, rejection, spawn error, timeout, or cancellation returned as `Result`
- WHEN the runtime returns the result of `run_shell`
- THEN the runtime attempts the explicit cleanup of the launcher resources
- AND it propagates any cleanup error
- AND if the shell was able to run, the returned error does not assert that the shell did not run

#### Scenario: Preparation failure before the launcher

- GIVEN the preparation, creation, or writing of a launcher resource fails before starting it
- WHEN Huginn processes the failure
- THEN it does not start the launcher nor the shell
- AND it attempts the explicit cleanup of the resources already created
- AND the result makes observable the primary error and any cleanup error, without hiding the cleanup error

#### Scenario: Abrupt drop uses best-effort cleanup

- GIVEN the future or its owner is abruptly dropped before `run_shell` returns a `Result`
- WHEN only the `Drop` cleanup can run
- THEN Huginn attempts to remove the resource on a best-effort basis
- AND the contract does not guarantee the removal if the operating system fails

#### Scenario: Complete write without an independent close error requirement

- GIVEN Huginn writes the private launcher resource
- WHEN the write completes
- THEN Huginn uses an operation that writes all bytes or reports a write error
- AND compliance does not depend on inducing a close error of `std::fs::File` through its safe API

<!-- /coderabbit-ai-generated:v1 --><!-- /coderabbit-ai-generated:v1 -->
