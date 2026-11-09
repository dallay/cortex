# Design: Autonomous Workspace Permissions and Platform Parity

## Technical approach

Implement the change in dependency order. Establish policy outcomes and one-operation `Allow once` behavior first. Before any sandbox implementation or platform-support claim, resolve the blocking macOS backend decision below. Apple explicitly documents `sandbox-exec(1)` and `sandbox_init(3)` as deprecated; `sandbox_init(3)` further states that predefined profiles must not be used when building against SDK >= 27.0 and that processes are killed. Do not select or implement either deprecated mechanism without explicit user approval. Until a supported macOS approach is established, shell execution on macOS MUST fail closed (or remain disabled), and macOS persistent grants MUST remain unavailable. Then evaluate candidate native isolation on Linux and Windows and prove, on each supported OS, that same-identity shell processes cannot read, enumerate, create, modify, or replace the future user grant registry or any ancestor needed to access or replace it. Until the relevant OS-specific proof passes, Huginn MUST neither load nor honor persisted grants, MUST NOT offer a persistent choice, and MUST NOT let stale or modified stored state suppress a prompt. Shell execution fails closed if the required sandbox cannot be enforced. Preserve `unsafe_code = deny`; do not add unsafe Rust as a workaround.

Only after isolation proof may the implementation evaluate and integrate a user-owned grant store, with real OS security tests. Then implement exact grant matching and revocation, followed by native filesystem and MCP integration, TUI completion, and final CI/review gates. A successful build, file permission check, registry ownership check, or common backend interface is not isolation evidence. Do not claim cross-platform parity until each OS passes its own runtime security tests.

Shell child processes run with the same OS user identity as Huginn: the same UID on macOS/Linux or `TokenUser` on Windows. `private-fs` 0.2.1 explicitly excludes same-UID/TokenUser attackers from its security guarantee. It is therefore only a conditional storage candidate and never an isolation control. It compiled in an external scratch project with Rust 1.89, but that provides no OS integration evidence. Huginn's package MSRV is 1.89, the Cargo workspace MSRV is 1.81, and the selected toolchain is 1.99; any dependency placement and compatibility decision must account for all three. Current CI excludes Huginn tests and clippy on Windows. Platform mechanisms remain candidates, not decisions: Linux Landlock may need namespace isolation and/or a dedicated launcher to cover the required filesystem and process boundary; kernel/API availability, inherited restrictions, and launcher behavior require validation. Windows AppContainer and ACL-based approaches have token, capability, object-coverage, and process-launch constraints; ACL checks alone do not prove isolation against same-token shell processes. No committed backend decision exists for either platform.

Managed restriction-source verification remains a separate unresolved, blocking deliverable. Keep authorization, sandbox enforcement, native filesystem checks, managed policy, MCP policy, and behavioral instructions as independent boundaries. Preserve explicit user decisions and exact operation scope: `Allow once` covers only the presented operation; a persistent grant, if eventually enabled, is limited to the canonical workspace root and exact capability/action/resource. Matching grants remain subject to current policy and sandbox enforcement.

## Architecture decisions

### Decision: Resolve policy and one-operation approval before persistence

**Choice:** First implement policy resolution and bounded `Deny`/`Allow once` behavior in `crates/huginn/application/src/permission_policy.rs` and `crates/huginn/application/src/authorization.rs`. Keep policy eligibility distinct from presentation. A policy-permitted request may be approved once; this decision is never stored. Preserve managed denials, invalid-policy denial, and safe cancellation as final non-execution outcomes.

**Alternatives considered:** Build the grant registry and prompt UI before establishing policy outcomes, or represent `Allow once` as a persistent/session grant.

**Rationale:** This yields a safe, useful authorization path without making persistent state a prerequisite or confusing a one-operation choice with durable authority.

### Decision: Prove native sandbox isolation before grant storage access

