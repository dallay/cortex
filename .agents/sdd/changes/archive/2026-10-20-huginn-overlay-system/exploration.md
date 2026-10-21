# Exploration: Huginn TUI Overlay System

> **Change slug**: `huginn-overlay-system`
> **Phase**: 1 — Explore
> **Artifact store**: `.agents/sdd/`
> **Parent plan**: `.agents/rpi/plan/tasks/huginn-overlay-system.md`

## Intent

Refactor `crates/huginn/presentation` so that **an interaction with the UI is
not a conversation message**. Conversation history persists only user/agent
messages and relevant tool results. Everything else routes through dedicated
overlays (`Overlay::Settings | Provider | Login | ModelPicker | Help |
Confirm | Toast`) or a persistent status bar. Errors are deduplicated.
This document records the verified current state of the codebase (the
mapping the parent plan describes) plus concrete counts of every call site
that must move, and proposes — for the proposal phase to lock in — the
shapes of `Overlay`, `EventKind`, `StatusBar`, and the dedup algorithm.

## Conventions in This Repo

- Workspace package: `apps/huginn` (binary) + `crates/huginn/{core,application,runtime,presentation,plugin-api,reference-extension}`. Root `Cargo.toml`.
- Presentation crate: `crates/huginn/presentation/`. Terminal I/O lives behind `Ratatui` (`terminal.rs`) and is driven by a deterministic state projection in `state.rs`.
- This change is a sibling of `.agents/sdd/changes/settings-modal/` which already landed `state.settings: Option<SettingsDialog>`. We absorb it as `Overlay::Settings(SettingsDialog)`.
- Prior precedent: `.agents/sdd/changes/provider-quota-tracking/`. Markdown headings, kebab-case filenames, `state.yaml` with `current_phase`/`completed`/`next`.

## Verified Map (with line numbers)

### State projection (`crates/huginn/presentation/src/state.rs`)

`UiState` (line 134) owns the four legacy overlay slots plus `committed`
and `status`:

| Field                                                         | Type                            | Line    | Notes                                        |
|---------------------------------------------------------------|---------------------------------|---------|----------------------------------------------|
| `editor`                                                      | `Editor`                        | 135     | Composer input                               |
| `completion`                                                  | `Completion`                    | 136     | `/`-prefix palette                           |
| `files`, `files_ready`                                        | `Vec<String>`, `bool`           | 137-138 | File index snapshot                          |
| `completion_editor`                                           | `Option<(String, usize, bool)>` | 139     | Editor fingerprint                           |
| `completion_catalog`                                          | `Vec<(String, u64)>`            | 140     | Catalog fingerprint                          |
| `tail`                                                        | `String`                        | 141     | Streaming in-flight text                     |
| `status`                                                      | `String`                        | 142     | **Sovereign status string**                  |
| `status_error`                                                | `bool`                          | 145     | Severity flag for `status`                   |
| `modal`                                                       | `Option<Modal>`                 | 146     | Approval prompt with `oneshot` reply channel |
| `plugin_dialog`                                               | `Option<PluginDialog>`          | 147     | Host-owned selection, no approval authority  |
| `workflow`                                                    | `Option<HostWorkflow>`          | 148     | Multi-step plugin form (workflow.rs)         |
| `workflow_focus`                                              | `u64`                           | 150     | Input lease, bumps on open/close             |
| `settings`                                                    | `Option<SettingsDialog>`        | 151     | **Absorbed from settings-modal**             |
| `settings_focus_epoch`                                        | `u64`                           | 153     | Input lease for settings                     |
| `widgets`                                                     | `Vec<(String, View)>`           | 154     | Contributed widgets                          |
| `busy`                                                        | `bool`                          | 156     | Turn in flight                               |
| `turn_cancel`                                                 | `Option<CancellationToken>`     | 157     | Current turn cancel token                    |
| `committed`                                                   | `Vec<String>`                   | 158     | **Universal scrollback sink**                |
| `events`                                                      | `Vec<Event>`                    | 159     | Authoritative event log                      |
| `session_id`, `message_count`, `epoch`, `approvals_this_turn` | `String`/`usize`/`u64`/`u64`    | 160-165 | Synchronization state                        |

`Modal` (line 112): `epoch`, `number`, `request: ApprovalRequest`, `code`,
`typed`, `armed`, `scroll`, `last_scroll`, `reply: Option<oneshot::Sender<bool>>`.

`PluginDialog` (line 168): `handle: DialogHandle`, `selected: usize`,
`body_scroll: usize`.

`Ui` (line 329): `state: Mutex<UiState>`, `dirty: Notify`, `stop:
CancellationToken`, `contributions: Arc<Contributions>`, `ui_config:
Arc<UiConfig>`, `save_ui_config: SaveUiConfig`, plus `requests` and
`prompts` async mutexes.

