<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:70168a6d5ba42de730739ce1c9732fcd2fdc2fe39b4e63d8f2e1fcd4c870bda2 -->

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:ed99815a5a04bdfc399ca33e91dc7bbf3a7b98f24938392e93bdb6bbb96d9bca -->

# Proposal: Autonomous workspace permissions for Huginn

## Intent

Implement on macOS an operating system boundary for Huginn shell commands, following the Codex technical pattern: a dynamically generated Seatbelt SBPL profile, a default-deny policy and execution of the real shell inside `/usr/bin/sandbox-exec`, with confinement inherited by its descendant processes. The reference adopts the sandbox mechanism; it does not adopt the whole Codex escalation policy.

Apple deprecates `sandbox-exec` and the related Seatbelt interfaces. The implementation must communicate this limitation, fail closed and not claim compatibility with other operating systems.

## Scope

### Included

- Implement a sandbox backend for macOS only, based on dynamic SBPL and the absolute executable `/usr/bin/sandbox-exec`, applied to `/bin/sh -c` and its descendants.
- Keep the sandbox active by default on macOS, with `(deny default)`, demonstrated minimum permissions, writes limited to the workspace, workspace reads and runtime exceptions under `/bin` and `/usr/lib`, metadata of the immediate parent of the workspace and network access denied.
- Separate the technical confinement from the approval policy. An ordinary approval authorizes an operation according to the policy; it does not remove or widen the operating system boundary.
- If preparation, validation, profile installation or the launcher fails, do not run the command. Report an error to the user; do not retry outside the sandbox.
- Keep persistent grants disabled and do not create, load or honor a persistent grants registry. The session grants of `HostPolicy` remain only in memory, bound to the exact session, action and canonical path for `native.write_file` and `native.edit_file` with native provenance and non-sensitive resource; they do not widen the shell sandbox.
- Preserve the current decisions on policy resolution and operation scope: managed restrictions prevail and only restrict; project trust does not widen permissions; `Deny` and `Allow once` keep their scope; no YOLO, Exit or total access modes are added.
- Preserve the current execution lifecycle, including selected environment, working directory, input/output, cancellation and timeout. For controlled exits, perform explicit cleanup and propagate its error; on abrupt termination or cancellation by `drop`, use `Drop` as a best-effort attempt, without promising that resources are removed.
- Document the Apple deprecation, the sandbox limits and the possibility that some existing commands fail.

### Excluded

- Any shell execution without a sandbox, opt-out mode, `sandbox = false` configuration or launcher bypass. On macOS, `run_shell` must always run under the sandbox; if that is not possible, it must fail without executing.
- Any permission escalation or elevation outside the sandbox. An ordinary `HostPolicy` approval does not constitute consent to remove the confinement. Any future elevation requires a separate change with an explicit consent contract, operation identity, risk explanation and mandatory rationale.
- Copying the full Codex escalation policy. This change copies only the technical Seatbelt mechanism and does not import its consent flows, exceptions or broad permissions.
- Linux and Windows, their isolation backends and any claim of parity between platforms.
- Network access, sockets or proxy exceptions inside the sandbox.
- Persistent grants, grants storage or a grants management interface. The managed-restrictions verifier remains an independent requirement for any future grants proposal.
- Broad Codex permissions without demonstrated need; signed App Sandbox, helpers, bookmarks, packaging or signing. The earlier App Sandbox POC is not isolation evidence.
- Claiming that the sandbox protects native operations, MCP calls or other effects outside the shell execution covered by this change.

## Approach

Wrap the existing shell in the fixed launcher `/usr/bin/sandbox-exec` and generate a safe SBPL policy from normalized and validated paths. The generation must treat each path as untrusted input, escape SBPL syntax correctly and prevent paths with metacharacters, symbolic links or race conditions from widening permissions. User paths are not interpolated into shell text. Explicit cleanup of controlled exits must propagate observable errors; a `Drop` during abrupt termination or cancellation is only a best-effort attempt and does not guarantee removal. The close error of `std::fs::File` is not observable through safe APIs and does not constitute an independent verification criterion.

The profile starts from `(deny default)`. It only includes the minimum permissions demonstrated to start `/bin/sh`, run and keep its descendants confined, read the required system components and access the workspace. Network stays denied. No general permissions are added for temporary directories, preferences, applications, proxies or optional services without evidence.

The sandbox is active by default on macOS and there is no opt-out nor sandbox-free fallback in this change. If configuration, generation, validation or profile application fails, or the launcher is unavailable, Huginn will not run the command and will show an error. Ordinary shell approval does not authorize execution without confinement. Any elevation outside the sandbox requires a separate change that defines a specific consent contract. On systems other than macOS, this change neither declares nor implements shell isolation.