**Choice:** Treat platform backend selection as unresolved. The macOS mechanism is a blocking open decision before sandbox code or macOS sandbox-support claims. `sandbox-exec(1)` and `sandbox_init(3)` are explicitly deprecated by Apple; the `sandbox_init(3)` documentation says predefined profiles must not be used when building against SDK >= 27.0, or processes are killed. Do not choose or implement a deprecated macOS backend without explicit user approval. Record these macOS paths for a decision: (1) a temporary `sandbox-exec` exception, with deprecation and removal risk; (2) fail closed or disable shell on macOS pending a supported mechanism, with no macOS parity; (3) investigate/redesign around signed App Sandbox packaging and workspace access, a large product change whose feasibility as a dynamic per-workspace CLI sandbox is not proven. Apple recommends the signed App Sandbox model, but it is not established as a drop-in replacement for Huginn's dynamic per-workspace shell sandbox. Linux Landlock (potentially combined with namespaces and/or a launcher) and Windows AppContainer/ACL are investigation candidates only; their constraints and ability to enforce the complete policy remain unproven. No committed backend decision exists. After a supported backend is selected for an OS, implement its adversarial runtime probes before adding or activating any persistent grant-store integration. Probe shell access to the registry location and every ancestor for read, enumeration, creation, modification, and replacement. Tie proof to the actual enforcement configuration used for shell execution. If a platform/backend cannot demonstrate every denial, persistent grants stay unavailable and shell execution that requires missing isolation is rejected; there is no unsandboxed fallback.

**Alternatives considered:** Load grants based on owner/mode/ACL checks, rely on private storage, infer safety from compilation or backend interfaces, treat deprecated macOS APIs as an approved implementation, or enable persistence on platforms without runtime evidence.

**Rationale:** Huginn and its shell children share an OS identity. Storage protections that exclude same-principal attackers cannot protect authorization state from shell code. Native sandbox isolation is therefore a strict prerequisite, not a property of the store. On macOS, deprecated mechanisms and the unresolved signed App Sandbox fit cannot support a safe implementation decision by assumption. User approval and a supported mechanism are required before proceeding with macOS sandbox code.

### Decision: Block macOS sandbox implementation pending supported mechanism approval

**Choice:** No macOS sandbox backend is selected in this design. Stop before implementing or claiming one until the user explicitly resolves the mechanism and its support constraints. Pending that decision, macOS shell execution must fail closed or remain disabled; persistent grants and macOS parity claims remain blocked. `unsafe_code = deny` remains in force.

**Alternatives considered:** Use deprecated `sandbox-exec(1)` temporarily; use deprecated `sandbox_init(3)` and a predefined profile; assume Apple's signed App Sandbox is a dynamic CLI-compatible substitute; or proceed with Linux/Windows and imply full platform parity.

**Rationale:** Apple marks both legacy interfaces deprecated, and `sandbox_init(3)` documents the SDK >= 27.0 predefined-profile process-kill constraint. Apple's recommended signed App Sandbox is materially different from a demonstrated dynamically configured per-workspace CLI sandbox. Choosing it without evidence would turn an unresolved platform and product-design issue into an unsupported security claim.

### Decision: Treat Linux and Windows mechanisms as candidates, not committed backends

**Choice:** Investigate Linux Landlock, potentially with namespaces and/or a launcher, and Windows AppContainer/ACL-based designs. Do not freeze an API, dependency, or architecture until real-platform probes show the required registry and ancestor access denials, process-launch behavior, and policy limits. ACL ownership/protection validation is not a substitute for shell isolation. Preserve `unsafe_code = deny` throughout.

**Alternatives considered:** Treat a shared interface, compilation, ACL configuration, or nominal OS feature availability as proof; claim parity from Linux and Windows alone.

**Rationale:** Kernel/API versioning, capability coverage, token behavior, inheritance, and process launch details constrain these approaches. None has been selected or verified in this change context. Linux/Windows implementation alone does not complete macOS/Linux/Windows parity.

### Decision: Evaluate `private-fs` 0.2.1 only as conditional storage