### `View` snapshot (`crates/huginn/presentation/src/terminal.rs`)

`View` (line 380) is the render-side mirror of `UiState`:

| Field           | Type                                                                           | Line |
|-----------------|--------------------------------------------------------------------------------|------|
| `editor`        | `Editor`                                                                       | 381  |
| `completion`    | `Suggestions`                                                                  | 382  |
| `tail`          | `String`                                                                       | 383  |
| `status`        | `String`                                                                       | 384  |
| `status_error`  | `bool`                                                                         | 385  |
| `busy`          | `bool`                                                                         | 386  |
| `modal`         | `Option<(String, u64, String, String, usize)>`                                 | 387  |
| `plugin_dialog` | `Option<(String, String, Vec<String>, usize, usize)>`                          | 388  |
| `workflow`      | `Option<(String, String, Vec<String>, usize, bool)>`                           | 389  |
| `settings`      | `Option<(Vec<String>, usize, Vec<(String, bool, u8)>, usize, Option<String>)>` | 390  |
| `widgets`       | `String`                                                                       | 391  |

`View::snapshot` (line 394) flattens each `Option<_>` into a flat tuple —
this is what makes the existing render work as a chain of `if let Some(...)`
in `draw_frame`.

### Render (`crates/huginn/presentation/src/terminal.rs`)

`draw_frame` (line 536) has **four sequential overlay branches**, ordered:

1. `modal` — line 540 (full-screen approval with code entry)
2. `workflow` — line 568 (header + content + footer)
3. `settings` — line 586 (tabbed header + body + footer) — comment at line 584 documents precedence: `Modal > Settings > PluginDialog`
4. `plugin_dialog` — line 639 (title/body/choices, windowed)
5. Tail/composer/status area — line 681 (the **non-overlay** branch: tail + completion popup + composer + status line)

`update_modal_scroll` (line 511) round-trips `last_scroll` from the
renderer back into `state.modal` keyed by code.

### Input dispatch (`crates/huginn/presentation/src/terminal.rs`)

`Driver::input` (line 843) is a single nested match:

1. `state.workflow_focus != focus` → discard (line 846)
2. `state.modal.is_none()` + `state.workflow.is_some()` → form keys (line 850-887)
3. Paste → composer or drop (line 889-898)
4. Ctrl+C → cancel/resolve/stop (line 906-916)
5. `state.modal.is_none()` + `state.settings.is_some()` → settings keys (line 917-959)
6. `state.modal.is_none()` + `state.plugin_dialog.is_some()` → plugin keys (line 960-979)
7. `state.modal.is_some()` → modal keys (line 981-1007)
8. Completion navigate/accept (line 1008-1045)
9. Plain composer keys (line 1055-1103)

The precedence in the **input** side is: modal > workflow > settings >
plugin_dialog > modal. There is an inconsistency at line 419 vs. 584 — the
input side checks settings before plugin_dialog, the render side checks
settings between workflow and plugin_dialog. **The render order is the
canonical one** (it has the comment). Migrating to a single `Overlay`
slot will reconcile this.

### Lifecycles (`crates/huginn/presentation/src/state.rs`)

- `Ui::dispatch_command` (line 377): special-cases `/settings` (line 384-397), then walks plugin commands.
- `Ui::dispatch_handle` (line 406): handles `Notice(view)` → `notice`, `Dialog(id)` → opens `PluginDialog`, `Workflow(wf)` → `open_workflow`, `Err` → `notice`.
- `Ui::refresh_completion` (line 428): returns early if any overlay is open (line 459).
- `Ui::start_file_index` (line 488): writes `status` directly when truncated/skipped (line 509, 523).
- `Ui::refresh_contributions` (line 531): cleans expired `workflow`, expired `plugin_dialog`, refreshes widgets. **Does NOT clean `settings`** — that is done in `request_decision` (line 707).
- `Ui::open_workflow` (line 575): takes `plugin_dialog`, opens `workflow`. If `modal.is_some()`, refuses.
- `Ui::submit_workflow`, `Ui::cancel_workflow`, `Ui::choose_plugin_dialog`, `Ui::resolve_modal`, `Ui::shutdown` (lines 591-648).
- `Ui::shutdown` (line 637): takes `modal`, `plugin_dialog`, `workflow`. **Does NOT take `settings`** — see Risks.
- `Ui::request_decision` (line 680): bumps epoch, takes `plugin_dialog`, `workflow`, `settings`, opens `Modal`.

### Contributions (`crates/huginn/presentation/src/contributions.rs`)

Line 24-26 — `/help` is registered as a command that returns
`CommandResult::Notice(View::text("Enter sends; Alt+Enter or Ctrl+J inserts a
newline. ..."))`. **This is the help text the user wants moved into
`Overlay::Help`.** No terminal surface today for it other than
`state.committed.push(text)` from `Ui::notice`.

