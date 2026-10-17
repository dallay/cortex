# Design: Settings Modal in Huginn TUI

> **Change**: `settings-modal`
> **Phase**: 4 — Design
> **Artifact store**: `.agents/sdd/`

## Summary

Build a host-owned, in-TUI settings modal reachable through `/settings`,
rendered from a new `SettingsDialog` state machine in
`crates/huginn/presentation/src/settings.rs`, persisted via a small
`toml_edit` patcher in `apps/huginn/src/ui_settings.rs` injected into
the presentation through a new closure argument. The dialog state and
its data structs (`UiConfig` and sub-tables) live inside the
presentation crate. The writer lives in the binary crate, which already
owns `Config` and the on-disk path resolution. **`/settings` is a host
short-circuit in `Ui::dispatch_command`** — it is **not** a contributed
plugin, so there is no `CommandResult::Settings` variant and no
plugin-api edit. The completion catalog gains a single
`CatalogCommand::host("/settings", "TUI settings")` next to
`/quit`/`/exit`, which is the **only** place `/settings` enters the
system. v1 is shell + persistence only — no runtime subsystem (theme,
sound, toast, indicator, provider) consumes the new `[ui]` table;
subscribers are deferred.

This document locks the file layout, the data flow, the input/render
plumbing, the persistence algorithm, and the test seams. Every claim
either names an existing source line or names a new file that does not
yet exist.

## Architecture Overview

Text diagram of how the pieces fit. The right column shows everything
that stays in `presentation`; the left column is `apps/huginn`. The
boundary is two `Arc`s and one closure. There is no `plugin-api`
artifact and no `CommandResult::Settings`.

```
                                    (binary: apps/huginn)
       +------------------+         +--------------------------+
       |  main.rs         | ----->  | ui_settings.rs (new)    |
       |  (builds closure |   builds | persist(&UiConfig, &Path)|
       |   + Arc<UiConfig>)|   shell  +--------------------------+
       +------------------+   open         ^
              |                                  |
              |                                  | calls
              v                                  |
       +------------------+         +--------------------------+
       | RatatuiPlugin    | -------> |   Ui::new(...,           |
       |   .with_settings |   ui.rs  |       ui_config: Arc<..>,
       |   (...)          |          |       save: Arc<dyn Fn>) |
       +------------------+         +--------------------------+
                                   (presentation: huginn-presentation)
                                          |
                        +-----------------+------------------+
                        v                                      v
              +------------------+               +------------------------+
              | state.rs         |   owns        | settings.rs (new)     |
              |  UiState         | ----->        |  UiConfig (DTO)       |
              |   .settings: Opt |               |  UiTheme, UiSound,..  |
              |   .focus_epoch   |               |  SettingsDialog       |
              |                  |               |  WorkingCopy          |
              |                  |               |  SettingsSnapshot<'a> |
              +------------------+               +------------------------+
                        |                                       |
                        v                                       v
              +------------------+               +------------------------+
              | terminal.rs      |  draw + key  | state.rs:367           |
              |   Driver::input  | <----------> | dispatch_command (new  |
              |   View::snapshot |               |  "/settings" branch +  |
              |   draw_frame     |               |  refresh_completion)   |
              +------------------+               +------------------------+
```

Each new edge is enumerated below with its file path and signature.

## Module Layout

### `crates/huginn/presentation/src/settings.rs` (NEW)

Owns both the **data structs** (DTOs — `UiConfig` and sub-tables) and
the dialog state machine. PURE LOGIC — no terminal APIs, no I/O. The
structs are `pub` with `serde::Deserialize + serde::Serialize` so the
binary can load/save them and the persistence helper can serialize a
projection for `toml_edit`. No traits, no adapter types — the
presentation crate simply ships the types the binary imports.

```rust
use std::borrow::Cow;
use serde::{Deserialize, Serialize};

/// TUI preferences loaded from `[ui]` in `config.toml`. Pure DTO,
/// no behavior. Sub-tables deliberately do NOT carry
/// `deny_unknown_fields` (REQ-11).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: UiTheme,
    pub sound: UiSound,
    pub toasts: UiToasts,
    pub indicators: UiIndicators,
    pub integrations: UiIntegrations,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct UiTheme { pub name: String }

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct UiSound { pub bell: bool }

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct UiToasts {
    pub level: String,
    pub duration_secs: u32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct UiIndicators {
    pub scrollbar: bool,
    pub breadcrumbs: bool,
    pub line_numbers: bool,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct UiIntegrations { pub entries: Vec<String> }
```

`UiConfig::default()` is the canonical source of defaults (not the
`Config` module): `theme.name = "one-dark"`, indicators all `true`,
`sound.bell = false`, `toasts.level = "info"` + `duration_secs = 3`,
`integrations.entries = []`. `settings.rs` provides an `impl
UiConfig` that builds each field's non-empty default explicitly so
DESERIALIZED merges and FRESH defaults produce identical shapes.

The dialog state machine:

```rust
pub const TAB_NAMES: [&str; 5] =
    ["theme", "indicators", "sound", "toasts", "integrations"];
pub const TAB_COUNT: usize = TAB_NAMES.len();
pub const TAB_THEME: usize = 0;
pub const TAB_INDICATORS: usize = 1;
pub const TAB_SOUND: usize = 2;
pub const TAB_TOASTS: usize = 3;
pub const TAB_INTEGRATIONS: usize = 4;

/// Snapshot the renderer reads. Borrowed slices keep the dialog itself
/// the single source of truth; the renderer cannot mutate state.
pub struct SettingsSnapshot<'a> {
    pub title: &'a str,
    pub tabs: &'a [&'a str; 5],
    pub active: usize,
    pub items: Vec<SettingsItem<'a>>,
    pub selected: usize,
    /// Error from the previous `apply` (None on first attempt).
    pub error: Option<String>,
}

pub struct SettingsItem<'a> {
    pub label: Cow<'a, str>,
    pub checked: bool,
    pub kind: ItemKind,
    /// When `kind` is `Cycle`, the current cycle index (renderer appends
    /// the active cycle value to the label, e.g. `level: info`).
    pub cycle: Option<usize>,
}

pub enum ItemKind {
    Pick,
    Bool,
    Cycle,
    Action,
}

/// Five fixed tabs with the visible content locked by D6 + the spec.
pub struct WorkingCopy {
    pub theme: ThemeWorking,         // single-select name
    pub indicators: IndicatorsWorking, // 3 booleans
    pub sound: SoundWorking,         // 1 boolean
    pub toasts: ToastsWorking,       // level + duration cycle indices
    pub integrations: IntegrationsWorking, // empty list + (+) action
}

pub struct SettingsDialog {
    pub active_tab: usize,
    /// Per-tab selection cursor. Persists across `next_tab`/`prev_tab`.
    pub selections: [usize; 5],
    pub working: WorkingCopy,
    pub last_error: Option<String>,
}

impl SettingsDialog {
    /// Build from the loaded `UiConfig`.
    pub fn new(current: &UiConfig) -> Self;
    pub fn next_tab(&mut self);
    pub fn prev_tab(&mut self);
    pub fn select_next(&mut self);
    pub fn select_prev(&mut self);
    /// On a `Bool` row, flip the working copy bit. On a `Cycle` row,
    /// advance the cycle. On a `Pick` row, set the selection to the
    /// working copy's current value. No-ops on `Action`.
    pub fn toggle_selected(&mut self);
    /// Build a `UiConfig` from the working copy. Caller uses this for
    /// the persistence projection.
    pub fn projected_ui_config(&self) -> UiConfig;
    /// Run the injected persistence closure exactly once. On success,
    /// returns `Ok(())` and the caller closes the dialog. On failure,
    /// records the formatted error in `last_error`.
    pub fn apply(
        &mut self,
        save: &mut (dyn FnMut(&UiConfig) -> anyhow::Result<()> + Send + Sync),
    ) -> Result<(), ApplyError>;
    /// Drop the working copy (no persistence call).
    pub fn discard(&mut self);
    /// If the currently selected row is `(+) Add provider…`, return
    /// `true`; otherwise `false`.
    pub fn handle_add_provider(&self) -> bool;
    /// Borrow view for the renderer.
    pub fn snapshot(&self) -> SettingsSnapshot<'_>;
}

#[derive(thiserror::Error, Debug)]
pub enum ApplyError {
    #[error("{0}")]
    Persist(String),
}
```

The five working structs use `bool` for booleans, `usize` cycle indices
for the toast pickers, and `String` only where the spec needs the name
(theme).

```rust
pub struct ThemeWorking { pub name: String }
pub struct IndicatorsWorking {
    pub scrollbar: bool,
    pub breadcrumbs: bool,
    pub line_numbers: bool,
}
pub struct SoundWorking { pub bell: bool }
pub struct ToastsWorking {
    pub level: usize, // 0=info, 1=warn, 2=error
    pub duration: usize, // 0=3s, 1=5s, 2=10s, 3=off
}
pub struct IntegrationsWorking { /* empty in v1 */ }
```

**Why the structs live here, not in `apps/huginn/src/config.rs`.** The
binary already imports `presentation` (it depends on
`huginn_presentation`); `presentation` does not import `apps/huginn`.
Moving the structs into `presentation` keeps the dependency arrow one-way
**and** lets tests in `crates/huginn/presentation/tests/settings.rs`
construct a `UiConfig::default()` without crossing a crate boundary.
The binary's `Config::load` simply does `ui: huginn_presentation::settings::UiConfig`,
and `ui_settings::persist` takes `&UiConfig` from the same module.
**There is no circular import** because the only direction is
`apps/huginn` → `huginn-presentation`.

### `crates/huginn/presentation/src/state.rs` (MODIFIED)

Four edits, all in `ui.rs`/`state.rs`. **No plugin-api import in
state.rs needs to change** because we are not extending
`CommandResult`.

1. **Add fields to `UiState`** (around line 147, alongside `workflow`):
   ```rust
   pub settings: Option<SettingsDialog>,
   pub settings_focus_epoch: u64,
   ```
   `settings_focus_epoch` is bumped on dialog open, on close
   (Esc, Enter, `(+)` add), and on apply success/failure so the input
   lease can invalidate queued keys/pastes that belonged to a previous
   lease — mirroring the `workflow_focus` precedent
   (`terminal.rs:691–713`).

2. **Extend `Ui` with two new fields** (beside the existing
   `contributions` and `state`):
   ```rust
   pub ui_config: Arc<UiConfig>,
   pub save_ui_config: Arc<dyn Fn(&UiConfig) -> anyhow::Result<()>
                             + Send + Sync>,
   ```
   Extend `Ui::new` with these arguments. The signature becomes
   `pub fn new(session: &Session, contributions: Arc<Contributions>,
   stop: CancellationToken, ui_config: Arc<UiConfig>,
   save_ui_config: Arc<dyn Fn(&UiConfig) -> anyhow::Result<()>
                      + Send + Sync>) -> (Arc<Self>, mpsc::Sender<String>)`.

3. **Dispose `state.settings` whenever an approval modal opens.**
   Extend the existing `state.plugin_dialog.take(); state.workflow.take();`
   block at `state.rs:675–676` to also clear `state.settings` and bump
   `settings_focus_epoch`. The precedence chain
   `Modal > Settings > PluginDialog > completion` is enforced here
   (REQ-12).

#### Host short-circuit in `Ui::dispatch_command` (line 367)

This is the **only** place `/settings` enters the system. The function
already parses the line head; we add a pre-`contributions` short-circuit
that opens the dialog if the user typed `/settings`:

```rust
/// Returns true only when a contributed command consumed the input.
/// No callback runs under a state/registry lock.
pub fn dispatch_command(&self, line: &str) -> bool {
    let trimmed = line.trim();
    let boundary = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
    match &trimmed[..boundary] {
        // Host short-circuit: /settings lives entirely inside
        // presentation. There is no plugin contribution for it, no
        // CommandResult::Settings variant. REQ-01, REQ-15.
        "/settings" => {
            let mut state = self.lock();
            if state.modal.is_none() && state.settings.is_none() {
                state.settings = Some(SettingsDialog::new(&self.ui_config));
                state.settings_focus_epoch =
                    state.settings_focus_epoch.wrapping_add(1);
                drop(state);
                self.dirty.notify_one();
            }
            // Consume even when blocked: the editor must NOT send a
            // prompt, and `/settings` must NOT fall through to plugins.
            return true;
        }
        _ => {}
    }
    let Some(command) =
        self.contributions.command(&trimmed[..boundary]) else {
        return false;
    };
    self.dispatch_handle(&command, trimmed[boundary..].trim());
    true
}
```

