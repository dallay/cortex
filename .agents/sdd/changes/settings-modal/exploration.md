# Exploration: Settings Modal in Huginn TUI

> **Change slug**: `settings-modal`
> **Phase**: 1 — Explore
> **Artifact store**: `.agents/sdd/`

## Intent

Add an in-TUI **settings modal** to Huginn so the user can inspect — and
eventually edit — interface-level preferences (theme, indicators, sound,
toasts, integrations). v1 is **shell + persistence only**: the modal must
render all five tabs with real items and the marker/checkmark convention of a
classic editor theme picker, persist changes to `config.toml` under a new
`[ui]` section, and close cleanly. Hooking the persisted values into sound,
toast, indicator, or provider subsystems is deferred to follow-up changes.

This document records the verified current state and the open design
decisions that the proposal must lock down before specs are written.

## Conventions Observed in This Repo

- Workspace package: `apps/huginn` (binary) + `crates/huginn/{core,application,runtime,presentation,plugin-api,reference-extension}`. See root `Cargo.toml`.
- Presentation crate: `crates/huginn/presentation/`. All terminal I/O lives behind `Ratatui` (`crates/huginn/presentation/src/terminal.rs`) and is driven by a deterministic state projection in `crates/huginn/presentation/src/state.rs`.
- Config crate: `apps/huginn/src/config.rs`. `Config` derives `Deserialize` with `#[serde(default, deny_unknown_fields)]`.
- Commands are split in two layers:
  - **Plugin-contributed commands** registered through `Contributions` (`crates/huginn/presentation/src/contributions.rs`). Looked up by `Ui::dispatch_command` (`state.rs:367`). Examples: `/help` (`contributions.rs:24–26`).
  - **Host-only commands** (`/quit`, `/exit`, `/compact`, `/summarize`) — these are added to the completion catalog (`state.rs:400–416`) but `dispatch_command` returns `false` for them so they fall through to the prompt sender. The only host-only command short-circuited before submission is `/quit` / `/exit`, hard-coded in the Enter handler (`terminal.rs:923–926`).
- Host commands `/help`, all modal/dialog rendering, and key wiring happen inside the presentation crate. The `apps/huginn` binary only loads `Config` and starts the plugin supervisor.
- Prior change precedent: `.agents/sdd/changes/provider-quota-tracking/`. Uses Markdown headings, GitHub-style task checkboxes, kebab-case filenames. `state.yaml` tracks `current_phase`, the cumulative `completed:`, and a `next` pointer.

## Verified Map of Relevant Code

### State projection (`crates/huginn/presentation/src/state.rs`)

- `UiState` struct (line 132) is the single mutexed projection. It owns:
  - `editor: Editor` (line 134) — composer input.
  - `completion: Completion` (line 135) — `/`-prefix palette.
  - `modal: Option<Modal>` (line 145) — approval prompt with one-shot reply channel and UUID confirmation code.
  - `plugin_dialog: Option<PluginDialog>` (line 146) — **closest analogue to what settings needs**.
  - `workflow: Option<crate::workflow::HostWorkflow>` (line 147) — multi-step plugin form; useful as a reference for tab/select rendering but heavier than necessary.
- `PluginDialog` (line 164): `selected: usize`, `body_scroll: usize` (PgUp/PgDn). No tabs, no footer.
- `Ui::new` (line 334) is constructed once per session; it does not accept config today.
- `Ui::dispatch_command` (line 367) is the only path that walks the user-typed `/`-prefix line into a `Contributions::command` lookup. If no plugin matches, this returns `false` and the line goes into the prompt sender (or, for `/quit`/`/exit`, hits the hard-coded short-circuit at `terminal.rs:923`).
- `Ui::refresh_completion` (line 400) assembles the command catalog: contributed commands + `CatalogCommand::host("/quit" | "/exit")` + (when model is configured) `/compact | /summarize`. Any new `/settings` command added here becomes a usable slash command *only* for completion display — it still falls through to the model as a regular prompt. **Routing decision: settings must reach `dispatch_command` (i.e., must be a contributed command) to avoid being sent to the model.**
- `Ui::dispatch_handle` (line 378) shows the existing surface for what registered plugin commands return: `Notice(view) | Dialog(handle) | Workflow(workflow)`. There is no `Settings` variant today; we either add one or open a stub plugin dialog with the settings title body.
- `Ui::choose_plugin_dialog` (line 590) takes the dialog, calls `handle.choose(selected)`, and shows the result as a notice.
- `Ui::resolve_modal` (line 599) and `Ui::shutdown` (line 608) already cover cleanup of `modal`, `plugin_dialog`, `workflow`, and `widgets`. `shutdown` clears `plugin_dialog.take()`, so a settings dialog opened on `/settings` is cleaned up at session end (line 614–618).
- `Ui::start_file_index` (line 459) is the only existing pattern for background work spawning under the UI mutex; not relevant to settings rendering, but is the precedent for not holding the state mutex during IO.