### Providers (`crates/huginn/presentation/src/providers.rs`)

`ProvidersUiPlugin` registers four commands (`/login`, `/providers`,
`/model`, `/logout`) — all return `CommandResult::Workflow(...)`. They all
flow into `state.workflow` today. After the migration they should open
`Overlay::Login`, `Overlay::Provider`, `Overlay::ModelPicker` (logout is a
special case that today uses `Workflow` for confirmation but could go to
`Overlay::Confirm`).

### Settings (`crates/huginn/presentation/src/settings.rs`)

`SettingsDialog` (line 196), `SettingsSnapshot` (line 134), `WorkingCopy`
(line 180), `SaveUiConfig` (line 89), DTOs (`UiConfig`/`UiTheme`/`UiSound`/
`UiToasts`/`UiIndicators`/`UiIntegrations`) at lines 13-78. The DTOs are
already `Deserialize`/`Serialize` and are consumed by `apps/huginn/src/config.rs`
via `use huginn_presentation::settings::UiConfig`. **We move the DTOs to
`presentation::config` (or re-export from there)** so consumers don't
break; `settings.rs` becomes a thin host around `Overlay::Settings`.

### Apps (`apps/huginn/src/{main.rs,config.rs,ui_settings.rs}`)

- `config.rs:2` — `use huginn_presentation::settings::UiConfig;` — line 2 imports. After move it becomes `use huginn_presentation::config::UiConfig;`.
- `main.rs:10` — same import. After move, same path.
- `main.rs:470-477` — wires `ui_config` (Arc<UiConfig>) and `save_ui_config` into `RatatuiPlugin::with_settings`.
- `ui_settings.rs:8` — same import.

These three import paths are the **only public API consumer** for the
DTOs. The migration is mechanical: update three `use` lines.

## Call-site counts (the concrete numbers)

### `notice()` call sites (production)

17 total across the codebase.

| Path          | Line    | Code                                                                                      | Tentative category                                                                    |
|---------------|---------|-------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------|
| `state.rs`    | 408     | `self.notice(view.plain_text())` from `Notice(view)`                                      | Notification (transient status)                                                       |
| `state.rs`    | 420     | `self.notice(format!("Unavailable plugin dialog: {id}"))`                                 | Notification                                                                          |
| `state.rs`    | 424     | `self.notice(error.to_string())` from `Err`                                               | Notification (dedup-eligible)                                                         |
| `state.rs`    | 554     | `self.notice(view.plain_text())` from completed workflow `Notice`                         | Notification                                                                          |
| `state.rs`    | 555-558 | `self.notice("UI action failed; retry or reopen...")` from non-Notice/non-Workflow result | Notification (dedup-eligible)                                                         |
| `state.rs`    | 611-613 | `self.notice("Action cancelled locally...")` from `cancel_workflow`                       | Notification                                                                          |
| `state.rs`    | 614     | `self.notice("Form discarded.")` from `cancel_workflow`                                   | Notification                                                                          |
| `state.rs`    | 623     | `self.notice(view.plain_text())` from plugin dialog choose                                | Notification                                                                          |
| `state.rs`    | 624     | `self.notice(error.to_string())` from plugin dialog choose error                          | Notification (dedup-eligible)                                                         |
| `state.rs`    | 674-678 | `self.notice(format!("Authorized by --allow: ..."))` from `granted()`                     | Notification (should go to history? user explicitly said `--allow` is interesting)    |
| `terminal.rs` | 289-293 | `ui.notice("No model configured...")` (startup, no login)                                 | Notification (overlay candidate — should be a transient `Overlay::Confirm` or status) |
| `terminal.rs` | 830     | `ui.notice("Paste exceeded the 64 KiB input limit...")`                                   | Notification (input overflow)                                                         |
| `terminal.rs` | 929     | `ui.notice("Settings discarded")`                                                         | Notification                                                                          |
| `terminal.rs` | 941     | `ui.notice("Add provider not implemented yet")`                                           | Notification                                                                          |
| `terminal.rs` | 953     | `ui.notice(message)` from `SettingsDialog::apply`                                         | Notification                                                                          |

### `notice()` in tests/support

| Path         | Line | Code                                                                           |
|--------------|------|--------------------------------------------------------------------------------|
| `fixture.rs` | 328  | `connection.ui.notice("REPLACEMENT_READY")` — PTY fixture, replace mode        |
| `fixture.rs` | 346  | `connection.ui.notice("APPROVAL_CLOSED")` — PTY fixture, replace-approval mode |
| `fixture.rs` | 355  | `next.ui.notice("NEW_GENERATION_READY")` — PTY fixture, drop-reopen mode       |
| `fixture.rs` | 398  | `ui.notice(error.to_string())` — engine error fallback                         |

### `committed.push` sites

Exactly **2** in `state.rs`:

| Path       | Line | Code                                                              |
|------------|------|-------------------------------------------------------------------|
| `state.rs` | 264  | `self.committed.push(text)` — `synchronize_views`                 |
| `state.rs` | 765  | `self.lock().committed.push(safe(&text))` — `Interaction::notice` |

There are no other direct `state.committed.push` call sites outside of
`state.rs`. Every `notice()` ends up here.

### `state.status =` and `state.status_error =` sites

| Path          | Line    | Code                                                                                                     |
|---------------|---------|----------------------------------------------------------------------------------------------------------|
| `state.rs`    | 200     | `self.status = format!("Tool: {}", safe(&call.name));` from `ToolStarted`                                |
| `state.rs`    | 210-214 | `self.status = ...` from `ToolFinished` (error path)                                                     |
| `state.rs`    | 218     | `self.status = format!("Approval: {}", safe(&request.action))` from `ApprovalRequested`                  |
| `state.rs`    | 221     | `self.status = ...` from `ApprovalResolved`                                                              |
| `state.rs`    | 224     | `self.status = "Thinking…"` from `TurnStarted`                                                           |
| `state.rs`    | 228     | `self.status = "Ready"` from `TurnFinished`                                                              |
| `state.rs`    | 231     | `self.status = safe(message)` from `Error` event                                                         |
| `state.rs`    | 232     | `self.status = "Context compacted..."` from `Compacted`                                                  |
| `state.rs`    | 236     | `self.status = format!("Tokens: ...")` from `Usage`                                                      |
| `state.rs`    | 272     | `self.status = "Interrupted: ..."` from `synchronize`                                                    |
| `state.rs`    | 509-512 | `state.status = format!("Partial file index: limit {}, skipped directories {}")` from `start_file_index` |
| `state.rs`    | 523     | `state.status = format!("Could not index the workspace: {}", safe(&message))`                            |
| `state.rs`    | 540     | `state.status = "UI action no longer available"` from `refresh_contributions`                            |
| `terminal.rs` | 1052    | `state.status = "The command changed; edit the draft before running it"`                                 |

`status_error = true` only at `state.rs:209` (ToolFinished error) and
`state.rs:524` (workspace index failure). Cleared at `:225, :229, :270`.

### `state.modal` sites

7 in `state.rs`:

| Path       | Line | Code                                                                                            |
|------------|------|-------------------------------------------------------------------------------------------------|
| `state.rs` | 390  | `state.modal.is_none() && state.settings.is_none()` in `dispatch_command` for `/settings` guard |
| `state.rs` | 412  | `state.modal.is_none()` guard in `dispatch_handle` before opening `PluginDialog`                |
| `state.rs` | 459  | completion refresh guard: `state.modal.is_some()`                                               |
| `state.rs` | 580  | `state.modal.is_some()` guard in `open_workflow`                                                |
| `state.rs` | 643  | `state.modal.take()` in `shutdown`                                                              |
| `state.rs` | 709  | `state.modal = Some(Modal {...})` in `request_decision`                                         |
| `state.rs` | 735  | `state.modal.take()` in `request_decision` cleanup                                              |

In `terminal.rs`: `:448` (snapshot), `:850` (input gate), `:890, :917, :981`
(input branches), `:513` (`update_modal_scroll`).

### `state.plugin_dialog` sites

10 in `state.rs`:

| Path       | Line    | Code                                                                                  |
|------------|---------|---------------------------------------------------------------------------------------|
| `state.rs` | 147     | field declaration                                                                     |
| `state.rs` | 413     | `state.plugin_dialog = Some(PluginDialog {...})` in `dispatch_handle`                 |
| `state.rs` | 459     | completion refresh guard                                                              |
| `state.rs` | 564     | `state.plugin_dialog.as_ref().is_some_and(\|d\| !d.handle.is_active())` expired check |
| `state.rs` | 568     | `state.plugin_dialog.take()` in `refresh_contributions`                               |
| `state.rs` | 583     | `state.plugin_dialog.take()` in `open_workflow`                                       |
| `state.rs` | 619-620 | `choose_plugin_dialog`                                                                |
| `state.rs` | 644     | `state.plugin_dialog.take()` in `shutdown`                                            |
| `state.rs` | 704     | `state.plugin_dialog.take()` in `request_decision`                                    |

### `state.workflow` sites (incl. `workflow_focus`)

13 in `state.rs`:

| Path       | Line    | Code                                                                             |
|------------|---------|----------------------------------------------------------------------------------|
| `state.rs` | 148     | field                                                                            |
| `state.rs` | 459     | refresh guard                                                                    |
| `state.rs` | 536-547 | `state.workflow.as_mut()` + `take()` in `refresh_contributions`                  |
| `state.rs` | 588     | `state.workflow = Some(form)` in `open_workflow`                                 |
| `state.rs` | 593-595 | `state.workflow.as_mut().is_some_and(HostWorkflow::submit)` in `submit_workflow` |
| `state.rs` | 601-606 | `state.workflow.as_ref()` + `take()` in `cancel_workflow`                        |
| `state.rs` | 645     | `state.workflow.take()` in `shutdown`                                            |
| `state.rs` | 705     | `state.workflow.take()` in `request_decision`                                    |