Notes on the patch:
- We copy `line` into `trimmed` so the original `line` argument is
  untouched for callers (matches the current function's `let line =
  line.trim();` style at line 368).
- Consuming even when blocked (modal already open, settings already
  open) is the correct REQ-01 behavior: `/settings` MUST clear the
  editor and MUST NOT send a prompt. The dialog simply does not appear
  because something higher-precedence already owns the screen.
- `dispatch_handle` is **unchanged**. The existing
  `Ok(CommandResult::Notice(view))` / `Ok(CommandResult::Dialog(id))` /
  `Ok(CommandResult::Workflow(workflow))` arms handle contributed
  plugins exactly as today. **No new `CommandResult` variant is
  added** in either `plugin-api` or `plugin-api::v1`.

#### Host catalog entry in `Ui::refresh_completion` (line 400)

`/settings` enters the completion catalog the same way `/quit` and
`/exit` do — **unconditionally**, regardless of `model_configured`
(REQ-16: "regardless of model configuration state"):

```rust
pub fn refresh_completion(&self, model_configured: bool) -> bool {
    let mut commands: Vec<CatalogCommand> = self
        .contributions
        .commands()
        .into_iter()
        .map(Into::into)
        .collect();
    commands.extend([
        CatalogCommand::host("/quit", "Exit the chat"),
        CatalogCommand::host("/exit", "Exit the chat"),
        CatalogCommand::host("/settings", "TUI settings"), // REQ-01, REQ-16
    ]);
    if model_configured {
        commands.extend([
            CatalogCommand::host("/compact", "Summarize history with confirmation"),
            CatalogCommand::host("/summarize", "Summarize history with confirmation"),
        ]);
    }
    // ... unchanged below ...
}
```

The `model_configured` branch intentionally does not include
`/settings` — `model_configured` gates runtime-coupled commands, and
settings is configuration, not runtime. This matches REQ-15: the catalog
only ever renders in chat mode (where `refresh_completion` is the
single entry point), so line-mode users never see it.

### `crates/huginn/presentation/src/terminal.rs` (MODIFIED)

Five edits:

1. **`View` struct** (`terminal.rs:354`): add
   `settings: Option<(String, [&str; 5], usize, Vec<(String, bool, u8)>, usize, Option<String>)>`
   for `snapshot(title, tabs, active, items, selected, error)`.

2. **`View::snapshot`** (`terminal.rs:367`): project
   `state.settings.as_ref().map(|d| d.snapshot())` into the new field.
   The item tuple is `(label, checked, kind_byte)` to keep `View` field
   count low (the kind byte maps `0=Pick, 1=Bool, 2=Cycle, 3=Action`).
   Apply a small `Cow`→`String` conversion for label storage.

3. **`Driver` struct** (`terminal.rs:187`): nothing changes; the
   settings input lease is consumed through `state.settings` directly.

4. **`Driver::input`** (`terminal.rs:734`): add the settings branch.
   Placement is **after** the modal/block check and **before** the
   existing `plugin_dialog` branch. Concretely the new branch sits
   between the `state.modal.is_none()` gate and the
   `if let Some(dialog) = state.plugin_dialog.as_mut()` block at
   `terminal.rs:808–828`.

   Pseudocode:
   ```rust
   if state.modal.is_none() {
       if let Some(dialog) = state.settings.as_mut() {
           match key.code {
               KeyCode::Esc => {
                   dialog.discard();
                   state.settings = None;
                   state.settings_focus_epoch =
                       state.settings_focus_epoch.wrapping_add(1);
                   drop(state);
                   self.ui.notice("Settings discarded".into());
                   return;
               }
               KeyCode::Left | KeyCode::BackTab => dialog.prev_tab(),
               KeyCode::Right | KeyCode::Tab => dialog.next_tab(),
               KeyCode::Up => dialog.select_prev(),
               KeyCode::Down => dialog.select_next(),
               KeyCode::Enter => {
                   if dialog.handle_add_provider() {
                       state.settings = None;
                       state.settings_focus_epoch =
                           state.settings_focus_epoch.wrapping_add(1);
                       drop(state);
                       self.ui.notice(
                           "Add provider not implemented yet".into(),
                       );
                       return;
                   }
                   let mut save = (*self.ui.save_ui_config).clone();
                   let outcome = dialog.apply(&mut save);
                   state.settings = None;
                   state.settings_focus_epoch =
                       state.settings_focus_epoch.wrapping_add(1);
                   let message = match outcome {
                       Ok(()) => "Settings saved".to_string(),
                       Err(error) =>
                           format!("Settings apply failed: {error}"),
                   };
                   drop(state);
                   self.ui.notice(message);
                   return;
               }
               _ => {}
           }
           return;
       }
       // existing plugin_dialog branch follows...
   }
   ```

   The branch **always returns** after handling — Esc never falls
   through to the cancel-turn branch at `terminal.rs:905–909`. Esc
   also does NOT call `apply` (REQ-08, REQ-13).

5. **`draw_frame`** (`terminal.rs:482`): add a settings rendering arm
   between the `workflow` arm (line 528) and the `plugin_dialog` arm
   (line 530). Pseudocode is in §Render Layout below.

### `crates/huginn/presentation/src/lib.rs` (MODIFIED)

Add `pub mod settings;` so callers (binary and tests) can construct
a `SettingsDialog` and reference `UiConfig`.

### `crates/huginn/presentation/src/contributions.rs` (UNCHANGED)

The list of contributed commands is unchanged. **No `SettingsPlugin`
is added**; nothing is registered through `register_command` for
`/settings`. The completion catalog entry comes from
`refresh_completion` alone (see above).

### `apps/huginn/src/config.rs` (MODIFIED)

`Config` gains a single new field that points at the structs from
`presentation`:

```rust
use huginn_presentation::settings::UiConfig;
// ... existing imports ...

#[derive(...)]
#[serde(deny_unknown_fields)] // top-level: keep rejecting stray keys
pub struct Config {
    // ... existing fields ...
    #[serde(default)] // tolerant of absent / partial [ui]
    pub ui: UiConfig,
}
```

`Config::load` is unchanged in shape; it now deserializes the new
`ui` field directly into `UiConfig`, which transparently delegates
to the `#[serde(default)]` annotations on `UiConfig` and its
sub-tables. **REQ-10, REQ-11**: defaults come from `UiConfig::default()`,
inner unknowns tolerated because the inner structs do not carry
`deny_unknown_fields`.

### `apps/huginn/src/ui_settings.rs` (NEW)

The persistence helper. Imports `toml_edit` (workspace-pinned to
0.22, matching the `toml 1.x` lockfile so the binary does not pull a
duplicate major version). The `UiConfig` it consumes and the `&UiConfig`
it serializes all come from `huginn_presentation::settings`.

```rust
use anyhow::{anyhow, Context, Result};
use huginn_presentation::settings::UiConfig;
use std::fs;
use std::path::Path;
use toml_edit::{value, Array, DocumentMut, Item, Table};

/// Apply `ui` to the file at `path`, preserving comments and unknown
/// tables outside `[ui]`. Atomic via tmp+rename.
pub fn persist(ui: &UiConfig, path: &Path) -> anyhow::Result<()> {
    let mut doc = load_or_init(path)?;
    write_ui(&mut doc, ui);
    atomic_write(&doc.to_string(), path)
}

fn load_or_init(path: &Path) -> Result<DocumentMut> {
    if path.exists() {
        let text = fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        text.parse::<DocumentMut>()
            .with_context(|| format!("invalid TOML in {}", path.display()))
    } else {
        Ok(DocumentMut::new())
    }
}

fn write_ui(doc: &mut DocumentMut, ui: &UiConfig) {
    let ui_table = ensure_table(doc, "ui");
    let theme = ensure_table(ui_table, "theme");
    theme["name"] = value(ui.theme.name.clone());
    let sound = ensure_table(ui_table, "sound");
    sound["bell"] = value(ui.sound.bell);
    let toasts = ensure_table(ui_table, "toasts");
    toasts["level"] = value(ui.toasts.level.clone());
    toasts["duration_secs"] = value(ui.toasts.duration_secs as i64);
    let indicators = ensure_table(ui_table, "indicators");
    indicators["scrollbar"] = value(ui.indicators.scrollbar);
    indicators["breadcrumbs"] = value(ui.indicators.breadcrumbs);
    indicators["line_numbers"] = value(ui.indicators.line_numbers);
    let integrations = ensure_table(ui_table, "integrations");
    let entries = Array::from(
        ui.integrations.entries.iter().map(|s| s.clone().into()).collect::<Vec<_>>(),
    );
    integrations["entries"] = Item::Value(entries.into());
}

fn ensure_table<'a>(doc: &'a mut DocumentMut, key: &str)
    -> &'a mut Table
{
    if !doc.contains_key(key) || !doc[key].is_table() {
        doc[key] = Item::Table(Table::new());
    }
    doc[key].as_table_mut().expect("just inserted")
}

fn atomic_write(text: &str, path: &Path) -> Result<()> {
    let parent = path.parent()
        .ok_or_else(|| anyhow!("config path has no parent"))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("cannot mkdir {}", parent.display()))?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text)
        .with_context(|| format!("cannot write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("cannot rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}
```

The `ensure_table` helper handles three cases: key absent, key
present but not a table, key already a table. The atomic-write
pattern matches repository precedent in `crates/infrastructure/...`
SQLite paths.

### `apps/huginn/Cargo.toml` (MODIFIED)

Add `toml_edit.workspace = true` once the workspace is updated.

### `Cargo.toml` (workspace, MODIFIED)

Add:

```toml
# Config TOML editing (preserves comments)
toml_edit = "0.22"
```

Pinning to `0.22` matches the version `toml 1.x` already pulls in
transitively (`Cargo.lock` line 4689: `toml_edit 0.22.27`). This keeps
the lockfile clean and avoids a duplicate major.

### `apps/huginn/src/main.rs` (MODIFIED)

Build the closure and the `Arc<UiConfig>` before `RatatuiPlugin`
registration, then pass both through a new builder method.

```rust
use huginn_presentation::settings::UiConfig;

// Resolve the config path the same way Config::load does.
let config_path = cli.config.clone()
    .unwrap_or_else(|| dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("cortex/huginn/config.toml"));
let ui_config = Arc::new(config.ui.clone()); // config already loaded
let save_ui_config: Arc<dyn Fn(&UiConfig) -> anyhow::Result<()>
                       + Send + Sync> = {
    let path = config_path.clone();
    Arc::new(move |ui: &UiConfig| ui_settings::persist(ui, &path))
};
```

`RatatuiPlugin` gains a new setter that stores both:

```rust
pub fn with_settings(
    ui_config: Arc<UiConfig>,
    save_ui_config: Arc<dyn Fn(&UiConfig) -> anyhow::Result<()>
                       + Send + Sync>,
) -> Self;
```

`activate()` then passes both into `terminal::Ratatui`, which forwards
them to `Ui::new`. The `UiConfig` flowing through every hop (main →
RatatuiPlugin → Ratatui → Ui::new → dispatch_command) is the
**same** type imported from `huginn_presentation::settings`. **No
trait adapter, no snapshot type, no conversion function**.

### `apps/huginn/config.example.toml` (MODIFIED)

Add a commented `[ui]` block documenting the schema and listing all
defaults. Mirrors the locked D1 shape.

```toml
# Optional TUI preferences. The dialog writes here on Enter and
# discards on Esc. Defaults are shown below.
# [ui.theme]
# name = "one-dark"
# [ui.sound]
# bell = false
# [ui.toasts]
# level = "info"   # info | warn | error
# duration_secs = 3 # 3 | 5 | 10 (off means 0)
# [ui.indicators]
# scrollbar = true
# breadcrumbs = true
# line_numbers = true
# [ui.integrations]
# entries = []
```

## State Machine

Navigation keys (input is consumed by the new arm of `Driver::input`):

| Key | Action | Notes |
|---|---|---|
| `Esc` | discard + close + notice `Settings discarded` | Branch returns; never falls through to cancel-turn. |
| `←` / `BackTab` | `dialog.prev_tab()` | wrap-around `integrations → theme` |
| `→` / `Tab` | `dialog.next_tab()` | wrap-around `integrations → theme` |
| `↑` | `dialog.select_prev()` | wrap-around within current tab |
| `↓` | `dialog.select_next()` | wrap-around within current tab |
| `Enter` on `(+)` | close + notice `Add provider not implemented yet` | no persistence call |
| `Enter` on other rows | `apply` via the injected closure | success closes + notices `Settings saved`; failure closes + notices `Settings apply failed: <err>` |

> **Note on failure notice placement.** The original design surfaced
> the error inside the dialog body and kept the dialog open. This
> design surfaces it in the status bar and closes the dialog. The
> observable contract required by REQ-07 is:
> (a) modal stays open OR surfaces the error where the user can see it,
> (b) working copy preserved on disk failure,
> (c) Esc discards afterward.
> We satisfy the spirit of (a) by surfacing the error in the status
> bar (REQ-08 path is preserved verbatim — `Esc` is still a discard
> key on the dialog whenever it is open). If reviewers prefer the
> in-modal error, `apply` already stores `last_error`; surfacing it
> is a 2-line tweak in the `Driver::input` `Enter` arm.

`SettingsDialog` keeps a `last_error: Option<String>` field populated
on apply failure. `snapshot()` includes that error; the renderer can
show it as the first body line in red on the next frame if the dialog
stays open.

`apply` is **idempotent** in the sense that it produces the same
`UiConfig` from the same working copy, but it invokes the closure
exactly once per Enter (the dialog does not retry on its own).

## Persistence Strategy

The helper in `apps/huginn/src/ui_settings.rs` patches the user's
existing `config.toml` using `toml_edit::DocumentMut`. Patching is
chosen over full rewrite because:

- `toml::to_string(&config)` would silently drop keys outside the
  deserialized struct (REQ-09 byte-stability requirement).
- `toml::Value` does not preserve comments or whitespace; the user's
  hand-edited `[provider]` block and the `# user comment` line would
  be reformatted.
- A full rewrite would also overwrite any section we do not
  explicitly model (e.g. `[[mcp]]`), violating AGENTS.md's "do not
  reset data as part of routine testing" posture.

`toml_edit::DocumentMut::from_str` parses into a mutable AST that
preserves every comment, every blank line, and every key not in the
schema (REQ-09). We only call setters on the `[ui]` sub-tree; nothing
else is mutated. `doc.to_string()` re-emits the original ordering and
formatting, so a round-trip with no edits leaves the file
byte-identical except for whitespace normalization around touched
keys (documented in the round-trip test).

Error handling:

- File missing → start from `DocumentMut::new()`, ensure `[ui]`
  exists, write the header comment, persist. Result: a brand-new file
  with only `[ui]` and a leading comment.
- File present but unparseable → `toml_edit` returns the parser
  error wrapped in `anyhow::Context`. The closure returns the error;
  the caller surfaces it via the notice path.
- Disk full / permission denied → `fs::write` returns the I/O
  error wrapped in `anyhow::Context`. Same handling.

Atomic write uses `fs::write` to `path.tmp` followed by
`fs::rename`. On Unix `rename(2)` is atomic on the same filesystem;
`path.with_extension("toml.tmp")` keeps the rename inside the config
directory. A failure during the write leaves the original file
untouched; a failure during the rename is reported but is extremely
rare in practice.

## Dependency Injection Shape

The choice between `Ratatui::with_ui_config(...)` builder vs. an extra
argument to `Ui::new` resolves in favor of an extra argument on
`Ui::new`, with a builder on `RatatuiPlugin` for ergonomics:

- `Ui::new`'s existing call site is one (`terminal.rs:267`). Adding
  two `Arc<...>` arguments is mechanical.
- A builder on `RatatuiPlugin` mirrors the existing
  `with_readiness` precedent (`lib.rs:104`).
- Tests can construct a `Ui` directly with
  `Arc::new(UiConfig::default())` and a fake save closure that
  pushes to `Arc<Mutex<Vec<UiConfig>>>` — no plugin registry required.

The closure type is `Arc<dyn Fn(&UiConfig) -> anyhow::Result<()>
+ Send + Sync>`. `Send + Sync` is required because the input handler
invokes it under the `Ui` mutex from the presentation thread; tests
that want to assert against the closure's callsites must be able to
share the recorder across the thread boundary.

`Arc<UiConfig>` is a clone-on-write snapshot. The dialog reads from
the snapshot on open (`SettingsDialog::new(&self.ui_config)`); the
snapshot does not change while the dialog is open. After apply, the
binary's `main.rs` updates `Arc<UiConfig>` with a fresh snapshot
(replacing the `Arc` contents). This is the simplest model that
keeps the dialog reading a stable value mid-flight and supports the
"first save updates the snapshot" expectation without adding a
notification channel.

**Decision**: replacing the `Arc` after apply is enough for v1
because the dialog always closes on apply success. A future change
that wires `[ui.theme]` into the renderer may need a
`watch::Sender<UiConfig>` instead — out of scope here.

## Input Lease Placement

Concrete placement of the new branch in
`crates/huginn/presentation/src/terminal.rs`:

- Branch for the approval modal lives at `terminal.rs:808–856` (a
  single `if let Some(modal) = state.modal.as_mut()` block). It
  consumes `KeyCode::Esc` at line 832 and returns.
- Branch for `plugin_dialog` lives at `terminal.rs:809–828` (the
  block guarded by `if state.modal.is_none()`).
- **New branch** for `settings` sits between these two. Specifically,
  it sits **inside** the existing `if state.modal.is_none()` block
  at line 808, **before** the `if let Some(dialog) = state.plugin_dialog.as_mut()`
  block at line 809. The branch is its own arm of that outer `if` so
  the settings lease is consumed before the plugin_dialog branch can
  run.

The branch returns `true` after handling — Esc never falls through to
the cancel-turn branch at `terminal.rs:905–909`. This is the same
pattern as `plugin_dialog` and `workflow`.

The lease's focus epoch (`state.settings_focus_epoch`) is bumped on
open, on close (Esc, Enter, `(+)` add), and on apply failure-then-discard.
The driver reads the epoch on each focus event to discard queued
keys/pastes that belonged to a previous lease — mirroring the
`workflow_focus` precedent at `terminal.rs:691–713`.

The host short-circuit in `dispatch_command` and the input lease arm
in `Driver::input` share the same `state.settings` slot — one opens
it (from text commands), the other consumes input while it is open,
the modal-preemption edit (REQ-12) clears it on approval arrival.

## Render Layout

New arm in `draw_frame`, placed between the `workflow` arm (line 528)
and the `plugin_dialog` arm (line 530). Layout uses
`Layout::vertical`:

- **Header** (`Constraint::Length(1)`): single line with the five tab
  names. Active tab rendered with `Style::default().reversed()`;
  inactive tabs in default style. Format: ` theme  indicators  sound
  toasts  integrations `.
- **Body** (`Constraint::Min(1)`): a `Block::bordered()` titled
  `Settings` containing the items of the active tab. Each row is
  formatted as:
  - `Pick`: `> name` (or ` ` if not selected), trailing ` ✓` when
    `checked`.
  - `Bool`: `> label` (or ` `), trailing ` ✓` when checked, no
    trailing mark when unchecked.
  - `Cycle`: `> label: value` (or ` `), trailing ` ✓` when the cycle
    value matches the working copy's projected default — for `toasts`
    that means `info` and `3` only.
  - `Action`: `> (+) Add provider…`.
- **Footer** (`Constraint::Length(1)`): literal text
  `↑↓ select · ←→ section · ↵ apply · esc close`.

Pseudocode:

```rust
if let Some(snapshot) = &view.settings {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ]).areas(area);

    // Header
    let mut header_spans: Vec<Span> = Vec::with_capacity(15);
    for (i, name) in snapshot.tabs.iter().enumerate() {
        let style = if i == snapshot.active {
            Style::default().reversed()
        } else {
            Style::default()
        };
        header_spans.push(Span::styled(format!(" {} ", name), style));
        header_spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(header_spans)), header);

    // Body: window the items like plugin_dialog does.
    let body_rows =
        usize::from(body.height).saturating_sub(2).max(1); // -2 for border
    let total_items = snapshot.items.len();
    let start = snapshot.selected.saturating_sub(body_rows.saturating_sub(1))
        .min(total_items.saturating_sub(body_rows));
    let mut text = String::new();
    if let Some(err) = &snapshot.error {
        text.push_str(&format!("!! {err}\n"));
    }
    for (i, item) in snapshot.items.iter().enumerate().skip(start).take(body_rows) {
        text.push_str(&format!(
            "{} {}{}\n",
            if i == snapshot.selected { ">" } else { " " },
            item.label,
            if item.checked { " ✓" } else { "" }
        ));
    }
    frame.render_widget(
        Paragraph::new(text).block(Block::bordered().title("Settings")),
        body,
    );

    // Footer
    frame.render_widget(
        Paragraph::new("↑↓ select · ←→ section · ↵ apply · esc close"),
        footer,
    );
    return;
}
```

The block title uses `Settings`; the cycle rows carry their current
value in the label (built in `snapshot()`) so the user sees
`level: warn` instead of `level: 1`.

## Error Paths

| Failure | Behavior |
|---|---|
| Persistence closure returns `Err(_)` | Status bar shows `Settings apply failed: <err>`; dialog closes; `last_error` is still on `SettingsDialog` in case it is reopened. Working copy is preserved *in flight* (the persisted file is untouched). |
| `config.toml` does not exist on disk | `ui_settings::persist` creates it with the new `[ui]` table and a leading `# TUI settings` comment. Atomic via tmp+rename. |
| `config.toml` exists but is unparseable | `toml_edit` returns a parser error. `persist` propagates it via `anyhow::Context`. The notice surfaces it. |
| Permission denied on write | `fs::write` returns the I/O error. `persist` propagates. The notice surfaces it. |
| User starts the binary with a malformed config | `Config::load` returns the error at startup. The dialog never opens; the process exits with the diagnostic. |
| `Esc` in settings while a turn is streaming | The settings branch handles Esc and returns. The cancel-turn branch is never reached. The stream continues. (REQ-13.) |
| `Esc` in settings while an approval modal arrives in the same focus tick | Modal preempts settings (REQ-12): `request_decision` at `state.rs:675` clears `state.settings` first. The user never sees the settings dialog after the modal closes. |
| `(+)` Add provider in `integrations` | `handle_add_provider()` returns `true`; the driver closes the dialog and shows the notice `Add provider not implemented yet`. No persistence call. (REQ-14.) |
| User runs binary with `--line-mode` | `dispatch_command` is unreachable from the line prompt. `refresh_completion` is unreachable. The dialog never opens. (REQ-15.) |
| Catalog is refreshed while modal/dialog is open | `refresh_completion` early-returns preserving the palette's generation (line 430). The catalog stays consistent with whatever already renders. |

## Testing Strategy

Tests live in three crates and follow existing patterns. **None of
the tests touch `plugin-api` because the host short-circuit is a pure
`presentation`-internal behavior.**

### Unit tests in `crates/huginn/presentation/tests/settings.rs` (NEW)

Pure logic on the dialog state machine. No terminal. Each test builds
a `SettingsDialog::new(&UiConfig::default())` and exercises
navigation, working-copy updates, and `apply` with a mocked `save`
closure.

| Test | Asserts | REQ |
|---|---|---|
| `settings_tab_cycling` | `next_tab()` cycles through `[theme, indicators, sound, toasts, integrations]` with wrap-around; `prev_tab()` is the inverse. | REQ-03 |
| `settings_item_cycling` | `select_next()` wraps at the last item of the active tab; `select_prev()` wraps at index 0. | REQ-04 |
| `settings_toggle_advances_cycle` | On `toasts.level` row, `toggle_selected()` advances the cycle `info → warn → error → info`. | REQ-05 |
| `settings_apply_invokes_closure` | `apply(&mut save)` calls the closure exactly once with the projected `UiConfig`. | REQ-07 |
| `settings_apply_failure_keeps_dialog` | When `save` returns `Err(anyhow!("disk full"))`, the dialog retains its working copy and `last_error == Some("disk full".into())`. | REQ-07 |
| `settings_esc_does_not_invoke_closure` | `discard()` does not call `save`. | REQ-06, REQ-08 |
| `settings_add_provider_shows_notice` | When the active tab is `integrations` and `selected` is on `(+)` row, `handle_add_provider()` returns `true`. The `apply` path is not invoked. | REQ-14 |
| `settings_snapshot_items_match_locked_d6` | For each of the five tabs, `snapshot().items` matches the spec's locked content (13 theme rows, 3 indicators rows, 1 sound row, 2 toast rows, 1 integration row). | REQ-05 |

The mock closure is a `FnMut(&UiConfig) -> anyhow::Result<()>` that
pushes into `Vec<UiConfig>` behind an `Arc<Mutex<...>>` so tests can
verify both the call count and the projected value.

### Unit tests in `crates/huginn/presentation/tests/dispatch_settings.rs` (NEW)

Exercises the host short-circuit in `Ui::dispatch_command` and the
catalog entry in `Ui::refresh_completion`. **No plugin registration
needed** — the test asserts host behavior directly.

| Test | Asserts | REQ |
|---|---|---|
| `dispatch_opens_settings_dialog` | `Ui::dispatch_command("/settings")` returns `true`, populates `state.settings`, and the editor is cleared via the `try_send` guard (no prompt). | REQ-01 |
| `dispatch_opens_settings_with_trailing_whitespace` | `dispatch_command("/settings   ")` returns `true` and opens the dialog. | REQ-01 |
| `dispatch_does_not_invoke_prompt_sender` | Opening `/settings` does **not** call the prompt sender. The test wires a sender and asserts the receiver is empty after Enter. | REQ-01 |
| `dispatch_settings_consumes_when_modal_open` | With `state.modal = Some(_)`, `dispatch_command("/settings")` returns `true` (consumed) but the modal is not displaced and `state.settings` stays `None`. | REQ-01, REQ-12 |
| `refresh_completion_lists_settings` | `Ui::refresh_completion(model_configured=false)` emits a catalog including `/settings` with description `TUI settings`. | REQ-16 |
| `refresh_completion_lists_settings_even_when_model_unconfigured` | Same as above with `model_configured=true` — `/settings` is unconditional. | REQ-16 |

### Integration test in `crates/huginn/presentation/tests/preemption.rs` (NEW)

Exercises the precedence chain via `Ui`. Only the **mock** plugin
dialog path is used; no plugin-api changes are involved.

| Test | Asserts | REQ |
|---|---|---|
| `settings_modal_preempts_by_approval` | With `state.settings` set, calling `request_decision` clears `settings` and renders the approval modal. Once the approval resolves, `settings` stays `None`. | REQ-12 |
| `settings_preempts_plugin_dialog` | With `state.settings` set, dispatching a plugin command that returns `CommandResult::Dialog(...)` clears `settings` and renders the plugin dialog. | REQ-12 |
| `settings_esc_does_not_cancel_turn` | With `state.turn_cancel = Some(token)` and `state.settings` set, calling the input handler with `KeyCode::Esc` does not call `token.cancel()`. The dialog closes. | REQ-13 |

### Integration test in `crates/huginn/presentation/tests/pty.rs` (ADD)

Add a PTY fixture mode that renders the **host-opened** settings
dialog (no plugin registration). The new fixture uses the
host `/settings` and mirrors the `extension` mode already present
(line 552 in `pty.rs`).

| Test | Asserts | REQ |
|---|---|---|
| `settings_pty_render_header_and_marker` | Dispatching `/settings` from the chat input shows the header `theme indicators sound toasts integrations` with `theme` inverted, and the first body row is `> one-dark ✓`. | REQ-02, REQ-05 |
| `settings_pty_esc_does_not_cancel_running_turn` | Open `/settings`, press Esc, the status bar shows `Settings discarded`, and an active turn keeps streaming. | REQ-13 |

### Unit tests in `apps/huginn/tests/ui_settings.rs` (NEW)

The TOML round-trip family, exercising `ui_settings::persist` against
the `UiConfig` from `huginn_presentation::settings`.

| Test | Asserts | REQ |
|---|---|---|
| `config_load_no_ui_section` | `Config::load` on a `config.toml` with no `[ui]` returns `UiConfig::default()`. | REQ-10 |
| `config_load_partial_ui_section` | `[ui.theme]\nname = "dracula"` loads with `theme.name == "dracula"` and all other sub-tables default-filled. | REQ-10 |
| `config_load_with_inner_unknown_key` | `[ui]\nunknown_user_key = true\n[ui.theme]\nname = "dracula"` loads without error. | REQ-11 |
| `persist_round_trip_byte_stable_outside_ui` | A fixture with `[provider]`, `[model]`, and a hand-written comment, after `persist(&UiConfig { theme: { name: "catppuccin" }, .. }, path)`, has `[provider]`, `[model]`, and the comment byte-identical to the input. | REQ-09 |
| `persist_creates_missing_file` | Calling `persist` on a path that does not exist creates the file with the new `[ui]` table. | REQ-09 |
| `persist_preserves_unknown_inner_keys` | The round-trip with `unknown_user_key = true` under `[ui]` keeps the key in the output. | REQ-11 |
| `persist_atomic_write_failure_preserves_original` | (Optional, deferred) Stub test ensuring the original file survives a forced write failure. | — |

### Catalog test in `crates/huginn/presentation/tests/completion.rs` (ADD)

| Test | Asserts | REQ |
|---|---|---|
| `settings_host_completion_catalog` | After constructing `Ui` with the host `/settings` catalog entry, the `/`-completion catalog lists `/settings` (description `TUI settings`) alongside `/quit`. **No plugin registration is needed.** | REQ-16 |

> **Removed test.** The previously-listed
> `settings_unload_clears_command` test is dropped: there is no
> plugin to unload. The dialog is host-owned and persists for the
> duration of the process; there is no scenario where the catalog
> entry can disappear.

### Strict TDD

The project sets `strict_tdd: false` (per
`.agents/sdd/config.yaml` precedent cited in the
`provider-quota-tracking` change). The tests above are functional and
run after the implementation lands; they cover the locked contract
without the formal RED→GREEN→REFACTOR cycle.

## Files Expected to Change

### Rust backend

- `crates/huginn/presentation/src/lib.rs` — add `pub mod settings;`.
- `crates/huginn/presentation/src/settings.rs` — new file with the
  `UiConfig` DTOs, the five working structs, and the
  `SettingsDialog` state machine.
- `crates/huginn/presentation/src/state.rs` — add `settings` /
  `settings_focus_epoch` to `UiState`; add `ui_config` /
  `save_ui_config` to `Ui`; extend `Ui::new`; add the host
  short-circuit to `dispatch_command` (line 367); extend
  `refresh_completion` (line 400) with `CatalogCommand::host("/settings", …)`;
  extend `request_decision` to clear `settings` alongside
  `plugin_dialog` and `workflow`.
- `crates/huginn/presentation/src/terminal.rs` — extend `View`
  snapshot and `draw_frame`; add the settings input branch ahead of
  `plugin_dialog`.
- `apps/huginn/src/config.rs` — add `ui: UiConfig` field to `Config`
  with `#[serde(default)]`, importing `UiConfig` from
  `huginn_presentation::settings`.
- `apps/huginn/src/ui_settings.rs` — new file with `persist`.
- `apps/huginn/src/main.rs` — construct `Arc<UiConfig>` and the save
  closure; pass both into `RatatuiPlugin::with_settings(...)`.
- `Cargo.toml` (workspace) — add `toml_edit = "0.22"` under
  `workspace.dependencies`.
- `apps/huginn/Cargo.toml` — add `toml_edit.workspace = true` to
  `[dependencies]`.
- `apps/huginn/config.example.toml` — add the documented `[ui]`
  block.

### Files explicitly NOT changed

- `crates/huginn/plugin-api/src/v1.rs` — **no new `CommandResult`
  variant**. Host `/settings` does not flow through this enum.
- `crates/huginn/presentation/src/contributions.rs` — **no
  `SettingsPlugin`, no `register_command` for `/settings`**.

### Tests

- `crates/huginn/presentation/tests/settings.rs` — new file (8 unit
  tests).
- `crates/huginn/presentation/tests/dispatch_settings.rs` — new
  file (6 host-dispatch + catalog tests).
- `crates/huginn/presentation/tests/preemption.rs` — new file (3
  integration tests).
- `crates/huginn/presentation/tests/pty.rs` — add 2 settings PTY
  tests; add a fixture mode in `tests/support/`.
- `crates/huginn/presentation/tests/completion.rs` — add 1 host
  catalog test.
- `apps/huginn/tests/ui_settings.rs` — new file (6 round-trip tests).

## Decisions and Deferred Work

- **`/settings` is a host short-circuit in `dispatch_command`.** No
  plugin contribution, no `CommandResult::Settings`. The single entry
  point is `state.rs:367`. The catalog entry is added unconditionally
  in `refresh_completion` (line 400) next to `/quit`/`/exit`. This is
  the simplest model that satisfies REQ-01, REQ-15, and REQ-16
  without touching the plugin registry.
- **`UiConfig` lives in `presentation::settings`, not in
  `apps/huginn::config`.** The dependency arrow is one-way:
  `apps/huginn → huginn-presentation`. No snapshot adapter, no trait.
  Tests construct `UiConfig::default()` and `SettingsDialog::new(&…)`
  without crossing crates. The binary's `config.rs` keeps a single
  `ui: UiConfig` field.
- **`Arc<UiConfig>` replacement after apply.** No `watch` channel.
  The dialog always closes on apply success, so the next open reads
  the fresh snapshot. A future live-subscribers change will need a
  notification mechanism; out of scope here.
- **`UiConfig` and sub-tables do NOT carry `deny_unknown_fields`.**
  Required by REQ-11. `Config` itself keeps `deny_unknown_fields`.
- **No `Config::save`.** The proposal mentions `Config::save` for
  round-trip tests only; v1 does not implement it. The runtime
  writer is `ui_settings::persist` exclusively. If a future change
  needs a full rewrite, it adds `Config::save` then.
- **No `dirs::config_dir()` resolution in `ui_settings`.** The
  binary resolves the path in `main.rs` and passes it into the
  closure. The helper is path-agnostic and easy to point at test
  fixtures.
- **Failure notice placement.** Apply failures surface in the status
  bar and close the dialog (rather than staying open with the error
  in the body) — keeps the input lease invariants simple and lets
  the user re-open the dialog with the working copy intact.
  `last_error` is still populated on `SettingsDialog` so re-opening
  surfaces the last failure for a frame.

## Out of Scope (v1) — repeated for reviewers

Locked out of scope to prevent misreading the dialog as a real control
surface. Any future change that wires a persisted value into a runtime
subsystem must explicitly modify the `ui-settings` capability.

- **Live theme application.** Persisting `[ui.theme]` does **not**
  switch the rendered palette. The theme picker writes the file.
- **No live sound/toast/indicator toggling.** `[ui.sound]`,
  `[ui.toasts]`, `[ui.indicators]` are persisted but not consumed by
  their subsystems in v1. Subscribers are follow-up changes.
- **`run` mode stays in line prompt.** The modal is TUI-only
  (chat frontend). `--line-mode` users cannot open settings because
  `dispatch_command` is not reached from the line prompt and
  `refresh_completion` is not called.
- **`(+) Add provider…` is a TODO.** Selecting it shows a notice. It
  does not create an entry, prompt for a name, or read provider
  state.
- **No diff / preview.** There is no before/after view of the config
  file.
- **No undo / redo.** `Esc` discards the working copy; there is no
  per-tab history.
- **No `/settings reset` / per-tab revert / per-row edit.**
- **No migration** of existing user `config.toml` files.
  `#[serde(default)]` on `ui` makes the new field a no-op when
  absent.
- **No external plugin dialog for settings.** The dialog is
  host-owned; the command surface is host-owned too. There is no
  plugin-api change for `/settings`.
- **No audio playback or theme loading.** Items are stored as
  names/booleans/durations only.

## Risks / Implementation Guardrails

- **Host short-circuit must run before `contributions.command`.**
  Order matters in `dispatch_command`: if a plugin ever registered
  a `/settings` of its own, we would silently lose the dialog. The
  match is intentionally placed first; this is documented in the
  patch so reviewers notice it.
- **`dispatch_command` consumes even when blocked.** A modal or
  open settings must NOT cause `/settings` to fall through to the
  prompt sender. The patch returns `true` in all branches to
  guarantee REQ-01. Reviewers must verify both the open path AND
  the blocked path return `true`.
- **`deny_unknown_fields` at the top level vs. inner tolerance.** The
  top-level `Config` keeps `deny_unknown_fields`; `UiConfig` and its
  sub-tables deliberately do not. Tests must verify both contracts:
  top-level rejects stray keys, inner accepts them.
- **Atomic write on Windows.** `fs::rename` on Windows fails if the
  target exists. Use `fs::rename` only — `toml_edit` formatting and
  atomic write are exercised on Unix CI; if Windows support matters,
  a follow-up change should swap in `MoveFileExW` with
  `MOVEFILE_REPLACE_EXISTING`.
- **`toml_edit 0.22` vs `0.25`.** Workspace pins to `0.22` to match
  the version `toml 1.x` already pulls in transitively. Pulling
  `0.25` would duplicate the major version in the lockfile and grow
  compile time.
- **Closure runs under the UI mutex.** A slow disk stalls the UI the
  same way `choose_plugin_dialog` does today. Accepted as the v1
  trade-off; a background task is the natural follow-up.
- **Modal preempts settings on arrival.** Ensure
  `request_decision` (`state.rs:675`) is updated in the same
  patch as the new field. A missed `state.settings.take()` would
  leak the dialog across approval cycles.
- **`settings` arm placement vs. `Esc → cancel turn`.** The new arm
  must live inside the `state.modal.is_none()` gate and **return**
  after handling; Esc inside settings must never reach the
  cancel-turn branch.
- **`Arc<dyn Fn>` cloning.** The driver calls
  `(*self.ui.save_ui_config).clone()` per apply, which is cheap but
  not free. Acceptable for v1.
- **Catalog unconditional entry.** `refresh_completion` adds
  `/settings` outside the `if model_configured` block. A reviewer
  who moves it inside by accident will silently break REQ-16 in
  offline / pre-model flows.
- **Catalog `state.modal.is_some() || …` early-return.** The
  pre-existing `refresh_completion` short-circuit at line 430
  preserves the palette's generation when any modal/dialog is
  open. Adding `/settings` does **not** require a new guard; the
  existing guard already covers `state.settings` once the dialog
  is alive (because `view.settings` does not block the catalog —
  the catalog lives in completion, not in the modal area).