**Choice:** Consider `private-fs` 0.2.1 only as a possible user-owned storage adapter after native isolation has passed. Do not use it to establish isolation or as a standalone authorization safeguard. Evaluate its API/security fit and compatibility separately for the Huginn package and the workspace before adoption. The external Rust 1.89 scratch compilation is not repository integration or OS security evidence; no OS integration tests have been completed for it.

**Alternatives considered:** Adopt it immediately, use it as the sandbox, or infer safe operation from its successful scratch compilation.

**Rationale:** Its documented threat boundary excludes the exact same-UID/TokenUser attacker model that applies to Huginn shell processes. Package compilation and storage behavior must not substitute for native isolation proof.

### Decision: Keep stored grants dormant until current-OS isolation is verified

**Choice:** At startup and before authorization, establish a verified isolation capability for the current OS and the current registry path/ancestors. If the proof is absent, failed, stale, or unavailable, do not open or parse the grant registry; ignore any grant as an authority and do not let it suppress a prompt. In that state, expose only `Deny` and `Allow once` when policy permits, and never present `Allow for this workspace`. If isolation is required to run the shell operation and is unavailable, do not spawn the process.

**Alternatives considered:** Read grants but decline to apply them, retain a persistent UI choice behind a warning, or trust previously verified state without rechecking the active sandbox configuration.

**Rationale:** Even reading or enumerating the registry can reveal protected state, and a dormant-looking entry can otherwise suppress an approval prompt. The fail-closed boundary must precede store access and all persistent-choice behavior.

### Decision: Preserve narrow user choices and revocation semantics

**Choice:** After isolation and store-security gates pass, store only explicit `Allow for this workspace` choices in user-owned state. Bind each grant to canonical workspace root and exact capability/action/resource identity. Provide user-visible listing, editing, and revocation. Revalidate policy, identity, and scope at effect time; revocation applies before subsequent authorization. Never import project-controlled grants or promote existing session decisions.

**Alternatives considered:** Project-local grants, broad workspace trust, implicit persistence, or a grant that applies across capabilities or resources.

**Rationale:** Explicit scope preserves the user's chosen boundary and prevents workspace location or project contents from creating authority. Revocation is meaningful only if checks consult current state before each later effect.

### Decision: Keep managed restrictions unresolved and independently blocking

**Choice:** Keep managed restrictions as a distinct restriction-only authority with a separately reviewed protected-source verifier. Invalid, unreadable, or unverifiable managed state must fail closed; do not interpret verifier failure as verified absence. Do not claim the managed-source verifier is complete, and do not let grant-store or sandbox completion close this gate.

**Alternatives considered:** Load administrator rules from ordinary user/project files, merge sources as equivalent, or silently ignore an unverified source.

**Rationale:** A user grant cannot override a managed restriction. The source verifier has separate ownership, integrity, and platform requirements and remains unresolved/blocking.

### Decision: Keep enforcement and effect families separate

**Choice:** Approval never disables or broadens the selected sandbox. Native filesystem tools retain path, root, symlink, capability, and resource checks at each effect boundary. Native workspace grants do not authorize shell or MCP. MCP retains server/tool/action/resource policy, and behavioral instructions remain guidance rather than host authorization.

**Alternatives considered:** Treat approval as profile elevation, reuse file grants for shell/MCP, or treat behavioral text or MCP annotations as isolation.

**Rationale:** These controls have different scopes and trust boundaries. Separation prevents one user choice from silently expanding authority.

### Decision: Do not claim platform parity without per-OS evidence

**Choice:** Treat Linux, macOS, and Windows as separate support claims. Each requires implementation and real-platform evidence for shell isolation, registry/ancestor denial, and the relevant Huginn tests. Add blocking Windows Huginn test and clippy coverage; current CI excludes Huginn there. Shared contracts, compilation, stubs, and fail-closed behavior alone do not establish parity.

**Alternatives considered:** Claim parity from a shared profile/backend interface, cross-compilation, or Linux/macOS-only CI.

**Rationale:** Native sandbox mechanisms and security behavior differ by OS. Evidence must test the actual backend and shell process on each target.

## Data flow

