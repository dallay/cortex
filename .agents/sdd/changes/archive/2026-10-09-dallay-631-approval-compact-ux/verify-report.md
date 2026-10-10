# Verification Report

> **Naming provenance (2026-10-10):** See ADR-0011. Crate/binary names
> in this report (`apps/agent`, `cortex-agent`, `agent-core`,
> `agent-runtime`) are historical; live code uses
> `apps/huginn`/`huginn`/`huginn-core`/`huginn-runtime`.

**Change**: dallay-631-approval-compact-ux **Version**: 1.0

---

## Completeness

| Metric           | Value                                                              |
|------------------|--------------------------------------------------------------------|
| Tasks total      | 23 across 5 phases (see tasks.md)                                  |
| Tasks complete   | 23 — Phase 1 (terminal counter/header/effect_line/reset), Phase 2 (`StandardLoop::compact`), Phase 3 (`/compact` & `/summarize` interception), Phase 4 (tests), Phase 5 (docs) |
| Tasks incomplete | None                                                               |

---

## Build & Tests Execution

**`cargo fmt --check`**: ✅ Passed after `cargo fmt`.

**`cargo clippy --workspace --all-targets -- -D warnings`**: ✅ Passed. One pre-existing
`dead_code` warning on `Policy::turn_count` is annotated `#[allow(dead_code)]`
because the helper is part of the public surface for tests and future UI even
though the binary does not use it directly.

**`cargo test -p agent-runtime --test transports`**: ✅ 6 passed / 0 failed.

**`cargo test -p agent-runtime --test coding_workflow`**: ✅ 27 passed / 0 failed.

**`cargo test -p cortex-agent --test cli`**: the original change added one
one-shot `run` forwarding test. The later review-fix iteration adds CLI coverage
for non-interactive chat rejection; accepted/declined interactive confirmation
is not exercised by this report. After the second review-fix cycle the two
interactive PTY tests (`chat_compact_emits_ndjson_event_after_confirmation`
and `chat_compact_decline_emits_no_compacted_event`) are driven through a Rust
PTY harness built on `portable-pty`, so they run on every CI job without
relying on the `expect` binary.

**`cargo test -p cortex-agent` (unit tests inside `apps/agent`)**:
✅ 6 passed / 0 failed (the six `effect_line_for_*` cases added to
`apps/agent/src/terminal.rs`). After the second review the binary-side test
suite covers nine cases: the six `effect_line_for_*` cases, the
`approval_header_numbers_repeated_identical_requests_without_claiming_difference`
header test, the new `safe_strips_ansi_sequences_from_effect_line` sanitization
test, and `policy_approval_counter_increments_within_a_turn_and_resets` for the
per-turn counter lifecycle.

**`pnpm exec markdownlint-cli2 docs/agent/validation.md docs/agent/implementation-specification.md docs/agent/linux-daily-use.md`**: ✅ 0 issues.

**`git diff --check`**: ✅ Passed (no trailing whitespace, no mixed indent).

---

## Spec Compliance Matrix

| Requirement                        | Scenario                                                                 | Test                                                                    | Result       |
|------------------------------------|--------------------------------------------------------------------------|-------------------------------------------------------------------------|--------------|
| R1 numbered approval prompts | Scenario 1 — single approval | `approval_header_numbers_repeated_identical_requests_without_claiming_difference` | ✅ COMPLIANT |
| R1 numbered approval prompts | Scenario 2 — repeated approval | `policy_approval_counter_increments_within_a_turn_and_resets` | ✅ COMPLIANT |
| R2 per-turn counter lifecycle | Counter resets at the start of each turn | `policy_approval_counter_increments_within_a_turn_and_resets` plus `policy.reset_turn()` call sites in `main.rs` | ✅ COMPLIANT |
| R3 `Effect:` line derivation | Shell, edit, create, MCP start, MCP call | `effect_line_for_*` unit tests | ✅ COMPLIANT |
| R4 `/compact` command in chat | Accepted / declined | `chat_compact_emits_ndjson_event_after_confirmation` and `chat_compact_decline_emits_no_compacted_event` drive the chat subcommand through a Rust PTY harness (`portable-pty`) and assert both branches. Real terminal with a real model remains a separate manual step recorded in `docs/agent/validation.md`. | ✅ COMPLIANT |
| R5 shared summarization | Manual and automatic compaction share summary helper | Runtime compaction + automatic flow tests | ✅ COMPLIANT |
| R6 approval state on denial | Existing denied-edit and denied-shell tests | Existing `coding_workflow` regression cases | ✅ COMPLIANT |
| R7 backwards compatibility | Additive trait method with default unsupported result | Existing implementors compile in workspace checks | ✅ COMPLIANT |

