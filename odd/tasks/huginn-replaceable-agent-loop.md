# Huginn replaceable agent loop (DALLAY-720 / GitHub #444)

Feature branch: `feature/dallay-720-huginnepic-03-replaceable-agent-loop-execution-control`, based on `origin/main` @ `1216d4e`.
Deliverable: the complete epic — a functional agent loop whose concrete strategy is a kernel plugin, certified by one conformance suite shared by two distinct implementations.

## Specs

- **S1 — Full epic, functional loop.** L4: "el objetivo mío es resolver esta EPIC completa y tener la lista, tener todo lo que es el loop de la gente funcional, todo el loop de la gente funcional". The deliverable is the whole epic's capability set, working end to end, not one slice.
- **S2 — Loop is a kernel plugin.** L4: "que sea como dicen en la misma descripción de la issue que va a ser un plugin del núcleo del kernel de nuestro sistema". The epic's own words: "Replaceable AgentLoop service". A third-party, statically composed plugin must be able to provide `agent:loop@1` without editing the host.
- **S3 — Two implementations, one suite.** Epic verbatim: "Two distinct loop implementations pass the same behavioral and security conformance suite, including cancel, denial, recovery and streaming."
- **S4 — Host owns authorization and effects.** Epic verbatim: "The host owns authorization and effects regardless of loop implementation. Avoid double invocation and automatically replaying effects with unknown outcomes."
- **S5 — Turns and lifecycle events.** Epic verbatim: "Inference→tool→inference turns and streamed lifecycle events".
- **S6 — Control surfaces.** Epic verbatim: "Interrupt, cancellation, steering, queued messages and budget enforcement".
- **S7 — Hooks, retries, recovery.** Epic verbatim: "Pre/post inference hooks, effect-aware retries and recovery".
- **S8 — Pinning, concurrency, replay safety.** Epic verbatim: "Context pinning, concurrency/backpressure and replay-safe semantics".
- **S9 — Turn and step identity.** Epic verbatim: "Replaceable AgentLoop service; turn and step identity".
- **S10 — Preserve completed functionality.** Epic verbatim: "Preserve completed functionality; do not treat this scope list as an unimplemented backlog." Existing cancellation, no-replay recovery, approval authority, streaming events, limits, tool hooks and session leases stay behaviorally intact.
- **S11 — Issues land in GitHub.** Epic verbatim: "refine and create implementation issues **in GitHub** so they synchronize to Linear". Satisfied by #533, #534, #535 plus the follow-up issues recorded in L6.

## Tasks

| ID | Task | Specs | Route | Commit |
| --- | --- | --- | --- | --- |
| T1 | Turn and step identity in lifecycle events (`TurnId`, `StepId`, additive schema, resume correlation) | S9, S5, S10 | inline | `babfb50` |
| T2 | Plugin-authorable loop registration: `huginn_plugin_api::v1::agent`, core-owned `agent:loop@1` id, ADR-0017 | S2, S10 | inline | `d6fd14f` |
| T3 | Loop-parametric conformance suite in a dedicated Huginn test-support crate, with a broken-loop fixture | S3, S4 | inline | (pending) |
| T4 | Second distinct loop strategy registered through the plugin path, passing the whole suite | S1, S3, S2 | inline | (pending) |
| T5 | Pre/post inference (turn-level) hooks with a block audit event | S7, S4 | inline | (pending) |
| T6 | Steering and queued messages: in-turn message injection with defined semantics | S6 | inline | (pending) |
| T7 | Budget enforcement: per-turn token and iteration budget from `Event::Usage`, fail-closed | S6, S4 | inline | (pending) |
| T8 | Context pinning: instructions and skill catalog loaded once per turn, with provenance | S8, S10 | inline | (pending) |
| T9 | Effect-aware retry classification for inference calls (idempotent vs effect-bearing) | S7, S4 | inline | (pending) |
| T10 | Certification matrix and loop contract gap list in `docs/huginn/agent-loop-conformance.md` | S3, S11 | inline | (pending) |
| T11 | Follow-up implementation issues for the remaining epic capabilities, created in GitHub | S11 | inline | (pending) |

## Log