```text
1. Policy settings + managed-source status
   -> application resolves allowed / prompt-eligible / final-denial outcome
   -> eligible request presents Deny or Allow once
   -> Allow once authorizes only the prepared operation; no persistence

2. Platform decision and candidate evaluation
   -> macOS backend approval is a blocking prerequisite; no deprecated backend is selected implicitly
   -> evaluate Linux Landlock with any required namespace/launcher support
   -> evaluate Windows AppContainer/ACL constraints and process launch
   -> unresolved, unsupported, or unverified platform fails closed

3. Per-OS native sandbox implementation after backend decision
   -> launch adversarial shell probes under the real sandbox
   -> verify denial of read/enumerate/create/modify/replace for registry + ancestors
   -> only a passing current-OS proof enables the next stage

4. Verified isolation capability + user-owned store candidate
   -> evaluate storage API, compatibility, and real OS security behavior
   -> integrate store only after its separate gates pass
   -> otherwise do not load/honor grants or offer persistent choice

5. Explicit persistent user choice
   -> exact canonical workspace/capability/action/resource record
   -> matching authorization rechecks current policy and isolation
   -> revocation/edit is reflected before the next authorization

6. Native filesystem / MCP / TUI integration
   -> native effects revalidate path/resource boundaries
   -> MCP uses its independent server/tool policy
   -> presentation displays only eligible choices and safe denial/cancel states
```

Managed restrictions are loaded and verified through a separate source path. Failure of that required verifier remains a final fail-closed outcome. No flow above treats behavioral instructions as host authority.

## Interfaces / contracts

- **Policy decision (application):** Distinguish allowed, prompt-eligible, and final-denial outcomes, including source/reason sufficient for the host and UI. `Allow once` applies only to the prepared operation.
- **Isolation capability (runtime):** A target-specific result must attest that the active sandbox prevents shell read, enumeration, creation, modification, and replacement access to the configured grant registry and every ancestor needed to reach or replace it. Missing or unverifiable capability means no grant-store access and no persistent choice. It is not inferred from storage metadata.
- **Workspace grant (application contract):** Canonical workspace root plus exact capability, action, and resource identity, with explicit user-choice provenance. Define resource matching safely; lexical path-prefix matching is insufficient. Grant lookup is disabled until the isolation capability is verified.
- **Grant store (apps/config adapter):** After prerequisites pass, support verified user-owned load, explicit atomic write, listing/editing, and revocation. Reject malformed or unverifiable state. Never accept project-controlled grant writes. Its own security tests do not replace sandbox probes.
- **Managed policy source (apps/config adapter):** Separate verified-absence, valid restriction-only configuration, and verification/read/parse failure outcomes. Protected locations and a complete cross-platform verifier remain unresolved and blocking.
- **Sandbox backend (runtime):** OS-specific enforcement; no unsandboxed fallback. Approval cannot change selected profile or sandbox.
- **Platform backend status:** macOS mechanism is a blocking open decision; no deprecated backend without explicit user approval. Linux Landlock (possibly namespaces/launcher) and Windows AppContainer/ACL are investigation candidates, not selected implementations. Do not claim parity from Linux/Windows evidence while macOS remains unresolved or unavailable.
- **MCP policy:** Preserve independent server/tool/action/resource authorization. Native workspace grants are not MCP grants.

## File changes

