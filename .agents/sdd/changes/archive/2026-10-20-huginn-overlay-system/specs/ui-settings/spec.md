# ui-settings Delta

The integration preserves REQ-01 to REQ-16 except for the complete
substitutions of REQ-07, REQ-08, and REQ-12 below. It does not add the live
theme, the toast layer, or onboarding; it also does not turn `(+) Add provider…`
into a new functional flow.

## MODIFIED Requirements

### REQ-07 — Apply on Enter

When Enter is pressed to apply, the system SHALL capture the working copy into
`UiConfig` and invoke the injected persistence function
`Fn(&UiConfig) -> anyhow::Result<()>` exactly once per Apply, without blocking
input. While that Apply is pending, repeated Enter SHALL NOT start another
write. Saves SHALL be serialized to preserve their order. On success it SHALL
update the snapshot of saved preferences for later openings; only if the
overlay and the Apply are still current SHALL it close `Overlay::Settings` and
show `Settings saved` as a non-conversational `Notification`. On a current
failure it SHALL keep it open, preserve the submitted copy, and show the error
in the body without a second equivalent notice. A late receipt SHALL be
consumed without reopening Settings, closing or modifying another overlay,
recovering focus, or affecting Approval; a late success SHALL update only the
saved snapshot, not the editable copy of another Settings already open. A late
failure SHALL NOT replace the saved snapshot with failed values. It SHALL
preserve the joint application of all values and the TOML contracts of REQ-09
to REQ-11; it SHALL NOT apply the live theme.

Previously: standalone modal with a success notice; now integrated as an overlay
and with an explicit non-conversational destination.

#### Scenario: SET-07-A — Joint application without conversation

- GIVEN `Settings` with theme `catppuccin` and modified indicators values
- WHEN Enter is pressed and the persistence succeeds
- THEN it is invoked exactly once with all values of the copy
- AND the overlay closes and `Settings saved` appears outside the conversation
- AND there are no partial writes nor a live theme change

#### Scenario: SET-07-B — Error keeps it open

- GIVEN a modified copy, a current Apply, and a persistence that returns `disk full`
- WHEN Enter is pressed
- THEN the overlay stays open with the same copy and the error visible in the body
- AND no other equivalent representation of the error nor conversational entry appears
- AND Esc still allows discarding without a new persistence invocation

#### Scenario: SET-07-C — Pending Apply does not duplicate and allows a retry after a failure

- GIVEN an Apply whose controlled persistence stays pending
- WHEN Enter is pressed several times and then it completes with an error
- THEN the persistence has been invoked only once and Settings keeps the submitted copy with the error
- AND a new Enter after the failure can start a single retry
- AND the success of that retry updates the snapshot for the next opening

#### Scenario: SET-07-D — Late receipt with another Settings open

- GIVEN a started Apply, its Settings closed, and another Settings open with its own editable copy
- WHEN the earlier receipt arrives, with success and with error in separate cases
- THEN the new Settings keeps the copy, focus, and state of its own operation
- AND the success updates the saved snapshot used in a later opening
- AND the error does not replace the saved snapshot nor insert an error in the new Settings

### REQ-08 — Discard on Esc

When Esc is pressed outside an incomplete paste, the system SHALL close
`Overlay::Settings`, discard its editable copy, and revoke the lease without
canceling the current turn. Before starting an Apply it SHALL show
`Settings discarded` as a non-conversational `Notification` and SHALL NOT
invoke persistence nor modify the file. If the Apply already started a write,
Esc SHALL NOT start an additional write nor promise a rollback or that the
file stays intact. The receipt of the started Apply SHALL still follow the
REQ-07 contract without reinstalling the overlay or its focus.

Previously: modal discard and notice; now a close with focus revocation and
explicit classification, preserving REQ-13.

#### Scenario: SET-08-A — Discard during streaming

- GIVEN `Settings` with unsaved changes, no Apply started, and a turn streaming
- WHEN Esc is pressed
- THEN it closes without persisting and the file keeps its bytes
- AND the turn continues and the key does not reach the editor nor the turn canceler
- AND `Settings discarded` is shown outside the conversation

#### Scenario: SET-08-B — Esc during a started write

- GIVEN an Apply whose write has started and is stopped by a controlled barrier, with a turn streaming
- WHEN Esc is pressed and then the write is released with success or failure, in separate cases
- THEN Settings closes immediately, revokes its lease, and does not cancel the turn
- AND the persistence has been invoked only once, with no discard write and no promised rollback
- AND the receipt does not reopen Settings nor recover focus nor add conversation
- AND on success the next opening after the receipt uses the saved snapshot; on failure it keeps the previous snapshot

### REQ-12 — Modal precedence

The system SHALL use `Approval > active ordinary overlay > completion`.
`Settings`, plugin dialogs, and workflows SHALL be ordinary overlays, not slots
with simultaneous priorities. A new authorized and current ordinary request
SHALL replace the current ordinary overlay; a rejected or revoked one SHALL NOT
do so. Replacing Settings SHALL discard its editable copy and revoke the lease;
before starting an Apply it SHALL NOT write. If the write already started,
replacing SHALL NOT start another one nor guarantee a rollback, and its receipt
SHALL comply with REQ-07 without altering the substitute overlay. Approval SHALL
preempt Settings and any other overlay; no ordinary overlay SHALL replace a
pending Approval. When resolving or closing Approval, Settings SHALL NOT be
restored automatically.

Previously: the chain `Modal > Settings > PluginDialog > completion`
contradicted the scenario that replaced Settings with a plugin dialog. This
substitution preserves that authorized replacement without downgrading
Approval.

#### Scenario: SET-12-A — Approval preempts Settings

- GIVEN `Settings` open on `sound` with unsaved changes and no Apply started
- WHEN the host requests Approval for a tool
- THEN Settings is discarded without persisting and Approval takes the focus and render
- AND when Approval is resolved, Settings does not reappear

#### Scenario: SET-12-B — Authorized dialog replaces Settings

- GIVEN `Settings` open on `theme` with no Apply started
- WHEN a current authorized command returns a `Dialog` result
- THEN Settings is discarded without persisting and the plugin dialog is rendered
- AND input captured under the previous lease does not reach the new dialog

#### Scenario: SET-12-C — Rejected request does not replace

- GIVEN `Settings` open with a modified copy, or a pending Approval in another case
- WHEN a revoked ordinary request arrives, or an ordinary one while Approval is pending, respectively
- THEN the current overlay keeps focus and content
- AND the rejected request starts no save nor grants permission, even if an earlier Apply may complete its write

#### Scenario: SET-12-D — Approval during Apply and late receipt

- GIVEN an Apply with a write already started and pending under a controlled barrier
- WHEN the host opens Approval and then the receipt arrives with success or error, in separate cases
- THEN Approval immediately preempts Settings and revokes its lease without starting another write
- AND the receipt does not close, resolve, or modify Approval and does not restore Settings nor its focus
- AND only a valid explicit acceptance of Approval can grant permission
- AND the success updates the saved snapshot for a later reopening; the error keeps the previous one
- AND no cancellation asserts undoing the started write nor publishes the copy as conversation