### Terminal rendering and input (`crates/huginn/presentation/src/terminal.rs`)

- `Driver` struct (line 187) and the per-session run loop (line 658) drain input + draw frames at ~30 fps (33 ms tick, line 663).
- `View` snapshot (line 354) packages the state for the renderer:
  - `modal: Option<(preview, number, code, typed, scroll)>` (line 361).
  - `plugin_dialog: Option<(title, body, choices, selected, body_scroll)>` (line 362).
  - `workflow: Option<(title, body, fields, selected, pending)>` (line 363).
- `View::snapshot` (line 367) destructures each Option into a flat tuple used by `draw_frame`. **A settings pane needs a new snapshot field**, or we have to project to the closest existing shape (`plugin_dialog`) with implicit tabs encoded inside the body — the cleaner path is to add a new field.
- `draw_frame` (line 482) lays out per-overlay as `Layout::vertical([body, footer])`, then a single `Block::bordered()` for body and a `Paragraph` for footer. Modal (line 486) uses two-line footer; workflow (line 514) uses two-line header + footer; plugin dialog (line 530) uses one-line footer — the **closest layout to the requested settings picker**.
- Key handling (line 734): the dispatch order matters — modal > plugin_dialog > workflow > completion > editor. Enter on plugin_dialog calls `self.ui.choose_plugin_dialog()` (line 821–823).
- `Ui::dispatch_command` at terminal.rs:928 only runs in the Enter-pressed state. `dispatch_command` returns `false` for `/quit`/`/exit` when the corresponding plugin is not registered, but Enter short-circuits `/quit`/`/exit` two lines earlier. **For `/settings`, we want the `dispatch_command` path to succeed so we open the modal — not to fall through to `try_send` to the model.**
- The repository's plugin dialog PTY tests (`crates/huginn/presentation/tests/pty.rs:566` onward — `long_plugin_dialog_window_keeps_selection_visible`, `long_plugin_dialog_body_never_hides_every_choice`) and `tests/extensions.rs:94–103` are the template for new settings-dialog tests.

### Config and CLI (`apps/huginn/src/config.rs`, `apps/huginn/src/main.rs`, `apps/huginn/config.example.toml`)

- `Config` has `deny_unknown_fields` (line 7) and `validate_common` (line 225). A new `[ui]` table must default to `#[serde(default)]` for that table or `Config::deserialize` will reject the user's existing `config.toml` files.
- `Config::load` (line 134) resolves the file from `--config` > `cortex/huginn/config.toml` > legacy `cortex/agent/config.toml` (read-only fallback). It never writes back.
- `Config` has no `save`/`write` method. **The settings dialog will be the first writer.** We need a sibling method (e.g. `Config::save(&self, path: &Path)`) that re-serializes the entire struct back to TOML.
- `prepare_config` (`main.rs:60`) fills from env after TOML load and runs `validate_common` before any DB open. That sequence must continue to run; settings persistence writes happen at a different lifecycle moment — when the user presses Enter inside the dialog.
- `apps/huginn/src/terminal.rs` is the **LinePrompt** line-oriented recovery adapter used in `--line-mode`. Settings modal is TUI-only, so it does not need to integrate there.
- Compatibility tests in `apps/huginn/tests/config_compat.rs` exercise legacy config paths. A save path adds no new validation behavior but does change file semantics — the test file should grow a regression for round-trip + omit-only properties.

### Workspace / config drift to ignore

- `.agents/sdd/config.yaml` is heavily Rook-centric and lists packages under `crates/domain/*` / `crates/infrastructure/*` that no longer exist in this workspace. The current workspace at the root `Cargo.toml` is `crates/huginn/*` and `crates/infrastructure/*` (and `crates/domain` is empty). Do not propose restructuring; we only touch the Huginn stack. Reference doc notes (`docs/huginn/`) are accurate but not preconditions for this exploration.

## Current Behavior the Modal Replaces

There is no existing settings surface for Huginn. Today:

- `/help` renders a static notice (`contributions.rs:24–26`). No way to change interface state.
- The plugin dialog (`crates/huginn/presentation/src/state.rs:164`) is the only generic selection widget and is driven by `huginn_plugin_api::v1::DialogHandle`, which is *content provided by a plugin*. Tabs and persistence do not exist there.
- The workflow form (`crates/huginn/presentation/src/workflow.rs`) is a multi-field form for plugin-supplied workflows, not a user-facing settings UX.

So this change introduces the first **native** UI surface owned by the host that is **not** a plugin dialog.

## Open Design Decisions Required Before `propose`

These are the decisions that must be locked before specs can be written. Each is consequential; defaults below are options, not recommendations.

### D1. TOML shape for the UI section

Two viable shapes:

- **(a) Nested sub-tables, one per tab** — `[ui.theme]` / `[ui.indicators]` / etc. Each sub-table holds the current selection or, for collection-style tabs, allows `[[ui.theme.entries]]` arrays. Cleaner for heterogeneous tabs (e.g. integrations as a list of named entries).
- **(b) Inline-array of tab/option objects** — e.g. `[[ui]]` with rows of `{tab, key, value}`. Awkward to author by hand. **Not recommended.**

Recommendation: **(a)** with one sub-table per tab. The `Config` struct gains a `pub ui: UiConfig` (defaulted), `UiConfig` has five sub-structs each with its own default. Save emits the existing fields plus whatever the user has touched.

### D2. How is `UiConfig` delivered into the UI?

- **`Ratatui::with_readiness`-style setter** (mirrors the existing `RatatuiPlugin::with_readiness` at `lib.rs:104`): add a builder for `UiConfig` on `Ratatui`/`RatatuiPlugin`. `Ui::new` would then accept an `Arc<UiConfig>` or a snapshot. Lets the binary pass config once on plugin registration.
- **Held on `Ui::new` directly** — extend `Ui::new` to take an additional argument. Less ceremony.

Recommendation: **add `Ratatui::with_ui_config(...)` and pipe through `Ui::new(ui_config: Arc<UiConfig>)`**. The config is read-only from the UI thread except when `apply` rewrites and reloads.

### D3. Where does TOML serialization live?

- **`SettingsDialog::to_toml(&self) -> toml::Value`** — pure projection from in-memory `SettingsDialog` to TOML. The dialog owns its own state for current values; serialization is a method on it.
- **Free function in a new module `apps/huginn/src/ui_settings.rs`** — keeps dialog logic in `presentation/`, serialization in the binary crate.

Recommendation: keep the dialog in `presentation/` (where the user interacts with it), put serialization in `apps/huginn/src/ui_settings.rs` because the binary already owns config and `Config::save`. Add a thin `pub fn persist(ui: &UiConfig, path: &Path) -> anyhow::Result<()>` that the dialog's `apply` calls via a closure the binary injects.

### D4. Apply strategy — full rewrite vs. patch

- **Full rewrite** — write the current `Config` back to disk. Simple, atomic, but **clobbers any unrelated keys** the user added by hand or CLI flags bounced back into config.
- **Patch + preserve unknown keys** — read TOML → modify only `[ui]` → write back. Needs `toml_edit` (`toml::Value` does not preserve formatting/comments).
- **Write to a separate `ui.toml` overlay** — leaves the main config alone, but splits user state across files. Not idiomatic.

Recommendation: **patch with `toml_edit`**, preserving comments and unknown sections. AGENTS.md says "do not reset data as part of routine testing"; the same posture argues against silent overwrite of hand-edited values.

### D5. Command routing for `/settings`

Two paths both work:

- **(a) Plugin-contributed command** — register a small `SettingsPlugin` in `crates/huginn/presentation/src/settings.rs` that registers a single `/settings` command via `Contributions`. It opens the dialog via a new `CommandResult` variant *or* via the existing `Dialog(handle)` variant with a settings-specific `DialogHandle` implementation that the presentation crate owns. This integrates with `dispatch_command` and `refresh_completion` automatically. **Cleanest, requires no touch to `terminal.rs` Enter handling.**
- **(b) Hard-coded host command** — add a branch before the prompt sender in `terminal.rs:928` (mirroring `/quit`/`/exit` at line 923) plus a new `CatalogCommand::host("/settings", ...)` in `refresh_completion`. Less ceremony but bypasses `dispatch_command`, mirroring the special handling already present for two commands.

Recommendation: **(a)** — register a settings plugin. Stays within existing architecture, gets catalog + dispatch + future plugin compatibility for free. The plugin can live inside the `presentation` crate so `apps/huginn` doesn't grow new wiring.

### D6. Tab model and per-tab item source