| File | Action | Description |
|---|---|---|
| `crates/huginn/application/src/permission_policy.rs` | Modify | Establish explicit policy outcomes and prompt eligibility; keep managed restrictions final and separate from user choices. |
| `crates/huginn/application/src/authorization.rs` | Modify | Implement bounded `Deny`/`Allow once` first; later add grant lookup and exact scope only behind verified current-OS isolation. |
| `crates/huginn/runtime/src/` sandbox modules | Add/modify only after platform decisions | Implement only selected, supported OS backends and the capability result required to gate all persistent-store access. macOS implementation is blocked pending explicit mechanism approval; Linux/Windows candidates require evidence before selection. No parity assumption from common interfaces. |
| Target-specific runtime manifests/configuration | Modify only after backend selection | Configure supported native sandbox backends without weakening workspace lint, `unsafe_code = deny`, or introducing an unsandboxed fallback. |
| `crates/huginn/runtime/tests/` | Add/modify | Add adversarial real-platform probes for registry and ancestor read/enumerate/create/modify/replace denial, plus sandbox failure/no-spawn cases. |
| `apps/huginn/src/config.rs` | Modify after isolation gate | Evaluate and, if approved by evidence, implement a user-owned grant store. `private-fs` 0.2.1 is conditional storage only; verify compatibility at Huginn MSRV 1.89 and workspace MSRV 1.81 under toolchain 1.99. |
| `apps/huginn/src/main.rs` | Modify by phase | Compose policy and sandbox first; only instantiate/load the store after the current OS isolation proof passes; later wire grant management and revocation. |
| `crates/huginn/application/tests/` and `apps/huginn/tests/` | Add/modify | Cover policy, `Allow once`, fail-closed dormant-store behavior, exact grant scope, matching, revocation, and non-matching decisions. |
| `crates/huginn/runtime/src/tools.rs` and native filesystem adapters | Modify in later integration phase | Preserve native path, root, symlink, and effect-boundary checks independently of authorization and shell isolation. |
| `crates/huginn/runtime/src/mcp.rs` | Modify only as needed | Maintain separate MCP policy; do not inherit workspace file grants or shell-isolation claims. |
| `crates/huginn/presentation/src/approval.rs` and related TUI code | Modify in later integration phase | Show `Deny`/`Allow once` before the persistence gate; show persistent choice only after it passes. Preserve reviewed scope, mouse/keyboard operation, and safe cancellation. |
| `.github/workflows/ci.yml` | Modify | Add Windows Huginn test and clippy gates and real-platform security coverage. Current Windows jobs exclude Huginn. |
| Managed-policy adapter in `apps/huginn/src/` | Separate unresolved work | Protected-source verifier remains blocking; do not mark as implemented by this change's sandbox/store evidence. |
| Huginn documentation and configuration examples | Modify in final integration | Document exact choice scopes, sandbox gate, disabled persistent behavior before proof, platform-specific evidence, and unresolved managed-source verification. |

Exact module boundaries and any dependency addition remain implementation decisions. Preserve the existing Huginn layering: core owns contracts, application owns orchestration, runtime implements I/O, presentation owns UI, and `apps/huginn` is the composition root.

## Testing strategy

| Layer | What to test | Approach |
|---|---|---|
| Policy and approval | `Deny`, one-operation `Allow once`, `OnRequest`, managed denial, invalid policy, unavailable approver | Deterministic application tests. Verify no persistent state is written for `Allow once`. |
| Sandbox unit/contract | Unsupported or failed enforcement, no-spawn behavior, capability reporting | Fake backends for deterministic failure contracts; these tests do not establish OS support. |
| Real OS isolation | Shell cannot read, enumerate, create, modify, or replace the grant registry and every ancestor on macOS, Linux, and Windows | Run adversarial subprocess probes inside the actual selected native sandbox on each OS. Require explicit pass evidence for every operation and target; inability to run is a blocked gate, not a pass. macOS probes cannot authorize a deprecated backend absent explicit user approval and a decision; until then macOS shell remains disabled/fail-closed and the gate is blocked. |
| Grant-store evaluation | `private-fs` API/security fit, maintenance, target behavior, ownership, atomic updates, links/ancestors, and toolchain compatibility | Conditional candidate review only after sandbox probes. External Rust 1.89 compilation is not integration evidence. Run real OS security tests before adoption. |
| Dormant-store fail-closed | Missing/failed isolation proof never reads/honors stale grants, suppresses prompts, or exposes persistent choice | Instrumented store fake that fails the test if accessed; assert only policy-permitted `Deny`/`Allow once` are offered. |
| Grant matching and revocation | Exact canonical root/capability/action/resource match, mismatches, current-policy override, revocation | Deterministic store/application tests plus real store integration after security gates. Test subsequent operation behavior after revocation. |
| Native filesystem | Read/write/edit/list/search boundaries, roots, symlinks, traversal, stale resource/content | Temporary fixtures and zero-effect assertions; revalidate immediately before effects. |
| Managed source | Protected source verification, verified absence vs invalid source, restriction-only semantics | Remains an unresolved blocking gate. Until reviewed verifier and real OS evidence exist, test and preserve fail-closed behavior; do not claim source verification. |
| MCP and TUI | MCP modes remain separate; approval choices, review completeness, keyboard/mouse, resize, cancellation, no-op feedback | Independent policy tests and deterministic terminal/PTY interaction tests. Persistent choice is absent until isolation is proven. |
| CI and architecture | Huginn suites, clippy, architecture boundaries, platform-specific security probes | Add blocking Windows Huginn test/clippy; run `just huginn-architecture` and focused crate suites. Do not call compilation parity. |