Validation includes adversarial tests on macOS: allowed workspace access; denial of credentials, other workspaces, external paths and network; descendant confinement; safe handling of special paths; error without execution when the profile or launcher fails; and cancellation/timeout with explicit cleanup and error propagation when the exit is controlled. For abrupt termination or cancellation by `drop`, only the best-effort attempt is verified; no removal guarantee is claimed for operating system failures. The matrix of major macOS versions reports observed results, without implying Apple endorsement.

## Affected areas

| Area                                                                    | Impact              | Description                                                                                                 |
|-------------------------------------------------------------------------|----------------------|-------------------------------------------------------------------------------------------------------------|
| `crates/huginn/runtime/src/tools.rs`                                    | Modified            | Wrap the real shell execution, preserve the lifecycle and fail closed.                                      |
| `crates/huginn/runtime/`                                                | Modified            | Safe policy generation and unit, integration and adversarial tests on macOS.                                |
| `docs/huginn/README.md`, `docs/huginn/architecture.md` and relevant ADR  | Modified            | Explain limits, deprecation, default, compatibility impact and platform scope.                             |
| `.github/workflows/ci.yml`                                              | Possible modification | Bring macOS tests into a suitable blocking path; define it in design and tasks.                             |
| `.agents/sdd/changes/huginn-autonomous-workspace-permissions/`          | Updated             | Align the proposal with fail-closed and leave any elevation outside the sandbox for a separate change.      |

## Compatibility impact

The active default on macOS is an intentional behavior change. Commands that read or write outside the workspace, use network or sockets, depend on disallowed services or require paths that are not included can fail and show an error. No exclusion is offered to recover compatibility through sandbox-free execution. The availability and behavior of `sandbox-exec` can vary or disappear in future macOS versions. Linux and Windows keep their current behavior and are out of scope; this does not claim that those systems are isolated.

## Risks

| Risk                                                                                                                                                          | Probability | Mitigation                                                                                                                                                                           |
|-----------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Apple changes or removes a deprecated interface.                                                                                                                 | High         | Test target macOS versions, report errors and do not enable an unconfined fallback.                                                                                              |
| The sandbox blocks valid commands because of the minimum profile.                                                                                               | High         | Document the impact, test representative tools and return actionable errors; do not allow an opt-out in this change.                                                       |
| An error in SBPL generation or escaping widens permissions.                                                                                                    | Medium       | Small and testable generator, validated inputs, adversarial tests and default deny.                                                                                |
| Path races, symbolic links or inherited descriptors elude the expected confinement.                                                                               | Medium       | Safe normalization, review and tests of paths, links, descriptors and descendants; do not claim unverified guarantees.                                                       |
| The wrapper alters timeout, cancellation or signals to the process group; cleanup may not complete on abrupt termination or operating system failures. | Medium       | Test controlled exits and observable error propagation; treat `Drop` as best effort, without promising removal on abrupt cancellation or operating system failure. |
| The active default affects current macOS users.                                                                                                                | High         | Treat it as an explicit compatibility change and provide clear errors; do not run outside the sandbox.                                                                       |

## Rollback plan

If the macOS matrix, the adversarial tests or the reliability of the deprecated interface do not meet the criteria, stop the launch and keep commands unexecuted under this change (fail-closed), without automatic fallback. A later rollback can remove the backend in an explicit change; it must not silently turn the default into sandbox-free execution. There are no persistent grants to migrate or roll back.

## Dependencies

- macOS with `/usr/bin/sandbox-exec` available for development and testing; its absence produces an error and no execution.
- Access to macOS runners or environments to test the target versions.
- Any consent to elevate permissions outside the sandbox is out of scope for this change and requires a separate change.

## Success criteria

- [ ] On macOS, `run_shell` uses by default the dynamically generated minimum SBPL profile and `/usr/bin/sandbox-exec`.
- [ ] The shell and its descendants stay inside the tested policy; if the profile or launcher fails, the user receives an error and the command does not run.
- [ ] There is no opt-out, no `sandbox = false` configuration and no fallback to sandbox-free execution.
- [ ] Ordinary approval does not remove or widen the sandbox; any future elevation requires another change and an explicit consent contract.
- [ ] The profile denies network and limits writes to the workspace; it does not import broad Codex permissions without evidence.
- [ ] Adversarial tests cover paths, file limits, network, descendants, launcher failures and cancellation/timeout; on controlled exits, explicit cleanup propagates observable errors. On abrupt termination or cancellation by `drop`, cleanup is best-effort and does not guarantee removal. The close error of `std::fs::File` is not verified separately because it is not observable through safe APIs.
- [ ] The matrix of major macOS versions reports reproducible results and observed limits.
- [ ] Persistent grants stay disabled; Linux and Windows remain out of scope, without parity claims.
- [ ] The documentation explains the Apple deprecation, the default change and the possibility that some commands fail.

<!-- /coderabbit-ai-generated:v1 --><!-- /coderabbit-ai-generated:v1 -->
