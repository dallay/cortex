# ADR-0018: Mandatory Seatbelt isolation for macOS shell commands

## Status

Accepted for implementation; operational verification remains limited to the
macOS version recorded in the SDD apply evidence. This status does not indicate
release acceptance.

## Context

`run_shell` executes commands with the host user's authority. Approval decides
whether an operation may run, but approval alone does not constrain filesystem
or network access. Huginn needs a system-enforced boundary for shell commands
without treating approval as permission to remove that boundary.

## Decision

On macOS, every `run_shell` invocation MUST use `/usr/bin/sandbox-exec` and a
dynamic deny-by-default SBPL profile. The profile permits only the workspace
and runtime operations supported by evidence, and denies network and socket
access. The shell and descendants remain subject to the same profile. Failure
to prepare or apply the profile MUST stop execution. There is no opt-out,
fallback, bypass, or persistent grant.

`Deny` continues to block execution. `Allow once` approves only the requested
operation and does not change the sandbox. Native tools and MCP are outside
this boundary. This decision makes no Linux or Windows isolation claim.

## Consequences

- Shell commands that require network access or paths outside the workspace
  can fail.
- Apple deprecates Seatbelt interfaces. `dyld-support.sb` is private Apple
  behavior. Availability and behavior can change; Huginn fails closed.
- Support claims are limited to macOS versions actually tested. The current
  implementation evidence is specific to macOS 27.3, build 26C5035r.
- Runtime allow rules require evidence and must not be broadened speculatively.
- This mechanism does not authorize persistent grants or change approval
  policy.

## Validation boundary

The PR-blocking `Test Huginn Seatbelt (macOS)` CI job checks that it runs on
macOS, requires `/usr/bin/sandbox-exec`, and runs the unfiltered
`cargo test -p huginn-runtime` suite. A CI pass is required for merge. It does
not establish compatibility with other macOS versions or Apple endorsement.

Implementation and scenario evidence: [SDD change](../../.agents/sdd/changes/huginn-autonomous-workspace-permissions/).