`workflow_focus` increments at `:539, :545, :589, :606, :646, :706`.

In `terminal.rs`: `:752, :800, :834, :846, :851` — the input lease dance.

### `state.settings` sites (incl. `settings_focus_epoch`)

8 in `state.rs`:

| Path       | Line | Code                                              |
|------------|------|---------------------------------------------------|
| `state.rs` | 151  | field                                             |
| `state.rs` | 153  | `settings_focus_epoch` field                      |
| `state.rs` | 390  | guard: `state.settings.is_none()`                 |
| `state.rs` | 391  | `state.settings = Some(SettingsDialog::new(...))` |
| `state.rs` | 392  | `state.settings_focus_epoch = ...wrapping_add(1)` |
| `state.rs` | 707  | `state.settings.take()` in `request_decision`     |
| `state.rs` | 708  | `state.settings_focus_epoch = ...wrapping_add(1)` |

In `terminal.rs`: `:415-441` (snapshot), `:920-947` (input keys).

### Help text in contributions

`contributions.rs:24-26` — the long help text registered to `/help` via
`CommandResult::Notice`. **This is the most visible violation of "UI ≠
conversation message"** the user cited.

### Workspace diagnostics

`state.rs:509-512` writes `"Partial file index: limit {}, skipped directories {}"`
to `state.status` — already a Status target. Belongs in `EventKind::Status`,
not `Notification`.

`state.rs:523` writes `"Could not index the workspace: {}"` — `status_error =
true`. Belongs in `EventKind::Status` with severity.

## Tests that will need migration

10 test target files in `crates/huginn/presentation/tests/` + `apps/huginn/tests/`.
Currently 79 direct references to overlay/completed/state fields.

| File                                  | Lines | Touches                                                                                                      | Estimated migration                                                             |
|---------------------------------------|-------|--------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------|
| `state.rs` (test)                     | 274   | `modal` (5), `committed` (4), `events` (2), `status`/`status_error` (3)                                      | rewrite to `Overlay::Confirm`/`Overlay::Modal` projections                      |
| `workflows.rs`                        | 240   | `workflow` (8), `committed` (10)                                                                             | rewrite to `Overlay::Form(HostWorkflow)` projection                             |
| `extensions.rs`                       | 144   | `plugin_dialog` (3), `modal` (1), `committed` (2), `status_error` (1)                                        | rewrite to `Overlay::PluginDialog` projection                                   |
| `providers_ui.rs`                     | 494   | `workflow` (~18), `committed` (4), `settings` (5 — these are `ConnectionSettings`, **not** the UI modal, OK) | rewrite to `Overlay::Login/Provider/ModelPicker`                                |
| `plugins.rs`                          | 98    | none                                                                                                         | no change                                                                       |
| `pty.rs`                              | 621   | uses substring matching on PTY output (`"Secret form"`, `"Huginn"`, `"********"`)                            | **no semantic change**; substrings stay the same because the rendered text does |
| `completion.rs`                       | 284   | none                                                                                                         | no change                                                                       |
| `files.rs`                            | 50    | none                                                                                                         | no change                                                                       |
| `input_parser.rs`                     | 170   | none                                                                                                         | no change                                                                       |
| `support/fixture.rs`                  | 404   | `notice` (4)                                                                                                 | rewrite to use the new `notice` signature; PTY strings stay                     |
| `apps/huginn/tests/providers_flow.rs` | n/a   | `committed.join("\n").contains("No connections")` (line 554)                                                 | rewrite assertion                                                               |

**Net: 5 files rewrite, 6 unchanged.** Migration must ship in the same PR —
failing tests are treated as a contract violation per the parent plan.

## Design proposals (NOT locked — for `proposal.md` to confirm)

### `Overlay` enum shape

**Recommended**: an enum with structs as payload. **No trait** — the enum
itself is the polymorphic slot; methods on the enum dispatch by match.

```rust
pub enum Overlay {
    Settings(SettingsDialog),                                   // already exists
    Provider(ProviderManager),                                  // new (placeholder data acceptable per in-house P1 scope)
    Login(LoginDialog),                                         // new (wraps current /login flow)
    ModelPicker(ModelPicker),                                   // new (wraps /model flow)
    Help(HelpDialog),                                           // new (moves the contributions.rs:24-26 text)
    Confirm(ConfirmDialog),                                     // new (general "are you sure?" — replaces ad-hoc Notice)
    Toast(Toast),                                               // new (transient status surfacing for commands)
}

impl Overlay {
    pub fn render(&self, area: Rect, frame: &mut Frame, state: &UiState) { ... }
    pub fn handle_key(&mut self, key: KeyEvent, ui: &Arc<Ui>) -> KeyOutcome { ... }
    pub fn close_epoch(&mut self) -> u64 { ... }                  // bumped to revoke focus lease
}

pub enum KeyOutcome {
    Consumed,                                                   // render input lease continues
    Closed,                                                     // overlay closed (caller bumps focus)
    Ignored,                                                    // key didn't match (fall through to editor)
}
```

