# huginn-overlays — Specification

## Purpose

Unify the TUI interactions without expanding plugin authority or losing the
existing flows. It does not include onboarding, the toast layer, or the live
theme.

## Requirements

### Requirement: OVL-01 — Single active overlay

The system MUST represent the overlays through a unified `Overlay` enum that
includes `Settings`, `Provider`, `Login`, `ModelPicker`, `Help`, `Confirm`
and `Approval`, and hosts the existing dialogs/workflows. Render and input MUST
refer to the same active overlay. An overlay MUST consume its keys without
sending them to the editor or to completion. Closing MUST return the focus to
the editor without resending the closing key. The migration MUST finish without
parallel legacy slots `modal`, `plugin_dialog`, `workflow` or `settings`.

#### Scenario: OVL-01-A — Help and return to the editor

- GIVEN the editor is available and there is no overlay
- WHEN `/help` runs and then `Help` is closed
- THEN the help is presented in `Help`, not in the conversation
- AND during its opening the keys belong to `Help`
- AND on closing the editor is recovered without inserting or sending the closing key

### Requirement: OVL-02 — Authorized declarative requests

`CommandResult::Overlay(OverlayKind)` MUST allow commands to request authorized
host overlays without granting input, render, or permission capabilities.
Plugins MUST NOT create or resolve `Approval`, obtain its `oneshot`, or use
`Confirm` as tool approval. Unauthorized, invalid, or revoked requests MUST be
rejected without effects or an alternative opening.

#### Scenario: OVL-02-A — Allowed request

- GIVEN a current command authorized to request `Help`
- WHEN it returns `CommandResult::Overlay(OverlayKind)` for `Help`
- THEN the host opens `Help` with its own controls and focus
- AND no conversation is added and no authorization is granted

#### Scenario: OVL-02-B — Without approval authority

- GIVEN a plugin that requests a disallowed overlay or shows `Confirm`
- WHEN the request is processed or that ordinary confirmation is accepted
- THEN no pending action obtains tool approval
- AND the disallowed request leaves a non-conversational error and does not replace the active overlay

### Requirement: OVL-03 — Exclusive Approval and fail-closed

Only the host MUST open `Approval` and own its `oneshot` response, separate
from `Confirm`. `Approval` MUST preempt any ordinary overlay, discard it,
and revoke its focus without restoring it afterwards. While it is pending,
ordinary requests MUST NOT replace it. Only a valid explicit acceptance of the
current request MUST grant permission. Closing, cancellation, channel loss,
revocation, and shutdown MUST end without granting it; stale callbacks MUST
NOT resolve a later request nor cause double responses. Preempting Settings
before starting an Apply MUST NOT write; if the Apply already started a write,
the preemption MUST NOT start another one nor promise a rollback. Its receipt
MUST still follow REQ-07 of `ui-settings` without reopening Settings nor
altering Approval.

#### Scenario: OVL-03-A — Preemption and current acceptance

- GIVEN `Settings` with unsaved changes, no Apply started, and a host approval request
- WHEN `Approval` opens and the user completes its valid explicit acceptance
- THEN `Settings` is discarded without persisting and its focus is revoked
- AND only the current request receives an acceptance, a single time
- AND `Settings` does not reappear when it ends

#### Scenario: OVL-03-B — Closing and shutdown

- GIVEN a pending `Approval`
- WHEN closing, cancellation, channel loss, or shutdown occurs, in separate cases
- THEN the wait ends without permission and without remaining blocked
- AND later input or callbacks grant no permission and do not reopen the overlay

#### Scenario: OVL-03-C — Blocked ordinary replacement and late response

- GIVEN a current `Approval` and a late response from an earlier request
- WHEN that response arrives and a command requests `Help`
- THEN `Approval` stays active and unresolved
- AND neither the earlier callback nor the command alters its authority or focus

### Requirement: OVL-04 — Revocation, secrets, and leases

Closing, replacing, canceling, or revoking an overlay MUST invalidate its
input lease, discard its editable buffers, and cancel its local task when it is
cancelable. A Settings write already started MAY complete: only what is needed
to consume its receipt according to REQ-07 of `ui-settings` MUST be kept,
without restoring focus nor retaining form secrets. Closing MUST NOT start an
additional write nor assert that the started one was undone. The system MUST NOT
look for a substitute contribution by name to continue a revoked action.
Secrets MUST stay in secret fields with constant masking and MUST NOT pass
through the conversational editor, history, JSON events, logs, or diagnostics.
Pastes and keys captured by a revoked lease MUST NOT cross to the next overlay
or to the editor. Canceling locally MUST NOT assert a remote or durable
rollback.

#### Scenario: OVL-04-A — Incomplete paste and focus change

- GIVEN `Login` with a sentinel secret and an incomplete paste under its lease
- WHEN it is canceled with Ctrl+C and another overlay opens before the rest of the paste arrives
- THEN no fragment reaches the new overlay nor the editor
- AND the secret does not appear in the conversation, terminal captures, JSON, or logs
- AND Esc inside the incomplete paste is still treated as a delimiter, not as an acceptance

#### Scenario: OVL-04-B — Contribution revoked during operation

- GIVEN a pending form from a registered contribution
- WHEN it is revoked and another one of the same name is registered, and then the earlier result arrives
- THEN the form is closed, the focus is revoked, and its local task is canceled
- AND the substitute is not executed and the late result is not applied to the new UI
- AND any cancellation notice does not promise to undo effects already performed

### Requirement: OVL-05 — Functional connection flows

`Provider`, `Login`, `ModelPicker` and the confirmations MUST preserve the
existing operations of `/providers`, `/login`, `/model` and `/logout`, not
placeholders. They MUST preserve validation, recoverable errors, cancellation,
selection for the next turn, local revocation, and credential handling. A
failure of the secure storage MUST NOT implicitly select ephemeral storage.
Unsupported OAuth MUST remain reported as unsupported.

#### Scenario: OVL-05-A — Configure and select without restarting

- GIVEN a controlled connections service with a valid provider and model
- WHEN `/login` completes, `/providers` is inspected, and another model is selected with `/model`
- THEN the real service operations are performed and the next turn uses the confirmed selection
- AND a started turn keeps its connection/model except for the existing revocation blocks
- AND no step requires a placeholder nor introduces UI conversation

#### Scenario: OVL-05-B — Secure error and logout

- GIVEN secure storage that fails and an existing connection, in separate cases
- WHEN saving credentials is attempted or `/logout` is confirmed, respectively
- THEN the failure is shown outside the conversation without an implicit ephemeral fallback
- AND logout preserves local revocation and credential cleanup without asserting remote invalidation
- AND canceling any of the forms does not confirm the operation
