# ui-settings — Delta Specification

> **Change**: `settings-modal`
> **Capability**: `ui-settings` (new)
> **Phase**: 3 — Spec
> **Artifact store**: `.agents/sdd/`

This is a delta spec for the new capability `ui-settings`. No other capability is modified.

## Purpose

Provide a host-owned, in-TUI settings modal reachable through the `/settings` command. The modal lets the user inspect and pick preferences across five tabs (`theme`, `indicators`, `sound`, `toasts`, `integrations`), persists selections to `config.toml` under a new `[ui]` section on Enter, and discards the working copy on Esc — without touching any runtime subsystem (theme renderer, sound, toast, indicator, provider subsystems). v1 contract is **shell + persistence only**; subscribers are follow-up changes.

**Owner**: presentation crate (`crates/huginn/presentation/src/settings.rs`) + binary helper (`apps/huginn/src/ui_settings.rs`).

---

## Requirements

### REQ-01 — `/settings` command registration

The system SHALL register a `/settings` command in the chat-mode `/`-completion catalog with the description `TUI settings`. When the user submits `/settings`, the system SHALL open the settings modal and SHALL NOT send `/settings` (or any argument) to the model as a prompt.

#### Scenario: open settings dialog from chat

- **Given** the user is in chat mode with the editor empty
- **When** the user types `/settings` and presses Enter
- **Then** the settings modal opens on the `theme` tab with `one-dark` selected and `✓`
- **And** the editor is cleared
- **And** no prompt is sent to the model

#### Scenario: `/settings` with trailing whitespace still opens the dialog

- **Given** the chat-mode editor contains the text `"/settings   "` (three trailing spaces inside the quotes; quotes are not part of the input)
- **When** the user presses Enter
- **Then** the settings modal opens
- **And** no prompt is sent to the model

### REQ-02 — Modal structure

The system SHALL render a bordered modal with three regions:

1. The one-line header SHALL show the five tabs (`theme`, `indicators`, `sound`, `toasts`, `integrations`) when they fit. If they do not fit, it SHALL show the active tab and its position, for example `integrations (5/5)` in 20 columns. The active tab SHALL use an inverted or underlined style.
2. A body listing the items of the active tab in the format `▶ name ✓` where `▶` marks the currently selected item (one row per tab) and `✓` marks the working-copy value. Only rows whose value matches the working copy carry the `✓`; the selected row (the row with `▶`) MAY or MAY NOT carry `✓` (they are independent concerns).
3. The dialog SHALL indicate that Space modifies the value, `↑/↓` moves the cursor, `Tab` changes section, Enter saves, and Esc discards. If the instructions do not fit on one line, it SHALL spread them over several lines without hiding the Space action in the 20×8 viewport.

#### Scenario: modal renders header body and footer

- **Given** the settings modal is open on the `theme` tab with `one-dark` as the working copy
- **When** the next frame is drawn
- **Then** the tab header shows `theme indicators sound toasts integrations` with `theme` highlighted
- **And** the body lists all 13 themes as rows in the documented order with `▶ one-dark ✓` on the first row
- **And** the instructions show Space to modify the value, plus selection, section change, saving, and discard.

### REQ-03 — Tab navigation

The system SHALL always switch the active tab on `Tab` and `BackTab`. When a toast picker is focused, `←` and `→` change its selected value without switching tabs. For other controls, `←` and `→` switch tabs as before. Wrap-around is permitted: tab navigation forward on the last tab returns to the first; tab navigation backward on the first tab returns to the last. When the active tab changes, the selected item resets to index `0` unless the new tab was previously visited in this dialog session and still has a remembered selection (implementation choice — the **observable contract** is that the body shows the items of the new tab with exactly one `▶` row, and `Esc` still discards the entire working copy).

#### Scenario: tab cycles to indicators with right arrow

- **Given** the settings modal is open on `theme`
- **When** the user presses `→`
- **Then** the active tab becomes `indicators`
- **And** the body shows the three boolean rows with `✓` on each (defaults are ON)
- **And** the selected row is the first one (`scrollbar`)

#### Scenario: tab cycles to integrations with Tab

- **Given** the settings modal is open on `toasts`
- **When** the user presses `Tab`
- **Then** the active tab becomes `integrations`
- **And** the body shows an empty list plus the row `▶ (+) Add provider…` selected

