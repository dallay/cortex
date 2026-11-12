# Design: Huginn overlay system

## Technical approach

A `Option<Overlay>` slot governs focus, input and render. The implementation of
forms and services is preserved, it is not replaced by empty labels.
The projection separates conversation, overlays, notifications, status and diagnostics.
The router publishes local selection observability, not remote availability.
Scope and precedence: the approved proposal and the four delta specs of this change.

Verified reading: `state.rs`, `terminal.rs`, `workflow.rs`, `settings.rs`,
`providers.rs`, `contributions.rs` of presentation; core/plugin-api contracts;
application connections/router; app wiring and related manifests/tests.
This phase runs no compilation, tests nor QA and does not certify previous gates.

## Architecture decisions

### D1. Deepen the overlay module, do not add handlers with access to Ui

**Choice:** `overlay.rs` concentrates the transition, the snapshot and pure input; it returns
effects that `Ui` executes outside the mutex. Remove the four incompatible slots and leases
when finished. `Overlay` does not receive `&Ui` in its mutable methods. **Alternative:** a handler trait that locks `Ui` again or keep slots
with priorities. **Reason:** it avoids reentrancy under lock and concentrates the focus
guarantees in a testable module. Neither the kernel nor the turn router is restructured.

### D2. The surface tag is neither payload nor authority

**Choice:** add `CommandResult::Overlay(OverlayKind)`; `OverlayKind::Approval` never exists.
Keep `Workflow(Workflow)` and `Dialog(String)`.
Add surface metadata to the workflow, preserving its definition, callback,
exact registration, fields and secrets. Do not rebuild a workflow by title,
prefix, command name nor by lookup of a substitute generation. **Alternative:** return only `OverlayKind::Login` instead of the current workflow. **Reason:** that substitution loses callbacks, context and asynchronous continuation.

### D3. A single visible destination per UI error

**Choice:** explicit category before dedup; form errors in their
body and other errors in the notice zone. No toasts nor a universal `notice`.
`CommandResult::Notice(View)` remains a compatible plugin result,
but it means Notification exclusively, never conversation. **Alternative:** classify text or deduplicate history. **Reason:** it would hide
legitimate messages and allow history contamination when a window expires.

### D4. Versioned source at the router serialization point

**Choice:** the connections service publishes a complete versioned snapshot
under its selection mutex; `ConnectionManager` forwards it and the bar consumes that
channel. `UiState` does not query the remote state. **Alternative:** periodic queries
or text emitted by commands. **Reason:** it orders selection, absence and revocation
without asserting connectivity, and it allows reflecting changes from every path.

### D5. Declarative views without execution authority

**Choice:** the `UiView`/snapshot projection includes explicit categories,
`OverlaySnapshot` and the observable state of the bar; it does not serialize capabilities,
handlers, callbacks nor secrets. The active surface is presented as an overlay,
not as conversational content. **Alternative:** emit notices and rebuild the
view from text. **Reason:** it keeps a verifiable boundary between interaction,
conversation and observability data.

### D6. Errors deduplicated by content, operation and time

**Choice:** apply dedup only to already classified presentation errors, with
hash and message comparison, injectable clock, operation and approved window. **Alternative:** global or prefix based dedup. **Reason:** it avoids suppressing
independent failures or altering the history; the exact policy remains defined by
the existing specs.

### D7. UI keeps bounded events, session keeps the full conversation

**Choice:** limit the UI event projection buffer to 256 entries; the
persistence of `Session.events` and the relevant conversation messages are not
truncated. **Alternative:** apply the limit to the durable record. **Reason:** the
limit controls UI memory without changing session recovery.

### D8. Progressive state integration, without dual authority

**Choice:** migrate first the ownership of the active state to the host-owned `UiState`;
during the transition, legacy objects can be adapters/exact payloads
inside `Overlay`, but their slots stop deciding visibility, focus or dispatch.
Snapshot/render and terminal input consult that same state. **Alternative:**
keep four slots and synchronize them with `ActiveOverlay<T>`. **Reason:** synchronizing
copies does not allow guaranteeing order nor a single lease; an isolated machine that does not drive
input/render does not complete OVL-01 either. This decision refines D1: removing independent
slots implies migrating them through adapters, not discarding their flows.

### D9. Precedence without a stack and with separate authorization

**Choice:** `Approval` is only created by the host authority, it preempts and discards
the ordinary overlay without restoring it, it cancels its local task if applicable; an ordinary opening
during Approval is rejected. There is no stack. **Alternative:** priority with
restoration or allowing plugins to replace Approval. **Reason:** it preserves
permission exclusivity and avoids resurrecting revoked state/focus.

## Data Flow