**Why not `Box<dyn OverlayHandler>`**: `Box<dyn ...>` adds heap allocation
per open, and trait objects can't carry associated data per variant as
cleanly as an enum. The render and input paths are simple matches; the
monomorphization cost is one-time per call site.

**Why payload types are local structs (not `PluginDialog` reused)**: each
overlay variant needs slightly different state (e.g. `Confirm` carries a
oneshot reply; `Login` carries a `Pending` like `HostWorkflow` does).
Sharing `PluginDialog` would force a tagged union on the inside, which is
what we're escaping.

**`Overlay::Settings` reuses `SettingsDialog` directly** (no wrapping
struct). This keeps the settings-modal change contract intact during the
strangler-fig migration.

### `EventKind` enum

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    Conversation,   // user + agent messages; routed to `state.committed`
    Overlay,        // instructions / help / settings body content; routed to Overlay::Help etc.
    Notification,   // "Form discarded.", "Settings saved"; routed to Overlay::Toast (transient)
    Status,         // live state, not history; routed to StatusBar
    Diagnostic,     // workspace index, file count; routed to StatusBar (verbose) or a debug channel
}
```

Routing tables (still TBD in `proposal.md`):

| Source                                     | Current                          | New (EventKind)                                      |
|--------------------------------------------|----------------------------------|------------------------------------------------------|
| `Event::Text { text }` from user/agent     | `state.tail` → `state.committed` | `Conversation` → `state.committed`                   |
| `Event::ToolFinished { output, is_error }` | `state.tail` + `state.status`    | `Conversation` (output) + `Status` (severity)        |
| `Event::TurnStarted/TurnFinished`          | `state.status`                   | `Status`                                             |
| `Event::Error { message }`                 | `state.status`                   | `Notification` (transient toast) — **dedup applies** |
| `Event::ApprovalRequested/Resolved`        | `state.status`                   | `Status`                                             |
| `Event::Compacted`                         | `state.status`                   | `Notification` (with Severity::Warn)                 |
| `Event::Usage`                             | `state.status`                   | `Status`                                             |
| `state.synchronize` history                | `state.committed.push`           | `Conversation`                                       |
| `Ui::notice(...)`                          | `state.committed.push`           | dispatch by *content category* (see below)           |
| `start_file_index` truncated               | `state.status`                   | `Diagnostic`                                         |

**`Ui::notice` must become a router**, not a push. Today everything funnels
to `committed`. After: classify by string prefix or by an explicit
`Notification` variant carried by callers — see Open Questions.

### Storage per kind

**Recommended**: keep `state.committed: Vec<String>` as the **only**
historical buffer, but every push goes through a router that also consults
the dedup filter. Toasts live in `state.toasts: VecDeque<Toast>` capped at
`UiToasts::duration_secs` worth or `4` items, evicted by tick. Status and
Diagnostic are **not stored** — they're projected on render from the live
event stream `state.events: VecDeque<Event>` (already exists as
`state.events: Vec<Event>`; convert to `VecDeque` for O (1) front eviction
and bounded memory). The existing `state.events` log already drops
`Text` to avoid unbounded growth (state.rs:242-244) — keep that policy.

### Deduplication algorithm

Two-tier: a fast inline filter on `state.committed` push, and a separate
"same-error toast" filter on toasts.

```rust
struct Dedup {
    last_hash: u64,
    last_at: Instant,
}

impl Dedup {
    fn should_emit(&mut self, kind: EventKind, message: &str) -> bool {
        // Hash the canonical tuple. FxHash is fine — not adversarial.
        let hash = fxhash::hash64(&(kind as u64, message));
        let now = Instant::now();
        if hash == self.last_hash && now.duration_since(self.last_at) <= Duration::from_secs(2) {
            self.last_at = now;
            false
        } else {
            self.last_hash = hash;
            self.last_at = now;
            true
        }
    }
}
```

Two filters: one for `state.committed` (Conversation history), one for
`state.toasts` (Notification). The `2s` window is measured from the **last dropped** event, not just the last emitted one — otherwise rapid
bursts would each get through after one second.

**Testing implication**: dedup needs an injectable clock (not
`Instant::now()` directly). Either an `Arc<dyn Clock>` in `UiState` or a
`#[cfg(test)] fn now(&self) -> Instant` method. Default impl calls
`Instant::now()`; tests inject a `MockClock` that returns controlled
values. Adds 1 test surface but keeps the algorithm deterministic.

