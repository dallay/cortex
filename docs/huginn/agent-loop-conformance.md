# Agent loop conformance

A loop is replaceable only if "a loop" means something checkable. This document states
the clauses an implementation must satisfy, the conformance case that proves each one, and
the clauses that cannot be proven through `AgentLoop` today.

Evidence: `crates/huginn/conformance` (suite) and `crates/huginn/conformance/tests/loops.rs`
(certification). Run it with:

```sh
cargo test -p huginn-conformance
```

A run certifies three implementations: `StandardLoop`, the production `LeasedAgentLoop`, and
`StepLoop`, authored in `crates/huginn-reference-extension` against the plugin contract only.
`StandardLoop` pins its providers and is therefore skipped for `C18`; `LeasedAgentLoop` and
`StepLoop` lease the kernel's services and are certified for it. `broken::ReplayingLoop` and
`broken::PinnedProviderLoop` must fail; a suite that cannot fail certifies nothing.

## Certified clauses

| # | Clause | Case | Enforced by |
| --- | --- | --- | --- |
| C1 | A turn opens with `TurnStarted` carrying its `TurnId` and closes with the matching `TurnFinished` | `streaming_lifecycle_order` | loop |
| C2 | Every step-scoped event carries the `StepId` of the model call that produced it | `streaming_lifecycle_order` | loop |
| C3 | A cancelled turn fails as `Cancelled`, reaches no model, records no completion, and stays `interrupted` | `cancel_before_model_stream` | loop |
| C4 | Cancellation interrupts a stream that is already open | `cancel_inside_model_stream` | loop |
| C5 | A cancelled turn never starts another effect | `cancel_between_tool_calls` | loop |
| C6 | A denial produces no effect, asks the host exactly once, and is reported to the model as that call's result | `denial_performs_no_effect` | loop + host `ApprovalPolicy` |
| C7 | An effect whose outcome is unknown is never replayed; the pairing is closed with an explicit unknown-outcome result | `effect_unknown_is_not_replayed` | loop |
| C8 | The iteration limit stops the turn with a diagnostic instead of looping | `iteration_limit_is_enforced` | loop |
| C9 | A blocked tool call produces no preview, no approval prompt, and no effect | `hook_block_prevents_effect` | loop + `agent:tool-hooks@1` |
| C10 | Duplicate or malformed tool-call ids are rejected before any effect | `duplicate_tool_calls_are_rejected` | loop |
| C11 | A turn-level hook that blocks costs no model call and leaves an auditable event | `inference_hook_blocks_the_call` | loop + `agent:inference-hooks@1` |
| C12 | Host steering reaches the model at the next step boundary and is recorded | `steering_reaches_the_next_step` | loop |
| C13 | The turn token budget stops the turn, and the over-budget step takes no effect | `turn_budget_stops_the_turn` | loop |
| C14 | Repository instructions are read once per turn; the system context cannot drift mid-turn | `context_is_pinned_for_the_turn` | loop |
| C15 | A transient inference failure may be repeated; an effect never is | `retry_repeats_inference_only` | loop |
| C16 | Compaction stops at a complete-turn boundary and never splits a tool pair | `compaction_keeps_turn_boundary` | loop |
| C17 | One in-process operation per session; overlapping turns and supported compaction reject before effects, without queueing | `concurrent_session_admission_is_bounded` | loop + shared `SessionOperation` |
| C18 | A loop that leases `agent:model` serves the next turn from the substituted provider; one that pins it never does | `substitution_rebinds_the_model` | loop + `Capabilities::rebinds_services` |

The host owns authorization in every clause: a loop may *ask* `ApprovalPolicy`, and no clause
lets a loop grant itself a decision.

## Single-process admission and backpressure

Each exact persisted `Session::id` has capacity **N=1**, shared across loop
instances and independently cloned or loaded snapshots in the process. A turn or
supported manual compaction acquires `SessionOperation::try_enter` before any
host access, including model pinning, provider leases, instruction or skill
loading, turn allocation, events, persistence, previews, approval, and tools.
Internal automatic compaction remains part of the admitted turn.

There is **no waiting queue**. The N+1th operation, and every further overlapping
request, immediately returns `AgentError::Session` with this diagnostic:

```text
session already has an active in-process operation (capacity 1; no waiting queue)
```

Rejection changes neither the submitted snapshot nor stored data, emits no event,
and performs no host effect. The active operation is not cancelled by rejection.
Its permit lasts through final persistence and releases on success, error,
cancellation, dropped future, or unwind. Poisoned admission state fails closed.
Different session IDs remain independent; no aggregate process-wide limit is
introduced.

C17 holds a turn open at a deterministic model boundary and submits a burst through
a separately built loop, checking immediate diagnostics, unchanged state and
counters, no file effect, compaction exclusion, independent-session progress, and
readmission after cancellation. An unguarded loop fails this case.

Admission does not merge snapshots: after an active operation finishes, reuse its
updated session or reload before retrying. Later use of a stale clone is not
protected. IDs are compared exactly as persistence addresses them; this rule adds
no UUID spelling normalization. The cross-process lease protocol is unchanged.
Trusted native loop authors must adopt the permit; the trait does not sandbox
implementations, and the conformance suite certifies this behavioral obligation.

Two clauses are capability-gated, and they differ from a pass: `C16` is skipped by a loop that
does not declare `Capabilities::compaction`, and `C18` is skipped by a loop that does not declare
`Capabilities::rebinds_services`. A skipped clause is not a certification: only the declared case
is proven. Declaring `rebinds_services` without publishing the loop into a supervisor fails `C18`
rather than skipping it, so the capability cannot be claimed without being exercised.

## Gap list

These are contract gaps, not loop defects. Each one is why an external implementation has to
copy host code or cannot express a guarantee at all.

| # | Gap | Evidence | Consequence |
| --- | --- | --- | --- |
| G1 | The complete-turn boundary rule is not part of the contract | The rule is implemented twice, in `huginn_application::loop_engine::complete_turn_ends` and in `StepLoop::complete_turn_end` | Every implementation must reimplement compaction boundaries and can silently disagree |
| G4 | Transient transport failures are not classified by the adapters | Only a truncated stream yields `AgentError::Inference`; HTTP failures still surface as `Model` | The retry budget added here never engages for a real provider |
| G5 | Capabilities are declared out of band | The suite needs `Capabilities { compaction }` because `AgentLoop::compact` has a default `Err` | Nothing in the trait says what an implementation supports |
| G6 | `Event::Error` carries no turn identity | Kept as-is for schema compatibility | A consumer must infer the turn positionally |
| G7 | Steering has no producer in the terminal | `LoopConfig::steering` is honored, the TUI composer still drops a draft while busy | The capability exists but no user can reach it |
| G9 | Tool-output bounding is not part of the contract | `StepLoop` had to define its own `MAX_OUTPUT_BYTES`; the host limit lives in `huginn_application::output`, unexported | Two loops can persist wildly different history sizes for the same tool |

G1 and G4 are the ones that would change the contract; the rest are either additive or
adoption work. They are tracked as follow-up issues rather than folded into this change.

G2 and G3 are closed. The hook-registry rule is one rule with one owner: the kernel owns
`agent:tool-hooks@1` and `agent:inference-hooks@1`, refuses a loop that supplies a foreign
registry, and says so in `v1::agent`. Rebinding is authorable rather than host-only:
`v1::agent` exports `ServiceSlotHandle`, `ServiceId`, `ModelService`, `SessionsService`,
`model_id()` and `sessions_id()`, and `C18` proves a substituted provider is the one the next
turn uses. The reference extension exercises both from the SDK alone.