```text
CommandResult / async event
        │ validate origin + ticket + operation + revision
        ▼
UiState.active_overlay ──→ snapshot/render
        ▲                         │
        └──── terminal input ─────┘
                   │ current lease/revision
                   ▼
             transition + effects
                   │ unlock before executing
                   └── I/O / callbacks / cancellation / Drop
```

The snapshot is a safe projection, not a serialization of the View payload.
It must hide secret values and form buffers, senders/receivers, callbacks,
resource handles, closures, renderer/service references and cancellation
tokens. The host authority keeps those elements until the operation completes or is invalidated;
they do not reach JSON events, logs, diagnostics or conversation.

## Integration review (2026-10-19)

### State and adapters

The active authority lives in a single host-owned field inside `UiState`. The
machine keeps the overlay, the exact origin, the operation, revision/ticket and `FocusLease`.
There is no parallel active state in `modal`, `plugin_dialog`, `workflow` nor
`settings`. `FocusLease` and revision advance with `checked_add`; exhaustion fails
closed, it never wraps nor reuses an identity. Every terminal event is
routed to the active state and carries the lease/revision captured at the start. The
application of a key validates both against the current state; if there was a close,
replacement or preemption, it is discarded. Esc consumed to close is not reinserted.
Paste keeps a fresh epoch per lease; fragments of a revoked epoch are discarded.

| Current origin                         | Adaptation inside Overlay                                                                                                                   | Host retention / close                                                                                        |
|-----------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------|
| Modal/Approval                          | exact Approval payload, own `oneshot` and epoch; never a plugin requested `OverlayKind`                                                       | Only the host creates/resolves; close/shutdown invalidates the epoch and resolves as cancelled, with no concession                    |
| `plugin_dialog` / `DialogHandle`        | `Dialog(String)` keeps exactly handle, registration and associated renderer/callback; typed surfaces use their own variant and payload   | Revocation cancels local work; no substitute contribution is looked up by name                                |
| `workflow` / `HostWorkflow` / `Pending` | `Workflow` keeps the registered definition, fields, callback, continuation and context; an optional surface does not replace the workflow                | Pending/cancel/abort stay under the host; a receipt applies only if the origin ticket and operation are still current |
| `SettingsDialog`                        | adapts to `Overlay::Settings` with an exact editable copy, without a second authority                                                           | persistence uses the captured snapshot; a late receipt does not close/edit the current overlay nor affect Approval          |
| Widgets                                 | widgets that represent modal interaction are integrated as variant payload; purely informative widgets stay in their `UiView` region | no informative widget owns focus/active overlay state                                                     |

The typed definition can be `Overlay { kind, payload }` with payloads that
keep their legacy types during this migration; `OverlaySnapshot` only exposes
kind, non-secret text/fields and the presentation state needed for render.
Do not copy handlers to the snapshot nor turn callbacks into strings.

### Transactions and external effects

Under the lock, the authority/origin/revision is validated and an atomic transition is performed:
the previous overlay is removed, the new one is installed if the policy allows it,
revision and lease are incremented, the previous ticket is invalidated and the resources
that must end are extracted. No renderer, callback, I/O, cancel/abort nor destructor
is called under the lock. After releasing it, effects are processed deterministically: first
the logical invalidation already published, then local cancellation/abort and the destruction of the
removed payload; then callbacks/renderers or I/O associated with the new operation.
Every reentrant callback revalidates origin/ticket/revision when it tries to
publish a result; it does not keep an implicit right to reopen. Approval preempts
atomically, extracts the ordinary overlay, invalidates its lease/ticket and cancels local
work outside the lock. An ordinary opening during Approval does not mutate state. When closing a workflow,
the ticket is invalidated and the local execution is cancelled; a late completion is consumed and
discarded without reopening nor mutating a later overlay.

Async results apply only if the expected ticket, exact origin, operation and
revision match. The identity includes the original registration/handle, not the plugin
name. A Settings Apply that started can end according to SET-07: a current success
updates the persisted snapshot and closes only the still current Settings/Apply; a stale success
can update only the saved snapshot, and stale failures do not
alter the visible state nor the snapshot. None of that restores focus or displaces Approval.

### Proposed vertical acceptance for 2.2

Host tests, not only unit tests of the isolated machine, must demonstrate at minimum:

1. `CommandResult::Overlay(Help)` opens a real view and not a notification, and the
   open/close adds no conversation entries.
2. Full Settings (editing, pending Apply, success/failure) keeps its real integrated
   flow and stale receipts do not modify the current overlay.
3. Snapshot and render, as well as terminal input, reflect the same `UiState`; the input
   requires a current lease/revision, and a closing Esc never reaches the editor.
4. Stale ticket, closed workflow, approval preemption and late completion are
   discarded; a host-only approval cannot be replaced by plugin requests.