#### Scenario: wrap-around from last to first with right arrow

- **Given** the settings modal is open on `integrations`
- **When** the user presses `→`
- **Then** the active tab becomes `theme`
- **And** the per-tab selection on `theme` is `0` (`one-dark`)

### REQ-04 — Item navigation per tab

The system SHALL move the per-tab selection on `↑` and `↓`. Wrap-around is permitted within the current tab. The selection SHALL NOT persist across tab changes unless the user explicitly commits it with Enter.

#### Scenario: arrow down moves within theme

- **Given** the settings modal is open on `theme` with `one-dark` selected
- **When** the user presses `↓`
- **Then** the next theme (`one-light`) is marked with `▶`
- **And** the `✓` stays on `one-dark` (the working-copy value)

#### Scenario: arrow up wraps to last item

- **Given** the settings modal is open on `theme` with `one-dark` selected
- **When** the user presses `↑`
- **Then** the last item (`solarized-light`) is marked with `▶`

### REQ-05 — Tab content (concrete items per tab)

The system SHALL render the following concrete items per tab. Each item's working copy carries `✓` when its value matches the documented default below.

- **theme** — 13 single-select rows, persisted as `name`:
  `one-dark` (✓ default), `one-light`, `catppuccin`, `catppuccin-latte`, `tokyo-night`, `tokyo-night-day`, `dracula`, `nord`, `gruvbox`, `gruvbox-light`, `terminal`, `solarized`, `solarized-light`.

- **indicators** — 3 boolean rows (each `✓` when ON, blank when OFF; all ON by default):
  `scrollbar`, `breadcrumbs`, `line_numbers`.

- **sound** — 1 boolean row (default OFF):
  `bell`.

- **toasts** — 2 cycle pickers:
    - `level` cycles in order: `info` (✓ default) → `warn` → `error` → `info`.
    - `duration_secs` cycles in order: `3` (✓ default) → `5` → `10` → `off` → `3`.
      The rendered row shows the current cycle value, e.g. `level: info` and `duration: 3s`. `→` on a cycle picker row advances the cycle; `←` moves backward (or, implementation choice, also advances — the **observable contract** is that `↵` on a cycle row commits the currently displayed value).

- **integrations** — an empty list, followed by the single row `(+) Add provider…`. No other integrations rows are rendered in v1.

#### Scenario: theme tab default is one-dark

- **Given** no prior `config.toml` exists (defaults are loaded)
- **When** the user opens `/settings`
- **Then** the `theme` tab is active
- **And** the body shows 13 rows
- **And** the first row is `▶ one-dark ✓`

#### Scenario: indicators tab has three booleans all on by default

- **Given** no prior `config.toml` exists
- **When** the user switches to the `indicators` tab
- **Then** the body shows three rows: `▶ scrollbar ✓`, `breadcrumbs ✓`, `line_numbers ✓`

#### Scenario: toasts tab level and duration cycle pickers

- **Given** the user is on the `toasts` tab with default working copy
- **When** the modal renders
- **Then** the body shows: `▶ level: info ✓`, `duration: 3s ✓`
- **When** the user presses `→` on the level row
- **Then** the level row shows `level: warn`
- **And** the `✓` no longer appears on `info` in the same session

### REQ-06 — Working copy

Space SHALL modify the value of the selected row in the working copy: it
chooses the theme, toggles a boolean, or advances a cyclic selector. Moving
the cursor with `↑/↓` SHALL NOT modify that value.

The system SHALL maintain a working copy of the values for every tab inside the dialog. Selection movements (`↑/↓`, `←/→` on cycle pickers, tab change) SHALL NOT persist to disk. The `✓` marker SHALL reflect the working copy, not the persisted `Config`. The working copy is committed only on Enter (REQ-07). The working copy is discarded on Esc (REQ-08).

#### Scenario: arrow down does not persist

- **Given** the settings modal is open on `theme` with `one-dark` selected
- **And** the previous saved value was `one-dark`
- **When** the user presses `↓` to move selection to `one-light`
- **Then** the persistence closure is NOT called
- **And** on disk, `[ui.theme].name` remains `one-dark`
- **And** the `✓` still appears on `one-dark` (working copy unchanged)

