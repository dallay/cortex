# Exploration: Effective parity for autonomous workspace permissions

## Executive summary

The inspected reference behavior does not mean that every operation inside a workspace is approved automatically or that every command requires approval. Approval policy and sandbox policy are separate: under an on-request policy, a shell command runs under the selected sandbox; a request for capabilities outside that policy can be escalated for approval, and approval does not remove the sandbox for an ordinary operation. Native file operations use host capabilities and permissions; they are not simply shell commands under another name. MCP has a separate policy, with per-server and per-tool controls.

Huginn should not claim parity until it defines the scope of each boundary. The inspected implementation provides sandboxing on macOS, Linux, and Windows through different mechanisms, and rejects filesystem execution when the executor cannot enforce the requested sandbox. This does not show that Huginn already has an equivalent implementation.

## Observed reference behavior

### Profiles, trust, and approval

- The configuration API defines `AskForApproval::OnRequest` and separate sandbox modes. Permission policy values and their resolution are handled independently. A permission profile maps to filesystem and network sandbox controls; project trust does not grant unrestricted permission.
- On-request approval means that the system can escalate an operation that is not allowed by the effective sandbox or policy. It does not mean “confirm every command.” A test establishes `on-request` as the default for the covered case. Do not generalize that default to other profiles or managed environments, which can impose restrictions.
- Project trust is an explicit state. Trust must not be treated as approval for every effect. The effective configuration continues to govern profile selection, sandboxing, and approval.

### Shell and native file tools

- Shell commands run in a child process through an executor and process sandbox. The filesystem runner prepares the policy and requests a sandbox. If sandbox selection returns `SandboxType::None`, the runner returns an error. The inspected behavior does not support silently falling back to unrestricted shell execution when the required sandbox fails.
- Linux uses Bubblewrap and has a configurable legacy Landlock path. Missing prerequisites do not establish a permissive fallback. macOS and Windows use separate sandbox paths and centralized selection. This evidence does not establish exact functional equivalence across operating systems, and Huginn must not assume that Linux Landlock/Bubblewrap, the macOS sandbox, and a Windows restricted token are interchangeable.
- Profiles map to filesystem policies (read/write access, roots, and exceptions) and network policy. Native file operations pass through the executor's filesystem context and policy. That policy can reject unsupported access; it does more than request approval based on the tool name. Do not model all native APIs as one `write_file` operation with a universal approval rule.
- This evidence does not support requiring approval for every shell command or describing unsandboxed shell as the target behavior for Huginn. It also does not justify removing authorization: sandboxing and approval are independent controls.

### MCP: `AppToolApproval` and exceptions

- `AppToolApproval` defines four modes: `auto`, `prompt`, `writes`, and `approve`. The effective rules are: `auto` requests approval unless annotations establish read-only behavior or indicate destructive/open-world effects; `prompt` always requests approval; `writes` requests approval unless `read_only_hint=true`; and `approve` does not request approval. In `auto`, missing annotations are treated conservatively as destructive/open-world by default.
- Overrides can apply per server and tool. Approval decisions can also be remembered and persisted. This differs from Huginn's currently defined volatile grants scoped to a session, action, or path. These behaviors require deliberate design; they must not be assumed to exist in Huginn.
- Explicit exception: full-access mode skips MCP approval even when the per-tool mode would require a prompt. This exception must remain visible when describing parity. `approve` is also a configurable mode, not an effect of placing a working directory inside a project.
- MCP annotations are policy metadata, not process isolation. The inspected evidence does not establish that MCP servers inherit the shell sandbox. Huginn should preserve this distinction and must not describe MCP as sandboxed on this basis.

## Current Huginn limits and incompatible contracts

- `crates/huginn/runtime/src/tools.rs:500-523` describes shell as POSIX with explicit approval and time/output limits, but without sandboxing. Lines `508-523` create the approval request. This differs from sandboxed shell behavior. Approval alone does not resolve that difference.
- `crates/huginn/runtime/src/mcp.rs:235-257` prepares an approval request for every MCP call. This boundary does not implement the four modes or the per-server/per-tool override scheme. The exceptions and persistence behavior require design; they cannot be assumed.
- `crates/huginn/application/src/authorization.rs:13-34` implements `HostPolicy`; lines `60-155` and `164-189` constrain decisions and grants. The core defines `ApprovalPolicy` and `PreparedAction` in `crates/huginn/core/src/lib.rs:362-404` and `424-450`. These contracts preserve approval for tool effects, but do not by themselves implement OS sandboxing.
- Earlier contracts in `.agents/sdd/changes/huginn-autonomous-workspace-permissions/specs/` that define Huginn's autonomous catalog, prohibit autonomy for shell/MCP, require approval for each call, or state that shell is unsandboxed conflict with the later user decision to reproduce the inspected effective behavior. Reconcile them in later SDD phases; do not modify them during this exploration. The approved TUI mouse/keyboard requirements and approval hierarchy/feedback requirements are independent and remain in scope. This exploration does not revoke them.

## Topology and delivery slices

The current RPI plan in `.agents/rpi/plan/tasks/huginn-approval-interactions.md:12-19, 34-38` retains three slices: policy/revalidation, mouse interaction, and approval/feedback/documentation. The actual policy and sandbox work, including MCP, expands the policy slice. It requires authority/configuration/profiles, OS-specific sandbox backends, unavailable-sandbox handling, separation of native file effects from shell, and escalated approval integration. Do not assume this fits the previous slice or split policy across UI slices. Keep mouse support across menus as an independent deliverable; do not block it on the sandbox backend.

## Recommendation

Before implementation, review the proposal, specs, design, and tasks to resolve the applicable profile/default by mode; project trust; approval decisions versus sandbox capabilities; the actual catalog of native operations; MCP modes, exceptions, and persistence; the sandbox strategy for macOS, Linux, and Windows; and fail-closed behavior when enforcement is unavailable. Define slices around verifiable delivery boundaries. Keep the approved mouse and approval-UX slice independent. Do not claim macOS/Linux/Windows parity for Huginn without implementation and evidence for each platform.

## Risks

- Keeping the earlier contracts alongside the later decision can produce contradictory behavior: bounded autonomous native operations versus effective parity with the inspected behavior.
- Treating approval as a substitute for sandboxing could let shell commands with broad access act outside the workspace. The inspected behavior keeps these controls separate.
- A superficial shared backend could imply isolation that differs from the isolation actually enforced by each operating system.
- MCP could remain too restrictive if Huginn requires universal confirmation, or become too permissive if it trusts annotations or copies the `approve` exception without the relevant profile context.
- Sandbox and profile work may exceed the first slice and the RPI budget. Slice replanning must not absorb or drop the already approved mouse UX work.