- **L1 — original request (verbatim, DALLAY-720 / #444).** Title: "[HUGINN][EPIC 03] Replaceable Agent Loop & Execution Control". Purpose: "Provide a production-quality agent execution cycle whose concrete strategy can be replaced without changing the kernel." Scope: "* Inference→tool→inference turns and streamed lifecycle events
* Replaceable AgentLoop service; turn and step identity
* Interrupt, cancellation, steering, queued messages and budget enforcement
* Pre/post inference hooks, effect-aware retries and recovery
* Context pinning, concurrency/backpressure and replay-safe semantics". Architectural boundaries: "The host owns authorization and effects regardless of loop implementation. Avoid double invocation and automatically replaying effects with unknown outcomes." Epic-level acceptance outcome: "Two distinct loop implementations pass the same behavioral and security conformance suite, including cancel, denial, recovery and streaming." Dependencies: "EPIC 01; EPIC 11 security; existing AgentLoop interface". Planning policy: "**Umbrella epic only.** No implementation tasks, child issues, sprint dates or release promises are authorized by this issue. Before work begins: audit current `main`, open PRs, existing issues and ADRs; select an approved vertical slice; then refine and create implementation issues **in GitHub** so they synchronize to Linear. Preserve completed functionality; do not treat this scope list as an unimplemented backlog." The full body, including the attached Huginn capability catalog, lives at GitHub #444.
- **L2 — audit delivered (read-only).** Baseline `1f421ed`; `origin/main` 8 commits ahead, none touching `crates/huginn/{core,application}` loop code. Open PRs were Renovate and release-please only. Findings: the two registered adapters share one engine (`LeasedAgentLoop` wraps `LoopExecution`); no turn/step identity; no plugin-authorable loop; `Event::Usage` recorded and never enforced; instructions reloaded every iteration; steering and queued messages absent; no turn-level hooks; no conformance suite.
- **L3 — user authorization to refine.** L3 verbatim: "dale". Authorized creating the slice A issues in GitHub.
- **L4 — user expands scope to the whole epic.** Verbatim: "1. Si vamos con al final, el objetivo mío es resolver esta EPIC completa y tener la lista, tener todo lo que es el loop de la gente funcional, todo el loop de la gente funcional y por supuesto que sea como dicen en la misma descripción de la issue que va a ser un plugin del núcleo del kernel de nuestro sistema. 2. Sí, todas las inches de taller están autorizadas, esto lo vamos a sacar en nuestro sprint directamente nosotros." Reading: the complete epic is authorized, the loop must be kernel-plugin-provided, and the sprint is the maintainer's own, so epic sprint approval is granted and the umbrella restriction no longer blocks implementation.
- **L5 — base correction.** Local `main` held 11 unpushed commits plus a stale dirty tree whose `crates/huginn/presentation/{src/terminal.rs,tests/pty.rs}` revert merged work from `45b6c46` (keyboard protocol, multi-line composer growth, paste spans, 254 PTY test lines). Feature work therefore runs in a separate worktree `/home/ryuknull/workspaces/cortex-dallay-720` created from `origin/main` @ `1216d4e`, leaving the user's local state untouched.
- **L6 — issues created.** GitHub #533 (turn/step identity), #534 (loop-parametric conformance suite), #535 (certification matrix and gap list). Recorded on #444.
- **L7 — T1 delivered.** `babfb50`. `TurnId`/`StepId` in `huginn-core`, `#[serde(default)]` on the new event fields so pre-identity sessions load as unknown, `Session::allocate_turn` persisting the allocator, per-step `StepScope` in the loop, adapters and fixtures updated. Tests: `crates/huginn/application/tests/turn_identity.rs` (4 cases). Evidence: 67 test targets ok, clippy `-D warnings` clean. Slice boundary: this commit also carries the contract-owned `agent_loop_id()` because the pre-commit hook checks the staged tree, which must compile on its own.
- **L8 — T2 delivered.** `d6fd14f`. `huginn_plugin_api::v1::agent`, `composition::loop_id` delegating to `huginn_core::agent_loop_id`, ADR-0017. Tests: `crates/huginn/plugin-api/tests/agent_loop.rs` (authorable path) and `crates/huginn/application/tests/plugin_loop.rs` (substitution moves the provider to the plugin, one plugin run, zero model calls — S4 no double invocation).
- **L9 — delivery strategy for the remaining slices.** The schema change touched 12 test fixtures, so slice 1 exceeded the 400-line review budget. From T3 on, each task lands as its own commit under ~400 authored lines, and any task that grows past that is split by capability, not by file.
- **L10 — incident.** A scripted multi-file edit inserted `step:` fields into production code and deleted the `InstructionSource` trait; the `check` pre-commit hook caught it and the block was restored and re-verified. A scripted edit must be followed by reading `git diff` for the production files it touched. `gentle-ai-worker` also failed on its first turn with zero tool calls (provider error); it was not retried and the work continued inline.