### REQ-07 — Apply on Enter

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:17ac322e423a66a5421da2a7614fba99186516a48de75a021180837a9058e97b -->

When Enter is pressed for a non-stale Apply, the system SHALL capture the working copy into
`UiConfig` and invoke the injected persistence function
`Fn(&UiConfig) -> anyhow::Result<()>` exactly once per Apply, without blocking
input. While that Apply is pending, repeated Enter SHALL NOT start another
write. Editing SHALL remain available while Apply is pending; completion SHALL
preserve any newer edits in the working copy. Saves SHALL be serialized to
preserve their order. A serialized write SHALL include processing the receipt
of the previous write and publishing its saved revision before the next queued
write starts. Each Settings instance SHALL track the saved snapshot revision on
which its working copy is based. Before accepting Apply and again
before starting its write, the system SHALL reject a copy whose base revision
is stale, without invoking persistence, and show an error in the body while
preserving its edits. Reopening Settings SHALL load the latest saved snapshot.
On success it SHALL
update the snapshot of saved preferences for later openings; only if the
overlay and the Apply are still current and the working copy has no newer
edits SHALL it close `Overlay::Settings` and
show `Settings saved` as a non-conversational `Notification`. If the overlay and Apply are still current and newer edits
exist, it SHALL keep Settings open without that notice, preserve those edits,
and advance that instance's base revision to the successful saved snapshot so
the next Apply can save them. On a current failure it SHALL keep it open, preserve the working copy including newer edits,
and show the error in the body without a second equivalent notice. A late receipt SHALL be
consumed without reopening Settings, closing or modifying another overlay,
recovering focus, or affecting Approval; a late success SHALL update only the
saved snapshot and its revision, not the editable copy or base revision of
another Settings already open. A late
failure SHALL NOT replace the saved snapshot with failed values. It SHALL
preserve the joint application of all values and the TOML contracts of REQ-09
to REQ-11; it SHALL NOT apply the live theme.

<!-- /coderabbit-ai-generated:v1 -->

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

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:270a98dcded4de20dfbdae565def97750037c9fcffed3ce2caf4d2035db12468 -->

#### Scenario: SET-07-E — Edits during Apply survive completion

- GIVEN a current Apply with controlled persistence pending
- WHEN the user changes the working copy after submission and the write succeeds
- THEN the saved snapshot contains the submitted values
- AND Settings stays open with the newer edits and without `Settings saved`
- AND a subsequent Enter can apply those newer edits against the updated snapshot
- AND in a separate failure case, Settings preserves the newer edits and shows the error in the body

#### Scenario: SET-07-F — Late success makes another Settings copy stale

- GIVEN a started Apply, its Settings closed, and another Settings opened before its receipt arrives
- WHEN the earlier write succeeds and updates the saved snapshot
- THEN the open Settings keeps its editable copy and original base revision
- AND Enter is rejected with an error in the body and no persistence invocation
- AND any Apply accepted before that receipt is rejected before its write starts if its base revision is now stale
- AND reopening Settings loads the successful saved snapshot and allows Apply

### REQ-08 — Discard on Esc

<!-- /coderabbit-ai-generated:v1 -->

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

### REQ-09 — TOML round-trip

The persistence helper SHALL use `toml_edit::DocumentMut` to patch the user's existing `config.toml`. Only the `[ui.*]` sub-tables SHALL be modified by the helper. Comments, key ordering, and tables outside `[ui]` SHALL be preserved byte-for-byte across a round-trip when no other writers touch the file.

#### Scenario: unrelated tables are byte-stable after apply

- **Given** a `config.toml` with:
  ```toml
  # user comment
  [provider]
  name = "openai"
  api_key = "${OPENAI_API_KEY}"

  [model]
  default = "gpt-4o"
  ```
- **And** an empty `[ui]` table or no `[ui]` at all
- **When** the user opens `/settings`, picks `catppuccin`, and presses Enter
- **Then** the persistence helper writes the file
- **And** `[provider]` and `[model]` contents (including the `# user comment`) are byte-identical before and after
- **And** the file gains a `[ui.theme]` block with `name = "catppuccin"`

### REQ-10 — Defaults