A `SettingsDialog` needs:

- `tabs: Vec<&'static str>` for the fixed five: `theme`, `indicators`, `sound`, `toasts`, `integrations`.
- `active_tab: usize` cycled by `Tab` / `BackTab` (or `←/→`).
- For each tab, a list of selectable items. v1 is shell-only, so items are mostly placeholders:
  - `theme`: `auto`, `dark`, `light`, `solarized` (or similar preset names) — backed by a real string in TOML.
  - `indicators`: `minimal`, `verbose` — single-select.
  - `sound`: `off`, `on` — toggles a boolean.
  - `toasts`: `off`, `errors`, `all` — single-select.
  - `integrations`: list of provider ids — read-only display in v1 (live integration with `plugins_rs::providers` is deferred).
- Each tab's selected option renders with the requested `▶ … ✓` marker and the active item gets the `✓` suffix.

Open sub-decision: do we want **horizontal Tab/BackTab** (and disable `↑↓` for tab change) or **vertical tabs as a sidebar**? The user prompt explicitly says "tabs in header" → render as header text like `theme · indicators · sound · toasts · integrations` with the active tab inverted/highlighted. Items listed beneath it as rows.

### D7. Persisted values vs. live values in v1

For each tab item, decide whether the displayed `✓` reflects (i) the value currently stored in `Config` (the **persisted** state) or (ii) a transient working copy inside `SettingsDialog` that commits on Enter.

- **Working copy in dialog, commit on Enter** — matches the picker analogy and user expectation. `Esc` discards. The user can move selection without committing.
- **Live binding to `Config`** — Esc has no semantics; selections take effect immediately.

Recommendation: **working copy**. This is the only way `Esc` means "discard" without surprising the user.

### D8. Focus / input lease

Settings sits behind `plugin_dialog` in the input order. We can either:

- Reuse the `plugin_dialog` code path (so its key handler at `terminal.rs:808–828` still works), but add tabs as a parallel "before" branch.
- Add a new `state.settings: Option<SettingsDialog>` alongside `state.plugin_dialog`, with **its own key handler** ordering ahead of plugin_dialog.

Both work. Recommendation: **new field + new key branch**, because the input model differs (Tab/BackTab changes tabs, Enter applies + closes, Esc closes without calling `handle.choose`). Reusing `plugin_dialog`'s handler would require a flag-based dispatch anyway.

### D9. What `apply` actually does to runtime state

v1 is shell + persistence. So:

- `apply` writes the new `[ui]` table to the user's `config.toml`.
- `apply` does **not** push the values into a live renderer (sound, theme). Subscribers are follow-up changes.
- A success/failure notice is shown in `committed` using the existing `Ui::notice` path.

## Candidate Scope

1. New `SettingsDialog` struct in `crates/huginn/presentation/src/settings.rs`.
2. Snapshot projection through `View` so `draw_frame` renders the header-with-tabs + item list + footer.
3. Key handler ahead of `plugin_dialog` in `input()` for Tab/BackTab/Arrow/Enter/Esc.
4. A `SettingsPlugin` registered in `presentation/src/lib.rs` exposing `/settings` as a contributed command. `dispatch_handle` learns to open a `SettingsDialog` instead of a `PluginDialog` when the result is a `SettingsCommandResult::Open`.
5. `UiConfig` added to `apps/huginn/src/config.rs` with `#[serde(default)]`, plus `Config::save(&self, path: &Path)` using `toml_edit`.
6. A `pub fn persist(ui: &UiConfig, path: &Path) -> anyhow::Result<()>` shim, called from the dialog's `apply`.
7. Update `apps/huginn/config.example.toml` with a commented `[ui]` block showing the schema.
8. Tests: a non-interactive test that the dialog state machine cycles tabs and stores per-tab selections; a TOML round-trip test confirming a write does not corrupt unrelated keys; a PTY test for the picker look (header + marker + footer).

## What Is Explicitly Out of Scope (v1)

- No wiring of `[ui]` values into the renderer, sound, indicators, or toast subsystems.
- No external plugin APIs (the dialog is internal; the *command* is plugin-contributed but the dialog state lives inside `presentation`).
- No migration of existing user config — `Config` already grows `#[serde(default)] pub ui: UiConfig`, so missing keys are fine.
- No settings for subcommands like `/settings reset`. Out of scope until we know which knobs are real.
- No `[[ui.integrations]]` editing — read-only display in v1, per the user prompt.

## Risks

