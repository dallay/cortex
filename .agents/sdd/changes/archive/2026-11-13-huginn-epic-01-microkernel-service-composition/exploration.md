# Exploration: Huginn EPIC 01 — Microkernel & Service Composition

> **Linear epic**: [DALLAY-718](https://linear.app/dallay/issue/DALLAY-718/huginnepic-01-microkernel-and-service-composition) / [#442](https://github.com/dallay/cortex/issues/442)
> **Change slug**: `huginn-epic-01-microkernel-service-composition`
> **Phase**: 1 — Explore
> **Artifact store**: `.agents/sdd/`
> **Baseline**: `origin/main` @ `03ba4f1` (working tree clean, `git diff origin/main...HEAD` empty)

## Intent

The epic is an **umbrella with a planning policy, not an implementation backlog**. Its
own text states: "No implementation tasks, child issues, sprint dates or release
promises are authorized by this issue. Before work begins: audit current `main`, open
PRs, existing issues and ADRs; select an approved vertical slice; then refine and create
implementation issues **in GitHub** so they synchronize to Linear."

This exploration delivers that audit. It establishes what the microkernel already
provides, which of the four declared capabilities are actually missing, and which single
vertical slice can carry the epic acceptance outcome.

## Epic scope as stated

1. Versioned contracts and deterministic service resolution
2. Plugin supervisor, dependency graph, lifecycle and reversible registrations
3. Typed events, ownership, scoped resolution and introspection
4. Explicit replacement of service providers with safe rollback

Architectural boundary (verbatim intent): model providers, session stores, tools, Agent
Loop and UIs remain **replaceable capabilities**, not privileged kernel implementations.

Epic acceptance outcome: "An alternative service implementation can be substituted
while consumers, authority checks and lifecycle guarantees continue to work."

## Baseline audit

### Repository and PR state

- `main` is at `03ba4f1`; the working tree is clean and identical to `origin/main`.
- Open pull requests: #508 (`EPIC 02` WASM component slice, `DALLAY-733`), #501, #500,
  #497 (Renovate), #337, #97 (release-please). **No open PR touches EPIC 01.**
- Recent merged Huginn work: #507, #505, #504, #499, #492, #489 (protected session
  composition), #488 (bounded local restriction snapshots). None implements EPIC 01.