When `config.toml` has no `[ui]` section, the system SHALL load the documented defaults. When `config.toml` has a partial `[ui]` section (for example only `[ui.theme]`), the system SHALL fill the missing sub-tables with their defaults. When `config.toml` has a complete `[ui]` section, the system SHALL load it as written.

#### Scenario: missing [ui] loads defaults

- **Given** `config.toml` has no `[ui]` section
- **When** the binary starts and calls `Config::load`
- **Then** loading succeeds
- **And** `config.ui.theme.name == "one-dark"`
- **And** `config.ui.sound.bell == false`
- **And** `config.ui.toasts.level == "info"` and `config.ui.toasts.duration_secs == 3`
- **And** `config.ui.indicators.scrollbar`, `breadcrumbs`, `line_numbers` are all `true`
- **And** `config.ui.integrations.entries` is empty

#### Scenario: partial [ui.theme] loads and fills the rest with defaults

- **Given** `config.toml` contains:
  ```toml
  [ui.theme]
  name = "dracula"
  ```
- **When** the binary starts and calls `Config::load`
- **Then** loading succeeds
- **And** `config.ui.theme.name == "dracula"`
- **And** `config.ui.sound.bell == false` (default filled in)
- **And** all other defaults are honored

### REQ-11 — Inner-unknown tolerance

The system SHALL NOT reject `Config::load` when a user has added unknown keys inside `[ui]`. This requires `UiConfig` (and its sub-structs) to NOT carry `deny_unknown_fields`, even though `Config` carries it at the top level.

#### Scenario: unknown inner key under [ui] does not break load

- **Given** a `config.toml` with:
  ```toml
  [provider]
  name = "openai"
  [ui]
  unknown_user_key = true
  [ui.theme]
  name = "dracula"
  ```
- **When** the binary starts and calls `Config::load`
- **Then** loading succeeds
- **And** `config.ui.theme.name == "dracula"`
- **And** the unknown key is preserved if the user later saves via the dialog

#### Scenario: unknown inner key does not clobber known fields on round-trip

- **Given** the loaded `config.toml` from the previous scenario
- **When** the user opens `/settings`, picks `catppuccin`, and presses Enter
- **Then** the file written by the helper still contains `unknown_user_key = true`
- **And** the file's `[provider]` block is unchanged

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

### REQ-13 — `Esc` never cancels an in-flight turn

Pressing `Esc` inside the settings modal SHALL close the settings dialog (per REQ-08) and SHALL NOT invoke the cancel-turn branch. The dispatch arm for settings returns `true` to consume the key event before any cancel-turn logic can run.

#### Scenario: esc in settings does not cancel a running turn

- **Given** the model is currently streaming a response to the user
- **And** the settings modal is open
- **When** the user presses Esc
- **Then** the modal closes
- **And** the running turn is NOT cancelled (the stream continues to completion)
- **And** the status bar shows `Settings discarded`

### REQ-14 — `(+) Add provider…`

When the user selects `(+) Add provider…` in `integrations` and presses Enter, the system SHALL show the notice `Add provider not implemented yet`. The system SHALL NOT mutate any state, SHALL NOT persist anything, and SHALL NOT read the providers table or any runtime subsystem.

#### Scenario: (+) Add provider shows notice without mutation

- **Given** the settings modal is open on `integrations`
- **And** `(+) Add provider…` is the selected row
- **When** the user presses Enter
- **Then** the modal closes
- **And** the status bar shows `Add provider not implemented yet`
- **And** the persistence closure is NOT called
- **And** no providers table is read or written

### REQ-15 — `run` mode

The system SHALL NOT open the settings modal in `run` mode (`--line-mode`). The `/settings` command SHALL NOT appear in any line-mode completion catalog. `--line-mode` users have no access to the modal.

#### Scenario: /settings is unavailable in line mode

- **Given** the user starts the binary with `--line-mode`
- **When** the user types `/settings` in the line prompt
- **Then** no modal opens
- **And** `/settings` is not in the completion list (if any)

### REQ-16 — Completion catalog

When in chat mode, the system SHALL include `/settings` in the `/`-completion catalog alongside `/help`, `/compact`, `/summarize`, and `/quit`, with description `TUI settings`. The catalog SHALL be refreshed when the model is configured (so that `/compact` and `/summarize` show when applicable) and SHALL include `/settings` regardless of model configuration state.

#### Scenario: /settings appears in completion next to /help

