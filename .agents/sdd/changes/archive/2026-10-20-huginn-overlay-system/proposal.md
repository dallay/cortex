# Proposal: Huginn overlay system

## Intent

Separate the TUI interactions from the conversation: help, forms,
configuration and notices must not contaminate the history. Unify overlay
focus, render and close, preserving the functional flows and the security guarantees.

This proposal fixes the approved decisions and replaces the incompatible
suggestions from `exploration.md` (toasts, placeholders and conversation dedup).
The RPI plan referenced there does not exist at the given path; its content is not presumed.

## Scope

### Included: P0 + P1, a single PR

- Overlay enum in presentation: Settings, Provider, Login, ModelPicker,
  Help, Confirm and Approval, plus hosting the existing dialogs/workflows.
  Settings absorbs `SettingsDialog`; Approval keeps its own `oneshot`,
  separate from the ordinary confirmations.
- Five categories: Conversation, Overlay, Notification, Status and Diagnostic,
  with explicit methods per category, without a generic `notice` nor classification
  by prefixes. Notifications and diagnostics use non-conversational surfaces.
- `CommandResult::Overlay(OverlayKind)` in plugin-api to request the overlays
  allowed by the host, without giving plugins the ability to approve actions.
- Compact bar always visible, also with an overlay open; provider,
  model and mode updated through `Event::ConnectionChanged` in core.
- Rename `committed` to `conversation`; limit the UI projection event buffer
  to a `VecDeque` of 256 entries, without truncating the persisted session.
- Deduplicate errors by message hash in a 2 second window, with an injectable
  clock, operation scope and equality check on collisions.
- Migrate consumers, fixtures and tests together with the change; keep providers,
  login and model functional, Settings persistence, secrets and leases/paste.

### Exclusions

- Toast layer, live theme application and P2 onboarding.
- Replace providers/login/model with placeholders, widen plugin permissions
  or change persistence/configuration contracts without migration need.
- Reopen or modify `settings-modal/state.yaml`, nor consider its gates met
  for appearing as archived without verify/qa reports.

## Capabilities

- **New:** `huginn-overlays` (lifecycle, focus, commands and Approval),
  `huginn-ui-events` (categories, history and dedup), `huginn-status-bar`
  (visibility and connection synchronization).
- **Modified:** `ui-settings`, to integrate Settings and its notices without losing
  working copy, apply/discard or persistence. `providers-ui` describes the Rook
  dashboard, not these TUI flows; it is not modified because of a name coincidence.

## Approach

Strangler-fig migration: introduce the new surfaces, move each flow
with its verifications and remove the legacy fields only at the end. The host keeps
the Approval authority and revokes focus when closing, replacing or cancelling overlays.
Category routing happens before deduplication: a UI interaction never
becomes conversation, not even when the window expires. Legitimate conversational
messages are not deduplicated and errors from independent operations are not mixed.

## Affected areas

- `crates/huginn/presentation/src/{state.rs,terminal.rs}`: projection, history,
  input, snapshots and render; new overlay, category and bar modules.
- `crates/huginn/presentation/src/{settings.rs,providers.rs,contributions.rs,workflow.rs}`:
  adaptation of the real flows and their results.
- `crates/huginn/plugin-api/src/v1.rs`: declarative overlay result.
- `crates/huginn/core/src/lib.rs` and
  `crates/huginn/application/src/connections.rs`: selection event and publication.
- `apps/huginn/src/` and event consumers: initial wiring and compatibility.
- `crates/huginn/presentation/tests/` and `apps/huginn/tests/providers_flow.rs`:
  history, overlay, connection, security and PTY regressions.

## Risks and technical gaps

- **High, focus/Approval:** avoid accidental approvals, late responses and
  secret leakage when replacing overlays; cancellation/close/shutdown fail-closed.
- **Medium, incomplete contract:** `ConnectionChanged` does not exist yet; design must
  specify payload, emitter, initial snapshot and ordering against async changes.
- **Medium, dedup:** specify operation identity, state limit and expiration
  and the temporal anchoring of the window; the hash is not enough and its implementation
  is left to design, without imposing a new dependency such as fxhash.
- **Medium, precedence:** `ui-settings` REQ-12 contains a string and a contradictory
  plugin/Settings scenario; spec must make the replacement contract consistent
  without changing the Approval priority or the focus guarantees.
- **High, regressions and review:** a P0+P1 PR can exceed the review budget;
  tasks must estimate it and keep the risk visible, maintaining the approved single delivery.

## Rollback

Revert the complete P0+P1 PR, including plugin-api, the core event, consumers and
tests, to avoid half-migrated contracts. It requires no data migration: keep
the existing sessions, credentials and `[ui]`; never reset the database.

## Dependencies and pending validation

Reuse Settings and the existing connection services, without taking
`settings-modal` as validated: verify/qa reports are missing and they do not
equal an approved gate. The next verify/qa must bring its own evidence. The `pty`
target is conditioned by `test-driver` and Unix; the `tui-fixture` binary does not
contain those tests. `--all-features` does activate `test-driver`. Do not attribute
failures to the base or to flakiness without a comparison. This phase runs no builds
nor runtime tests.

## Verifiable success criteria

- [ ] Opening/closing help, Settings and providers/login/model flows adds no entries
  to the conversation, even when the action is repeated after more than 2 seconds.
- [ ] User/agent messages and relevant tool results remain in the conversation;
  notices, UI errors and diagnostics stay outside.
- [ ] Settings preserves apply/discard/persistence errors; providers/login/model
  complete their existing operations without placeholders.
- [ ] Approval preempts other overlays; close, cancellation, shutdown and stale
  callbacks do not grant permissions. Plugins cannot create or resolve Approval.
- [ ] Secrets do not appear in history/logs; stale input and paste do not cross leases.
- [ ] The bar visible with and without overlay reflects provider/model/mode when the connection changes.
- [ ] With a controlled clock, equal errors from the same operation are suppressed within
  2 seconds and reappear outside; distinct operations and collisions are not suppressed.
- [ ] The projection keeps at most 256 events and drops the oldest ones;
  synchronization, streaming and persisted history keep their behavior.
- [ ] Legacy fields are removed and tests/fixtures are adapted to the new contract;
  PTY texts may move to overlays, they never stay in the history just for a test.
- [ ] Own Verify and QA document evidence, failures and untested cases before archiving.
