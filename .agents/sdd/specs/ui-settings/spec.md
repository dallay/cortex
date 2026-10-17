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

- **Given** the chat-mode editor contains the text `/settings   ` (trailing spaces)
- **When** the user presses Enter
- **Then** the settings modal opens
- **And** no prompt is sent to the model

### REQ-02 — Modal structure

The system SHALL render a bordered modal with three regions:

1. A single-line tab header showing the five tab names (`theme`, `indicators`, `sound`, `toasts`, `integrations`). The active tab SHALL be rendered with inverted or underlined style; inactive tabs SHALL render in default style.
2. A body listing the items of the active tab in the format `▶ name ✓` where `▶` marks the currently selected item (one row per tab) and `✓` marks the working-copy value. Only rows whose value matches the working copy carry the `✓`; the selected row (the row with `▶`) MAY or MAY NOT carry `✓` (they are independent concerns).
3. A one-line footer with the literal text `↑↓ select · ←→ section · ↵ apply · esc close`.

#### Scenario: modal renders header body and footer

- **Given** the settings modal is open on the `theme` tab with `one-dark` as the working copy
- **When** the next frame is drawn
- **Then** the tab header shows `theme indicators sound toasts integrations` with `theme` highlighted
- **And** the body lists all 13 themes as rows in the documented order with `▶ one-dark ✓` on the first row
- **And** the footer shows `↑↓ select · ←→ section · ↵ apply · esc close`

### REQ-03 — Tab navigation

The system SHALL switch the active tab on `←`, `→`, `Tab`, and `BackTab`. Wrap-around is permitted: pressing `→` on the last tab returns to the first; pressing `←` on the first tab returns to the last. When the active tab changes, the selected item resets to index `0` unless the new tab was previously visited in this dialog session and still has a remembered selection (implementation choice — the **observable contract** is that the body shows the items of the new tab with exactly one `▶` row, and `Esc` still discards the entire working copy).

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

The system SHALL maintain a working copy of the values for every tab inside the dialog. Selection movements (`↑/↓`, `←/→` on cycle pickers, tab change) SHALL NOT persist to disk. The `✓` marker SHALL reflect the working copy, not the persisted `Config`. The working copy is committed only on Enter (REQ-07). The working copy is discarded on Esc (REQ-08).

#### Scenario: arrow down does not persist

- **Given** the settings modal is open on `theme` with `one-dark` selected
- **And** the previous saved value was `one-dark`
- **When** the user presses `↓` to move selection to `one-light`
- **Then** the persistence closure is NOT called
- **And** on disk, `[ui.theme].name` remains `one-dark`
- **And** the `✓` still appears on `one-dark` (working copy unchanged)

### REQ-07 — Apply on Enter

When the user presses Enter, the system SHALL project the working copy into a `UiConfig` value and invoke the injected persistence closure `Fn(&UiConfig) -> anyhow::Result<()>` exactly once.

- On success: the modal closes and the system shows the notice `Settings saved`.
- On failure: the modal stays open, the working copy is preserved, and the body shows the formatted error message.

#### Scenario: enter applies and patches config.toml

- **Given** the settings modal is open on `theme` with `catppuccin` selected
- **And** the persistence closure has not been called yet
- **When** the user presses Enter
- **Then** the persistence closure is called exactly once with `UiConfig { theme: { name: "catppuccin" }, .. }`
- **And** the modal closes
- **And** the status bar shows `Settings saved`

#### Scenario: enter with failing persistence keeps modal open

- **Given** the settings modal is open
- **And** the persistence closure returns `Err(anyhow!("disk full"))`
- **When** the user presses Enter
- **Then** the modal stays open
- **And** the modal body shows `disk full`
- **And** the user can press Esc to discard

#### Scenario: enter from indicators applies all three booleans atomically

- **Given** the settings modal is open on `indicators` with `scrollbar=on`, `breadcrumbs=off`, `line_numbers=on` (user toggled `breadcrumbs` to off)
- **When** the user presses Enter
- **Then** the persistence closure is called exactly once with a `UiConfig` whose `indicators` sub-table mirrors all three values
- **And** no partial writes occurred

### REQ-08 — Discard on Esc

When the user presses Esc, the system SHALL close the dialog, SHALL NOT invoke the persistence closure, and SHALL show the notice `Settings discarded`. The persisted file SHALL be unchanged.

#### Scenario: esc discards without touching disk

- **Given** the settings modal is open with working-copy `tokyo-night` selected
- **And** the previous saved value was `one-dark`
- **When** the user presses Esc
- **Then** the persistence closure is NOT called
- **And** the modal closes
- **And** the status bar shows `Settings discarded`
- **And** on disk, `[ui.theme].name` remains `one-dark`

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

When an approval modal opens while the settings modal is open, the system SHALL dispose of the settings dialog and render the approval modal. Once the approval resolves, the settings dialog SHALL NOT come back. The precedence chain SHALL be: `Modal > Settings > PluginDialog > completion`.

#### Scenario: approval modal preempts settings

- **Given** the settings modal is open on `sound`
- **When** the system requests approval for a tool call
- **Then** the settings dialog is disposed
- **And** the approval modal renders in its place
- **And** once the approval resolves, the settings dialog does NOT come back

#### Scenario: plugin dialog arrives while settings is open

- **Given** the settings modal is open on `theme`
- **When** a plugin command returns a `Dialog` result
- **Then** the settings dialog is disposed (per the precedence chain)
- **And** the plugin dialog renders in its place

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

- [ ] `cargo test -p huginn-presentation --test settings` passes (state machine: tabs, selections, apply, esc-not-mutate).
- [ ] `cargo test -p huginn-presentation --test pty` adds a render test for the header + items + marker + footer.
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
