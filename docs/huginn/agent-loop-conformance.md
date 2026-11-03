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
`broken::ReplayingLoop` must fail; a suite that cannot fail certifies nothing.

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

The host owns authorization in every clause: a loop may *ask* `ApprovalPolicy`, and no clause
lets a loop grant itself a decision.

## Gap list

These are contract gaps, not loop defects. Each one is why an external implementation has to
copy host code or cannot express a guarantee at all.

| # | Gap | Evidence | Consequence |
| --- | --- | --- | --- |
| G1 | The complete-turn boundary rule is not part of the contract | The rule is implemented twice, in `huginn_application::loop_engine::complete_turn_ends` and in `StepLoop::complete_turn_end` | Every implementation must reimplement compaction boundaries and can silently disagree |
| G2 | A supervisor-based loop cannot choose its hook registries | `LoopPlugin::activate` overwrites `LoopConfig::hooks` and `inference_hooks` with the kernel's own services; the conformance harness had to hand the same hook through two paths | An author cannot predict which registry a loop will see |
| G3 | Lease-aware rebinding is not authorable | `ServiceSlotHandle`, `ModelService` and `SessionsService` are not exported by `v1::agent` | An external loop pins its providers with `Arc`, so a mid-process substitution leaves it on the retired ones |
| G4 | Transient transport failures are not classified by the adapters | Only a truncated stream yields `AgentError::Inference`; HTTP failures still surface as `Model` | The retry budget added here never engages for a real provider |
| G5 | Capabilities are declared out of band | The suite needs `Capabilities { compaction }` because `AgentLoop::compact` has a default `Err` | Nothing in the trait says what an implementation supports |
| G6 | `Event::Error` carries no turn identity | Kept as-is for schema compatibility | A consumer must infer the turn positionally |
| G7 | Steering has no producer in the terminal | `LoopConfig::steering` is honored, the TUI composer still drops a draft while busy | The capability exists but no user can reach it |
| G8 | Concurrency and backpressure are unspecified | Only cross-process session leases exist | Two turns on one session in one process have no defined outcome |

G1, G2 and G4 are the ones that would change the contract; the rest are either additive or
adoption work. They are tracked as follow-up issues rather than folded into this change.