## Migration / rollout

Do not import project-file grants or promote session-scoped decisions. Start with no persistent grants. Existing persisted or externally supplied grant data remains inert until the active OS sandbox has passed every required adversarial probe and the store has separately passed its security and compatibility evaluation. In the unverified state, do not load or honor the store, do not suppress a prompt, and do not offer a persistent choice. If shell isolation itself is unavailable, reject shell execution.

Implementation sequence and gates:

1. **Policy and `Allow once`:** implement policy outcomes, prompt eligibility, denial/cancellation, and operation-scoped `Allow once`; do not introduce persistent store access.
2. **Platform mechanism decision:** resolve the macOS backend as a blocking decision before any sandbox implementation or macOS sandbox-support claim. Apple documents `sandbox-exec(1)` and `sandbox_init(3)` as deprecated; do not choose either without explicit user approval. Compare temporary `sandbox-exec` use (deprecation/removal risk), fail-closed/disabled macOS shell (no parity), and investigate/redesign for signed App Sandbox packaging/workspace access (large product change; dynamic per-workspace CLI feasibility unproven). Investigate Linux Landlock (possibly namespaces/launcher) and Windows AppContainer/ACL constraints without treating either as committed. Keep persistent grants disabled.
3. **Native sandbox and isolation probes:** after supported backend decisions, implement only those selected backends and real-platform probes for shell read/enumerate/create/modify/replace access to the registry and all ancestors. Failure or unresolved mechanism on any OS blocks that OS's persistent grants and parity claim; there is no fallback.
4. **Store evaluation and integration:** only after a target OS passes isolation, evaluate conditional storage, toolchain placement/compatibility, and real OS store security tests. Add store integration behind the proven isolation capability. Until each applicable gate passes, fail closed without opening the registry.
5. **Grant matching and revocation:** add exact scope matching, explicit persistent-choice recording, listing/editing/revocation, and checks proving revocation governs the next authorization.
6. **Native filesystem, MCP, and TUI:** complete effect-boundary validation, preserve MCP separation, and finalize accessible review/approval interactions with only currently eligible choices.
7. **Final CI and review:** add/run blocking per-OS tests and clippy, including Windows Huginn coverage; run architecture and relevant quality gates. Review the managed-source verifier as a separate unresolved blocker. Do not claim parity unless each OS has real evidence, and do not claim managed verification is complete without its independent gate.

No phase may skip a prior security dependency. Rollback or failed evidence disables persistent grants for the affected OS; it must not fall back to project state, session remembrance, a broader grant, or unsandboxed shell execution. Revocation cannot reverse effects that already occurred.

## Open questions