- **Given** the chat-mode editor is empty
- **When** the user types `/`
- **Then** the completion catalog lists `/settings` (description `TUI settings`) alongside the other host/contributed commands
- **And** selecting `/settings` from the catalog inserts it into the editor

---

## Out of scope (v1 contract)

This delta intentionally does **not** cover:

- Live theme application (persisting `[ui.theme]` does NOT switch the rendered palette).
- Live sound, toast, or indicator toggling (those values are persisted but not consumed by their subsystems in v1).
- Reading provider state for the `integrations` tab (the only interactions are the read-only empty list and the `(+) Add provider…` TODO).
- `/settings reset`, per-tab revert, or per-row edit (Esc discards the entire working copy; that's the only escape hatch).
- Migration of existing user `config.toml` files (`#[serde(default)]` on `ui` makes the new field a no-op when absent).
- An external plugin dialog for the settings body (the dialog is host-owned; only the command surface goes through the plugin catalog).
- `--line-mode` integration.

These exclusions are the canonical v1 contract — a future change that wires a persisted value into a runtime subsystem will explicitly modify this capability.

---

## Data Structures (informative)

The following types are sketched for design to consume; the spec does not lock byte-level layout beyond what REQs assert.

```rust
// crates/huginn/presentation/src/settings.rs
pub struct SettingsDialog {
    pub active_tab: usize,
    pub selections: [TabSelection; 5],
    pub working: WorkingCopy,
}

pub struct WorkingCopy {
    pub theme: ThemeWorking,
    pub indicators: IndicatorsWorking,
    pub sound: SoundWorking,
    pub toasts: ToastsWorking,
    pub integrations: IntegrationsWorking,
}

// apps/huginn/src/config.rs
#[derive(Deserialize, Default)]
#[serde(default)] // NOT deny_unknown_fields — tolerates hand-added keys
pub struct UiConfig {
    pub theme: UiTheme,
    pub sound: UiSound,
    pub toasts: UiToasts,
    pub indicators: UiIndicators,
    pub integrations: UiIntegrations,
}
```

```rust
// apps/huginn/src/ui_settings.rs
pub fn persist(ui: &UiConfig, path: &Path) -> anyhow::Result<()>;
```

The `apps/huginn` binary builds the closure `Box<dyn Fn(&UiConfig) -> anyhow::Result<()>>` that delegates to `ui_settings::persist` and passes it into `Ui::new` (or `Ratatui::with_ui_config`).

---

## Acceptance Criteria

- [ ] `cargo test -p huginn-presentation --test settings --test state` passes (state machine: tabs, selections, apply, esc-not-mutate, and input dispatch).
- [ ] `cargo test -p huginn-presentation --features test-driver --test pty` adds a render test for the header + items + marker + footer.
- [ ] `cargo test -p huginn-presentation --test plugins` confirms `/settings` appears in the catalog with description `TUI settings` alongside `/help`, `/compact`, `/summarize`, `/quit`.
- [ ] `cargo test -p huginn --test config_compat` (or a new `tests/ui_settings.rs`) loads a fixture without `[ui]`, applies defaults; loads a partial `[ui]`, fills defaults; loads a fixture with inner unknowns, loads without error.
- [ ] A round-trip test loads a fixture with `[provider]`, `[model]`, and a hand-written comment, applies `[ui.theme] = "catppuccin"`, and asserts the bytes outside `[ui]` are byte-identical.
- [ ] `/settings` clears the editor without invoking the prompt sender (test asserts no `try_send` call).
- [ ] Esc in settings closes the dialog, shows `Settings discarded`, and does NOT cancel the running turn (test asserts the cancel-turn branch is skipped).
- [ ] Enter invokes the closure exactly once with the projected `UiConfig`; success closes and shows `Settings saved`; failure keeps the dialog open with the error visible and Esc still discards.
- [ ] `(+) Add provider…` shows the notice and does not invoke the closure.
- [ ] Approval modal arriving during settings disposes settings; plugin dialog arriving during settings disposes settings (per the precedence chain).
- [ ] `/settings` is unreachable in `--line-mode` (no catalog entry, no modal dispatch).
- [ ] `apps/huginn/config.example.toml` documents the `[ui]` schema mirroring D1.
- [ ] `just ci-local` is green.