### `StatusBar` shape

```rust
pub struct StatusBar {
    pub provider: Option<String>,         // updated by `refresh_contributions` on Event::ConnectionChanged (new)
    pub model: Option<String>,            // same
    pub workspace: PathBuf,               // set at `Ui::new`, never changes
    pub context: ContextBudget,           // from Event::Usage: input/output tokens
    pub symbols: &'static str,             // "⌘K Commands" literal — constant
    pub dot: DotState,                    // Idle | Busy | AwaitingApproval | Error
}

pub enum DotState { Idle, Busy, AwaitingApproval, Error }

impl StatusBar {
    pub fn render(&self, area: Rect, frame: &mut Frame) { ... }   // ● Provider · Model · Workspace · ⌘K Commands
}
```

**Source of truth**: each `Event` (`TurnStarted`, `TurnFinished`,
`ApprovalRequested`, `ApprovalResolved`, `Usage`, `Compacted`,
`Error`, `ConnectionChanged`) updates one or more fields. The
`StatusBar` is owned by `UiState` and rendered on every frame
between the conversation tail and the composer. The current
`state.status: String` field **is replaced** by `StatusBar` —
`state.status_error: bool` becomes the `DotState::Error` flag.

**No thread/race issue**: `StatusBar` lives under the existing
`UiState` mutex. The `EventSink::emit` path already takes the lock.

### Migration order (strangler-fig, per parent plan D8)

1. **Add `overlay.rs`, `status_bar.rs`, `event.rs`** modules. Don't yet wire them.
2. **Add `state.overlay: Option<Overlay>` + `state.status_bar: StatusBar`** alongside legacy fields.
3. **Migrate `Overlay::Settings`** first — this is the smallest, already-landed dialog. Touch only `dispatch_command` and the settings input branch.
4. **Migrate `Overlay::Help`** — convert `contributions.rs:24-26` from `Notice(...)` to a new `OpenOverlay(Overlay::Help)` `CommandResult` variant. Add the variant to `huginn_plugin_api`.
5. **Migrate `Overlay::Login/Provider/ModelPicker`** — `ProvidersUiPlugin` returns the new variants. `state.workflow` coexists with `state.overlay`.
6. **Migrate `Overlay::Confirm`** — replace ad-hoc `Notice` flows ("UI action failed", "Connection action failed", "Connection kept.") with `Confirm` toasts.
7. **Migrate `Overlay::Toast`** — wrap dedup-filtered `notice` callers.
8. **Migrate `Overlay::PluginDialog`** — keep existing struct, just wrap in enum.
9. **Migrate `Overlay` for approval `Modal`** — keep the struct, wrap.
10. **Wire `StatusBar`**: every `Event` handler updates fields; remove `state.status` / `state.status_error`.
11. **Remove legacy fields** (`state.modal`, `state.plugin_dialog`, `state.workflow`, `state.settings`, `state.committed` for non-conversation, `state.status`, `state.status_error`).
12. **Update tests** as you go. Each overlay migration step is its own commit with passing tests.

This is longer than 1 PR but matches the parent plan's "remove legacy
fields is the last task" guardrail.

## Risks

### Technical

1. **Locking**. `Overlay::handle_key(&mut self, key, &Arc<Ui>)` lets the
   overlay read the state without holding the lock — exactly the existing
   pattern (`state.rs:914`, `state.rs:949`). Each overlay borrows the
   `Arc<Ui>` and takes the lock as needed. No new locking primitives.
2. **Render polymorphism**. `draw_frame` becomes a single `match self.overlay {
   Some(o) => o.render(area, frame, &view),
   None => render_conversation(...),
   }`. The `Layout::vertical([Constraint::Min(1), Constraint::Length(2)])`
   split becomes the overlay's responsibility per variant. `Layout` calls
   move from `terminal.rs` into each overlay's `render`. That's the
   biggest code-motion in the change.
3. **Input polymorphism**. Same shape — `match self.overlay { Some(o) =>
   o.handle_key(key, ui), ... }`. Single match replaces the four
   `if state.X.is_some()` branches.
4. **Test surface**. 5 test files need rewriting (state.rs, workflows.rs,
   extensions.rs, providers_ui.rs, providers_flow.rs). They MUST migrate
   in the same PR. Failing tests = contract violation.
5. **Clock injection for dedup tests**. Add `Arc<dyn Clock>` or a
   `#[cfg(test)]` override. Low risk; we already use `tokio::time::pause()`
   in some tests.

### Process

1. **Settings-modal dependency**. The parent plan requires
   `settings-modal` to be archived first. It is **already merged** at
   `state.settings`/`state.settings_focus_epoch`. So this change can start
   immediately; the absorb is mechanical (wrap in `Overlay::Settings`).