- **Blocking:** Which supported macOS mechanism, if any, will be approved for shell isolation? Do not begin macOS sandbox implementation or claim macOS sandbox support until this is resolved. Options are a temporary deprecated `sandbox-exec` exception with deprecation/removal risk; fail-closed/disabled macOS shell pending a supported mechanism (no parity); or investigate/redesign signed App Sandbox packaging/workspace access (large product change; dynamic per-workspace CLI feasibility unproven).
- Which Linux Landlock design, possibly combined with namespaces and/or a launcher, and which Windows AppContainer/ACL design can prove all required registry and ancestor access denials for same-UID/TokenUser shell processes? No backend decision is committed; constraints need runtime evidence.
- Does `private-fs` 0.2.1 meet the user-store requirements after isolation proof, and can it be used without violating Huginn's 1.89 MSRV and the workspace's 1.81 MSRV under the 1.99 toolchain? Its scratch compilation used Rust 1.89 and had no OS integration tests.
- What exact resource matching relation supports intended narrow write scope without prefix, symlink, or canonicalization bypasses?
- Which settings surface will expose persistent grant listing, editing, and revocation?
- Which protected locations and reviewed APIs verify the managed restriction source and its full ancestor chain on each OS? This is unresolved and independently blocks final parity/release claims.
- What evidence and release criteria will mark isolation as verified for a particular OS and sandbox configuration, and how will configuration changes invalidate that evidence?

## Risks

- Same-identity shell code may read or alter a user-owned registry unless native isolation is proven for the registry and every ancestor. Keep all persisted state dormant and persistent choices unavailable until per-OS adversarial probes pass.
- `private-fs` 0.2.1 excludes same-UID/TokenUser attackers. Treating it as isolation would create a direct authorization bypass. Its external scratch compilation is not a security test.
- Toolchain constraints differ: Huginn MSRV 1.89, workspace MSRV 1.81, selected toolchain 1.99. A dependency can compile in an app-only scratch project and still be unsuitable for workspace integration.
- Current CI excludes Huginn on Windows. Until Windows Huginn tests, clippy, and actual runtime probes run as blocking gates, there is no Windows parity evidence.
- macOS has no approved sandbox backend in this design. Apple's `sandbox-exec(1)` and `sandbox_init(3)` documentation marks these mechanisms deprecated; `sandbox_init(3)` warns against predefined profiles with SDK >= 27.0 and says processes are killed. An explicit user decision is required before considering a deprecated exception. Apple's signed App Sandbox recommendation is not proven to provide dynamic per-workspace CLI isolation. Until resolved, macOS shell must fail closed or remain disabled, persistent grants stay blocked, and platform parity is incomplete.
- Linux Landlock plus possible namespaces/launcher and Windows AppContainer/ACL are candidates only. API/kernel support, token/capability limits, and launcher/object coverage may prevent equivalent enforcement; Linux/Windows results alone do not establish three-platform parity.
- Native isolation may not be demonstrable on one or more target OSes. Persistent grants and cross-platform parity must remain unavailable/unclaimed there; never substitute a weaker storage check.
- The managed-source verifier remains unresolved and blocking independently of sandbox/store work. Treating verification failure as source absence could bypass administrator restrictions.
- Broad or unstable resource matching can expand a grant. Require canonical identity, exact operation scope, effect-boundary revalidation, and adversarial path tests.
- UI or task ordering may accidentally expose a persistent choice before the gate passes or let an old grant suppress prompting. Test choice availability and dormant-store behavior as explicit security properties.

## Platform research references

- Apple `sandbox-exec(1)` manual: deprecated. `https://developer.apple.com/library/archive/documentation/Security/Conceptual/AppSandboxDesignGuide/`
- Apple `sandbox_init(3)` manual: deprecated; predefined profiles must not be used when building against SDK >= 27.0 because processes are killed. See the Apple Developer Documentation entry for `sandbox_init(3)`.
- Apple App Sandbox design guidance recommends the signed App Sandbox model. This guidance does not establish that signed entitlements provide a drop-in dynamic per-workspace sandbox for this CLI. `https://developer.apple.com/library/archive/documentation/Security/Conceptual/AppSandboxDesignGuide/`
- Linux Landlock documentation: `https://docs.kernel.org/userspace-api/landlock.html`
- Windows AppContainer overview: `https://learn.microsoft.com/windows/win32/secauthz/appcontainer-for-legacy-applications`
- Windows access-control overview: `https://learn.microsoft.com/windows/security/identity-protection/access-control/access-control`