| Risk | Impact | Mitigation direction |
|---|---|---|
| TOML `deny_unknown_fields` rejects user configs that suddenly contain a new top-level `[ui]` from another tool | Existing configs fail to parse on startup | Make `ui: UiConfig` itself `#[serde(default)]`; the table deserializer ignores unknown inner fields only when the table is also default-tolerant — confirm by adding a regression test with a hand-written `[ui] unknown = true` |
| `apply` overwrites hand-edited top-level keys via full rewrite | User's manual edits to `[provider]` etc. disappear on settings save | Use `toml_edit` patch; preserve comments + unknown tables; add a round-trip test with extra keys |
| Settings dialog and an approval modal both arrive while both are `Some` | The wrong modal wins | Modal opening (`request_decision`) already takes the `plugin_dialog` and `workflow` (state.rs:675–677) — settings must follow that same precedence (modal first, then settings, then plugin_dialog) |
| `Esc` in settings is indistinguishable from the modal-level `Esc` reject (terminal.rs:905–909 only handles `cancel` context) | Could cancel a turn | Place the settings handler before the modal/turn layer with no fall-through to the `Esc → cancel turn` branch, mirroring plugin_dialog's placement |
| `/settings` is treated as a model prompt and sent to the LLM | Wasted tokens, surprising behavior | Register a contributed command (D5a) so `dispatch_command` returns `true` and the Enter handler clears the editor at terminal.rs:929 without sending |
| `Ratatui::with_ui_config` clashing with `with_readiness` | Plugin builder becomes ambiguous | Provide `RatatuiPlugin::with_readiness_and_ui_config` to keep both, or a single `RatatuiPluginBuilder` |
| File-write races with concurrent config writers | Loss of changes | Mutex around `[ui]` write through `presentation::ui_config::persist`, called from the locked UI mutex so the dialog and the writer never interleave |
| `CatalogCommand::host` catalog shows `/settings` as completable even if it didn't dispatch | Future-proof nit; today host commands are catalog-only and `/help` is the only plugin example | Acceptable for v1 — `dispatch_command` will return `true` when registered, so the catalog will be honest |
| `crates/huginn/presentation` does not depend on `apps/huginn`, so it can't reach `Config` directly | Architectural confusion | Keep `Config` in `apps/huginn`; the dialog only knows `UiConfig` (a small subset moved into a shared module or duplicated with serde). Persist function is injected as `Fn(&UiConfig)` closure into the dialog |
| The five tabs are mostly TODOs | v1 ships a shell with no real effect | Spec must encode that this is intentional; the contract is "shell + persistence, subscribers follow" |

## Validation Strategy for This Change

- **Focused iteration**: `cargo test -p huginn-presentation --test extensions`, and add a new `tests/settings.rs` for the dialog state machine. PTY tests via `tests/pty.rs` to cover the rendering and key loop.
- **Config**: `cargo test -p huginn --test config_compat` and a new round-trip test under `apps/huginn/tests/`.
- **Wiring**: `cargo test -p huginn-presentation --test plugins` to confirm `/settings` still appears in catalog alongside `/help`.
- **Pre-PR gate**: `just ci-local` from the project root, per AGENTS.md.

## Existing SDD Precedent

- `.agents/sdd/changes/provider-quota-tracking/` — extensive Exploration doc with the same shape, including "Open Decisions for Proposal/Spec" and a Risk table. Our section ordering matches.
- `.agents/sdd/specs/providers-ui/spec.md` referenced by that change lists existing UI requirements. Settings modal adds a new capability (`ui-settings`) and should not modify `providers-ui` unless a future change explicitly requires cross-referencing.
- `.agents/sdd/config.yaml` is irrelevant to this change; its workspace inventory is Rook-era drift.

## Recommendation

Proceed to `propose` with the following defaults unless the user disagrees:

- **D1 = (a)**: nested `[ui.*]` sub-tables in TOML.
- **D2 = setter**: add `Ratatui::with_ui_config` and `Ui::new(..., ui_config)`.
- **D3 = split**: dialog logic in `presentation`, persistence helper in `apps/huginn/src/ui_settings.rs`.
- **D4 = patch with `toml_edit`**.
- **D5 = (a)**: settings is a plugin-contributed command.
- **D7 = working copy**: commits on Enter, discards on Esc.
- **D8 = new field + new handler**: `state.settings: Option<SettingsDialog>` ahead of `plugin_dialog`.
- **D9 = apply only persists + notices**: no live subsystem wiring in v1.

The proposal can carry these defaults and ask for confirmation on every D-question, since locking them is a single round-trip.