2. **PTY tests are substring-based**. `pty.rs` asserts `pty.wait_for("Huginn")`,
   `pty.wait_for("Secret form fixture")`, `pty.wait_for("UI action failed")`,
   `pty.wait_for("Form discarded.")`. The rendered strings must stay the
   same during migration to avoid silent breakage. **Invariant: the
   migration preserves every string `pty.rs` waits for.** This is the
   single most important compatibility guarantee.
3. **Public API churn**. The `huginn_plugin_api::v1::CommandResult` enum
   gains `Overlay(OverlayKind)` (or similar). Plugin authors need a
   migration guide — but no external plugin is registered today other
   than the in-tree `ConversationPlugin`, `ProvidersUiPlugin`, and
   `reference-extension`.

### Compatibility

1. **DTO move**. `apps/huginn/src/{config.rs,main.rs,ui_settings.rs}` all
   `use huginn_presentation::settings::UiConfig`. After the move they
   become `use huginn_presentation::config::UiConfig`. Three import lines.
   `presentation::settings::UiConfig` can re-export to keep
   back-compat for one minor.
2. **`Ui::new` signature unchanged**. `ui_config: Arc<UiConfig>` and
   `save_ui_config: SaveUiConfig` stay. The injection moves through
   `state.overlay` instead of `state.settings` on open.
3. **PTY compatibility strings**. As above.

## Open questions (decide before `proposal.md`)

1. **`Ui::notice` API surface**. Three options:
    - **(a)** Keep `notice(text: String)` but route by content prefix (`"overlay:"`, `"status:"`, `"diag:"`) — ugly.
    - **(b)** Add `notice_event(kind: EventKind, text: String)` and
      deprecate `notice(text)`. Cleaner. Touches 11 callers.
    - **(c)** Replace `notice` with `conversational(text: String)`,
      `notification(text: String)`, `status(text: String)`. Most explicit.
    - **Recommendation**: (c). It's the largest surface change but the
      cleanest at the call sites, and it forces every caller to think
      about kind.
2. **`Overlay::Help` activation path**. Add a `CommandResult::OpenOverlay`
   variant to `huginn_plugin_api`, OR keep `/help` as a host-shortcut
   like `/settings`. The latter is simpler; `/help` is already a
   contributed command, but we can register a fresh plugin command that
   returns a new variant. **Recommendation**: add `CommandResult::Overlay(OverlayKind)`
   to `huginn_plugin_api::v1::CommandResult`. Plugin authors can then
   open any host overlay (Settings, Help, etc.) by name — useful for
   future plugin-provided "/configure" flows.
3. **Approval modal as `Overlay`**. The approval `Modal` carries a
   `oneshot::Sender<bool>` and is opened from `ApprovalPrompt::request_decision`,
   not from a user command. Treating it as `Overlay::Confirm` is fine if
   `Confirm` carries a reply channel. Otherwise keep it as a separate
   field. **Recommendation**: keep the approval `Modal` as its own slot
   OR as `Overlay::Approval`. The parent plan says "replace Modal", but
   its `oneshot` reply channel makes it a special case. **The user must
   decide** — this is the only category that touches an async reply.
4. **StatusBar source of truth on connection**. Today `provider` and
   `model` are tracked by `ConnectionsService` and propagated through
   `Event` (none exist yet for connection changes). **Need a new event
   variant** `ConnectionChanged { provider, model, mode }` emitted by
   `ConnectionsService` whenever the active selection changes. Without
   this, the status bar will not stay live. Confirm scope with user.
5. **`state.committed` vs `state.tail` vs `state.events`**. The current
   model is `tail` for streaming, `committed` for historical. After:
   `state.conversation: Vec<String>` (was `committed`), `state.tail:
   String` (unchanged), `state.events: VecDeque<Event>` (was
   `Vec<Event>`; needs ring-buffer trait). Or keep names — minimal
   churn. **Recommendation**: rename `committed` → `conversation` for
   clarity, but keep `tail` and `events`. Ring-buffer for `events` is
   needed if we add `ConnectionChanged` etc. — otherwise unbounded.

## What this exploration enables next

The `proposal.md` phase can lock:

- The `Overlay` enum shape (structs-not-trait).
- The `EventKind` taxonomy and routing table.
- The `Dedup` algorithm + clock injection.
- The `StatusBar` shape + the new `ConnectionChanged` event variant.
- The strangler-fig task list.
- The rename `state.committed` → `state.conversation`.
- The PTY-string invariant.

The user must decide before `proposal.md`:

- Q1: notice API (a/b/c).
- Q2: `CommandResult::Overlay` variant vs. host shortcut.
- Q3: approval `Modal` as `Overlay::Approval` vs. separate slot.
- Q4: new `Event::ConnectionChanged` variant in `huginn_core` (touches
  `ConnectionsService`, not just presentation).
- Q5: rename `committed` (cosmetic) vs. preserve.