- Open GitHub issues are the 15 Huginn epics (#442–#457) plus the EPIC 02 slice #506. **No implementation issue exists for EPIC 01.**

### ADRs already accepted

- `ADR-0003` — minimal service composition kernel. Kernel owns service resolution,
  plugin lifecycle, generation identity, effect cleanup and diagnostics. One
  process-level resolution scope, one selected provider per service, explicit major
  versions. Missing hard dependencies block activation; ambiguous providers and
  dependency cycles fail explicitly. Activation publishes registrations only on
  success and cleans partial effects on failure. Dependents stop accepting new work and
  stop before a required provider is removed.
- `ADR-0012` — hexagonal application and lifecycle-owned capabilities. Confirms the
  kernel, ports and composition root already exist and must not be recreated.
- `ADR-0013` — authorable plugin contract and loading. Static native composition is
  accepted; runtime loading stays deferred to the WASM platform epic.

### Existing foundation to preserve (must not be recreated)

| Capability                     | Evidence                                                                                                                         | State                |
|--------------------------------|----------------------------------------------------------------------------------------------------------------------------------|----------------------|
| Versioned service identity     | `crates/huginn/core/src/kernel.rs:18-22` (`ServiceId::new(name, major)` → `name@major`)                                          | Present              |
| Transactional activation       | `kernel.rs:278-330` — staged registrations, manifest-vs-actual equality check, publish-then-commit                               | Present              |
| Failure rollback               | `kernel.rs:114-127` — cleanup cancels, clears commits/resources, drains owned tasks with a 5s deadline                           | Present              |
| Generation identity            | `kernel.rs:49-76` — `Activation` with generation, published flag, cancellation token                                             | Present              |
| Per-generation revocation      | `crates/huginn/core/tests/registration_ownership.rs` — resources drop on failure, remove, shutdown and supervisor drop           | Present              |
| Ambiguous provider rejection   | `kernel.rs:210-223` — `provider_owners()` fails on duplicate `ServiceId`                                                         | Present              |
| Dependency cycle detection     | `kernel.rs:225-260` — DFS over manifest `requires` → provider owners                                                             | Present              |
| Deterministic activation order | `kernel.rs:191-208`, `kernel.rs:262-276` — `BTreeMap` iteration + readiness filter                                               | Present              |
| Ownership of owned resources   | `kernel.rs:103-109` — `own_task`, `own_resource`                                                                                 | Present              |
| Port-only composition          | `crates/huginn/application/src/composition.rs` — `ModelPlugin`, `ToolsPlugin`, `SessionsPlugin`, `LoopPlugin` behind `ServiceId` | Present              |
| Loop-scoped typed events       | `crates/huginn/core/src/lib.rs:127-190` — `Event`, `EventSink`, `ScopedEventSink`                                                | Present (loop scope) |

## Gap matrix

| Epic capability                                                      | Present                                                                                                                                 | Missing                                                                                                                                                                                                                                                                                        |
|----------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| 1. Versioned contracts, deterministic resolution                     | `ServiceId` with explicit major version; exact-match resolution; deterministic ordering; ambiguous providers and cycles fail explicitly | No major-version compatibility rule (a consumer requiring `agent:model@2` cannot consume `agent:model@1`); no multiple-candidate selection semantics                                                                                                                                           |
| 2. Supervisor, dependency graph, lifecycle, reversible registrations | Supervisor with `register`/`start`/`remove`/`shutdown`; dependency graph; transactional publication; rollback; per-generation cleanup   | No restart or replacement of an already-active plugin; `remove` marks dependents `Blocked` and never reactivates them when the dependency returns; no mechanism forces in-flight consumers to stop accepting new work after removal                                                            |
| 3. Typed events, ownership, scoped resolution, introspection         | Loop-scoped typed events exist; `Diagnostic` exposes manifest, state, generation and last error                                         | No kernel lifecycle event stream; ownership is recomputed locally inside `start()` and never exposed; no dependents map; no service → provider introspection; resolution is a flat global map with no scope concept (`PluginContext.services` is a filtered snapshot taken at activation time) |
| 4. Explicit replacement with safe rollback                           | —                                                                                                                                       | **Nothing exists.** There is no `replace` operation on the supervisor or the composition root                                                                                                                                                                                                  |

## Consequences that matter for the acceptance outcome

The epic outcome requires that "consumers, authority checks and lifecycle guarantees
continue to work" when a service is substituted. Three concrete gaps block it today:

1. **No substitution path.** `Supervisor` exposes only `register`, `start`, `remove` and
   `shutdown`. A replacement can only be approximated by removing the provider and
   registering a new plugin, which permanently blocks dependents and cannot restore the
   previous provider if the new one fails to activate.
2. **In-flight authority is not revoked by removal.** Consumers hold an `Arc<dyn
   Trait>` obtained from `resolve`. After `remove`, the map entry disappears and the
   generation token is cancelled, but an existing holder keeps working. `ADR-0003`
   requires dependents to "stop accepting new work and stop before a required provider
   is removed".
3. **No introspection of who owns what.** `provider_owners()` is a private helper
   discarded after `start()`. Nothing can answer which plugin provides a service or
   which plugins depend on it, so a replacement cannot even decide who to drain.

## Candidate vertical slices

### Option A — Service replacement with transactional rollback (recommended)

Add a first-class replacement path: `Supervisor::replace_service` (or the equivalent
composition-root operation) that registers an alternative provider, validates the
manifest, activates it transactionally, publishes only on success, and **restores the
previous provider and its dependents when activation fails**. Supporting work: expose
ownership and dependents for the drained set, and make dependents stop accepting new
work across the swap.

- Pros: it is the slice that directly satisfies the epic acceptance outcome; it
  exercises capabilities 1, 2, 3 and 4 together; it forces the kernel to stay
  capability-agnostic, because the substituted service can be a model provider, a
  session store, a tool registry or the loop.
- Cons: it is the largest of the candidates and touches `huginn-core` plus the
  composition root; it needs a precise rollback contract before code is written.
- Complexity: High.

### Option B — Ownership, dependents and scoped resolution

Make ownership a first-class, introspectable fact: persist the service → provider map,
expose dependents, give plugins a resolution scope instead of a flat global map, and
publish diagnostics that answer "who provides this and who depends on it".

- Pros: smaller, self-contained, and it is the prerequisite for any safe replacement;
  it also removes the flat-global-resolution smell in the kernel.
- Cons: it does not by itself demonstrate the epic acceptance outcome; a substitution
  still has to be built on top later.
- Complexity: Medium.

### Option C — Kernel lifecycle event stream

Emit typed kernel events (discovered, activated, deactivated, replaced, blocked,
failed) to subscribers, so the TUI and diagnostics can observe composition changes.

- Pros: enables observation, useful for the TUI epic and for diagnostics; low risk.
- Cons: it adds a new event contract without proving the substitution guarantee; it is
  the most deferrable of the three.
- Complexity: Medium.

### Recommended sequence

A is the only candidate that delivers the epic outcome, and it needs B. C is
independent and can follow. The epic authorizes **one** slice, so the recommendation is
Option A with Option B's ownership work folded in as its prerequisite, and C deferred.

## Constraints and risks

- `huginn-core` must stay free of terminal, HTTP, SQLite and Rook dependencies; the
  kernel must not learn about models, tools or sessions.
- The `plugin-api` author-facing contract is versioned (`huginn-plugin-api::v1`); any
  new kernel surface that plugins can observe must respect that versioning and must not
  break the existing v1 contract.
- The EPIC 02 slice is open in PR #508 and touches the same runtime surface. The EPIC 01
  slice must not conflict with that branch.
- `unsafe_code = "deny"` and clippy `-D warnings` are mandatory workspace invariants.
- TDD is the default for executable behavior: each slice starts with a failing test.

## Evidence gaps

- The Huginn capability master catalog linked from the epic is a Google Drive document
  that requires Google sign-in; it could not be read from this environment. The audit
  above is based on the epic text, the accepted ADRs, `main` and the open PRs.
- Linear issue list was queried through the Linear MCP; the epic body and the EPIC 02
  slice body were read directly. No other EPIC 01 child issue exists.

## Next phase

Propose the selected slice as an implementation change with an explicit rollback
contract, an explicit authority contract for dependents during a swap, and a
verification plan that proves the epic acceptance outcome. The refined slice must be
created as a GitHub issue first so it synchronizes to Linear, per the epic planning
policy.