5. The workflow keeps its exact continuation/callback/registration; local cancellation
   happens on close/revocation and does not declare a durable/remote rollback.
6. A reentrant renderer/callback and a Pending/payload destructor that queries Ui do not
   lock nor deadlock: they are invoked outside the mutex.
7. More than 256 UI events do not truncate the persisted session nor make a UI
   interaction reappear in conversation, even after the dedup expires.

These are verifiable criteria derived from OVL-01/02/03/04/05, SET-07/12 and
EVT-01/02; they do not add product capabilities.

### Dependency-ordered cuts and the task gap

The current task 2.2 combines the machine and the snapshots but requires no vertical integration;
2.3–2.5 migrate flow/render/input afterwards, which leaves the state without a single owner
and makes accepting OVL-01 impossible during the interval. It is proposed to keep
the numbering and the global authorization, but order the review packages as follows:

1. **2.2a, integrated host authority:** `UiState` takes ownership of `ActiveOverlay`,
   transitions, tickets/leases and the safe snapshot; connect the result dispatch
   and the basic open/close of all surfaces without removing the payloads.
2. **2.2b, adapters and lifecycle:** migrate Dialog/Handle, HostWorkflow/Pending,
   Approval/oneshot+epoch, SettingsDialog and widgets to the payload; extract the effects and
   close/revoke/cancel outside the lock, keeping exact callbacks and receipts.
3. **2.2c, vertical input/render and acceptance:** terminal and renderer read/write
   through the shared authority; remove the legacy active ownership; run the host
   tests of the previous list before advancing.
4. The existing tasks 2.3–2.5 contain part of those same migrations (workflow/dialog, Approval and input/paste). The tasks owner must rebaseline
   them as remaining hardening/coverage after 2.2c, removing duplication and
   keeping their contracts and gates; they cannot be a precondition to integrate the
   authority in 2.2. 2.6 keeps the real Provider flows, but depends on the
   authority/adapters of 2.2c and on the required input hardening, not on
   introducing a second owner. 3.1 adapts Settings visually/functionally over that same
   authority; it does not reintroduce `SettingsDialog` as a slot.

The 2.2a–c subcuts and the re-baseline of dependencies/tasks are proposals for the
owner of that artifact; here no task is marked as complete nor `tasks.md`, `state.yaml` or progress is changed. 2.2 must remain blocked for global apply
until the orchestrator updates the breakdown and approves the order/dependencies.
It does not imply a toast layer: notices and diagnostics stay in the specified
non-conversational surfaces.

## Interfaces / Contracts

```rust
enum OverlayKind { Settings, Provider, Login, ModelPicker, Help, Confirm }
// Approval originates exclusively in the host and is not in OverlayKind.
struct OverlayTicket { operation: OperationId, revision: Revision }
struct FocusLease { id: FocusId, paste_epoch: PasteEpoch }
struct OverlaySnapshot { kind: OverlayKind, revision: Revision, view: SafeOverlayView }
```

`open/close` validate origin and ticket; they return a transition and typed effects,
they do not execute external effects. An `Overlay` can temporarily host the legacy typed
payload, but `UiState` is its only active owner. The snapshot omits
callbacks, senders, resources and secrets. No interface gives plugins an
input/render capability nor Approval resolution.

## Test strategy

| Layer              | What to test                                                                                                   | Approach                                                           |
|-------------------|--------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------|
| Unit             | authorization, ticket/origin/revision, monotonicity, lease/paste, transitions and redacted snapshot               | controlled clock/channels, overflow fails closed                 |
| Host integration | real opening, settings, workflows, preemption, cancellation, reentrant callbacks/Drop and late results | `UiState`/terminal/render harness, barriers to order races |
| Contracts        | declarative commands, unreachable Approval, exact registration                                             | plugin-api tests and host dispatch                                  |
| Persistence/PTY | categories outside conversation, close without a reinjected key, >256 without truncating the session                      | existing projection/session and PTY tests                         |
| Selection/bar    | versioned local snapshot and connection events                                                              | application/core unit tests and the UI consumer                     |

## Migration / Deployment

It requires no data migration nor feature flag. It is a change of compiled contracts
that must integrate consumers and fixtures in the same authorized delivery.
The integration is done by dependency-ordered review packages, without claiming
that future phases are complete.

## Open questions

- [ ] The technical snapshot contract admits the suggested `SafeOverlayView`; specify
  the non-secret fields against the real types at implementation time, without widening the visible data.
- [ ] The tasks owner must decide how to formalize 2.2a–c and the new dependencies
  with respect to 2.3–2.6 before resuming the global apply.
- [ ] Verify during implementation whether the existing notice destination covers
  all form errors without an additional surface (no toast layer).