**Compliance summary**: After the second review cycle the report demonstrates
approval numbering, the per-turn counter lifecycle, the `/compact` accept/decline
chat branches, the sanitization of the `Effect:` line, and the existing runtime
turn selection and cancellation cases. The remaining manual step is the Ctrl+C
cancellation test with a real model, recorded in `docs/agent/validation.md`.

---

## Correctness (Static — Structural Evidence)

| Requirement                                | Status         | Notes                                                                                                       |
|--------------------------------------------|----------------|--------------------------------------------------------------------------------------------------------------|
| R1 numbered header                          | ✅ Implemented | `Policy::approve` increments `turn_count` and renders `(request #N this turn)` without unverified difference claims |
| R2 per-turn reset                           | ✅ Implemented | `apps/agent/src/main.rs` calls `policy.reset_turn()` in `Run` and at the top of each `chat` iteration        |
| R3 `Effect:` derivation                     | ✅ Implemented | `effect_line()` helper in `apps/agent/src/terminal.rs`; six unit tests cover prefixes                       |
| R4 `/compact` command                       | ✅ Implemented | `chat` loop intercepts `/compact` and `/summarize`; `run_compact` reads confirmation, calls `StandardLoop::compact` |
| R5 manual summarization                     | ✅ Implemented | `crates/agent/runtime/src/loop_engine.rs` adds `pub async fn compact` mirroring the summary branch          |
| R6 denial integrity                         | ✅ Implemented | No change to the loop; existing denied-edit/denied-shell tests continue to pass                              |
| R7 backwards compatibility                  | ✅ Implemented | No public API change in `agent-core` or `agent-runtime`; new method is additive                              |

---

## Coherence (Design)

| Decision                                                | Followed?   | Notes                                                                                          |
|---------------------------------------------------------|-------------|-------------------------------------------------------------------------------------------------|
| Header shape: `Approval for <action> (request #N this turn):` | ✅ Yes | Single `Action:` line and a conditional `Effect:` line above the preview; prompt reworded     |
| Counter held in `Mutex<u64>`                              | ✅ Yes       | Brief critical section, no async lock needed                                                   |
| `/compact` intercept before `run(...)`                     | ✅ Yes       | `chat` only; `Run` forwards the literal as a regular prompt                                     |
| `StandardLoop::compact` reuses the summary branch prompt | ✅ Yes       | String literal lives once; both call sites use the same prompt                                  |
| Approval previews and headers reworded without changing `ApprovalRequest` | ✅ Yes | `ApprovalRequest` and `ApprovalPolicy` are unchanged                                          |

---

## Issues Found

**CRITICAL** (must fix before archive):

- None.

**WARNING** (should fix):

- **Interactive acceptance with a real model is still deferred.** The
  repeat-approval and Ctrl+C-during-`/compact` scenarios require a real
  terminal with a model available; they were not exercised in this CI run. The
  2026-10-09 local Ollama record documents an earlier accepted-edit /
  denied-write / `/compact` flow and is available in `docs/agent/validation.md`.
  The pending manual step is a Ctrl+C during a live `/compact` summary.

**SUGGESTION** (nice to have):

- Consider a turn-end approval summary line (`Approvals: #2 (1 denied, 1 approved)`)
  once the turn completes; deferred.
- Consider exposing `/compact --dry-run`; deferred.
- `complete_turn_ends` in `crates/agent/runtime/src/loop_engine.rs` walks
  `messages[from..]` for every assistant message and is O(n²) in the worst
  case. A single pass would be cheaper; deferred because the reviewer flagged
  it as an observation, not a blocker. Tracked as
  [issue #310](https://github.com/dallay/cortex/issues/310).

## Implementation drift from archived spec/design

The archived `spec.md` and `design.md` describe the original approach: capture
a parallel `Arc<StandardLoop>` in `main.rs` alongside `LoopService` so the
binary can call `compact()` on the concrete type. The first review cycle
(commit `7a4bec3`) replaced that with a capability method on the
`AgentLoop` trait; the binary now resolves `LoopService` from the supervisor
and calls `AgentLoop::compact` through the trait object, so no parallel
handle exists. The archived `spec.md` (`§Composition and
`AgentLoop::compact`) and `design.md` (the `loop_arc` design notes) still
describe the superseded approach and remain as the historical record of
what was proposed; this verify report is the authoritative "as implemented"
view. No code or runtime behavior is affected.

---

## Verdict

PASS WITH WARNINGS — historical report, corrected during PR #305 review fixes
and refreshed after the second review cycle.

This report records the original implementation verification, not a claim that
the missing interactive acceptance scenarios were tested. In particular, the
previous report text incorrectly referred to a non-existent CLI test and
planned tests as though they were evidence. Later review-fix verification is
recorded in `.agents/rpi/plan/tasks/dallay-631-pr305-review-fixes.md`,
`.agents/rpi/plan/tasks/dallay-631-pr305-second-review-fixes.md`, and
`docs/agent/validation.md`; local TUI plus real-model acceptance remains open
for the Ctrl+C cancellation case.
