# Proposal: Settings Modal in Huginn TUI

> **Change**: `settings-modal`
> **Phase**: 2 — Propose
> **Artifact store**: `.agents/sdd/`

## Intent

Add a `/settings` command to Huginn's TUI that opens a host-owned
modal with five tabs (`theme`, `indicators`, `sound`, `toasts`,
`integrations`), lets the user pick options using the
`▶ name ✓` marker convention, persists the picked values to a new
`[ui]` section in `config.toml` on Enter, and discards the pending
selection on Esc — without touching any runtime subsystem.

## Scope

### In scope

1. **`/settings` command registration.** A new
   `SettingsPlugin` in `crates/huginn/presentation/src/settings.rs`
   registers a single conversation-domain command named `/settings`
   through `Contributions` (mirrors the `/help` registration in
   `contributions.rs:24–26`). The command returns a new host-side
   result that opens a `SettingsDialog`. A shortcut is also added
   to `Ui::dispatch_command` before the `contributions.command(...)`
   lookup, so `/settings` is never sent to the model as a prompt.

2. **`SettingsDialog` state.** New
   `pub settings: Option<SettingsDialog>` on `UiState`, parallel
   to `plugin_dialog` and `workflow`. Holds: `active_tab: usize`,
   a per-tab `selected: usize`, and the working-copy value for each
   tab (the `✓` reflects the working copy, not the persisted
   `Config`). `Esc` clears this field without persisting; `Enter`
   triggers `apply`.

3. **Five fixed tabs with these concrete items** (the visible
   content the user signs off on per the locked D6):

    - **theme** — 13 named themes rendered as single-select rows.
      Persisted as a single `name` field. Items: `one-dark` (✓
      default), `one-light`, `catppuccin`, `catppuccin-latte`,
      `tokyo-night`, `tokyo-night-day`, `dracula`, `nord`, `gruvbox`,
      `gruvbox-light`, `terminal`, `solarized`, `solarized-light`.
    - **indicators** — 3 boolean toggles: `scrollbar=on ✓`,
      `breadcrumbs=on ✓`, `line_numbers=on ✓`. Each renders a `✓`
      when on, blank when off.
    - **sound** — 1 boolean toggle: `bell=off ✓`.
    - **toasts** — 1 cycle picker (level) + 1 cycle picker (duration_secs). Level cycles `info ✓ → warn → error`.
      Duration cycles `3s ✓ → 5s → 10s → off`.
    - **integrations** — empty list + a single `(+) Add provider…`
      row that is a documented TODO: selecting it emits a notice
      `Add provider not implemented yet` and does not mutate
      state.

   Navigation: `←/→` (or `Tab`/`BackTab`) switches tabs; `↑/↓`
   moves the per-tab selection; `Enter` applies; `Esc` discards.

4. **Input lease with `Modal > Settings > PluginDialog > completion`.**
   The `SettingsDialog` branch is placed in `Driver::input`
   (`terminal.rs:734`) **between** the modal branch and the
   `plugin_dialog` branch, so an approval modal arriving during
   settings takes precedence (and disposes of `settings`)
   exactly the way the current code disposes of `plugin_dialog`
   and `workflow` at `state.rs:675–676`. `Esc` in settings never
   falls through to the modal/cancel-turn branch.

5. **View snapshot and rendering.** `View::snapshot`
   (`terminal.rs:367`) gains a `settings: Option<(title, tabs,
   active_tab, items, selected)>` field. `draw_frame`
   (`terminal.rs:482`) renders a header line with the tab list (active tab inverted), a body with the items rendered as
   `▶ name ✓` rows (active line reverse-video), and a one-line
   footer `←/→ tabs · ↑/↓ select · Enter apply · Esc discard`.

6. **Persistence via injected closure.** The `presentation` crate
   stays agnostic of the binary: `Ui::new` (or a
   `Ratatui::with_ui_config(...)` builder) accepts a closure
   `Fn(&UiConfig) -> anyhow::Result<()>` plus the current
   `Arc<UiConfig>` snapshot. `SettingsDialog::apply` calls the
   closure with its projected `UiConfig`; on success it shows
   notice `Settings saved`, on failure it shows the error and
   leaves the dialog open. The closure body lives in
   `apps/huginn/src/ui_settings.rs` and uses `toml_edit` to
   patch the existing `config.toml`, preserving comments and
   any user-written tables outside `[ui]`.

7. **`Config` schema.** `apps/huginn/src/config.rs` gains
   `pub ui: UiConfig` (`#[serde(default)]`), where `UiConfig`
   has five sub-tables — `[ui.theme]`, `[ui.sound]`,
   `[ui.toasts]`, `[ui.indicators]`, `[ui.integrations]` —
   each default-tolerant. `Config` keeps `deny_unknown_fields`
   at the top level but the new `ui` table itself does not
   reject inner unknowns. `Config::save(&self, path)` is
   introduced but is **not** the writer used at runtime; the
   runtime writer is the toml_edit patcher in `ui_settings.rs`.
   `Config` serialization here is for round-trip tests only.

8. **TOML example + example file.**
   `apps/huginn/config.example.toml` grows a commented `[ui]`
   block documenting the schema, mirroring the example
   in the locked D1.

9. **Tests.**
    - `crates/huginn/presentation/tests/settings.rs` — non-interactive
      test of the dialog state machine: tab cycling,
      selection cycling, `apply` builds a `UiConfig` matching the
      working copy, `esc` does not mutate.
    - `crates/huginn/presentation/tests/pty.rs` — a render test for
      the header + items + marker + footer.
    - `apps/huginn/tests/config_compat.rs` (or a new
      `tests/ui_settings.rs`) — round-trip test:
      `Config::load` → patch `[ui]` via toml_edit → unrelated
      keys untouched, comments preserved.
    - `crates/huginn/presentation/tests/plugins.rs` — `/settings`
      appears in the `/`-completion catalog alongside `/help`,
      `/compact`, `/summarize`, `/quit`.

### Out of scope (v1)

These are explicitly **not** part of this change. Locking them out
in writing so future readers don't misread the dialog as a real
control surface:

- **Live theme application.** Persisting `[ui.theme]` does **not** switch the rendered palette. The theme picker only
  writes the file.
- **No live sound/toast/indicator toggling.** `[ui.sound]`,
  `[ui.toasts]`, `[ui.indicators]` are persisted but not
  consumed by their subsystems in v1. Subscribers are
  follow-up changes.
- **`run` mode stays in line prompt.** The modal is TUI-only (chat frontend). `--line-mode` users cannot open settings.
- **`(+) Add provider…` is a TODO.** Selecting it shows a
  notice. It does not create an entry, prompt for a name, or
  read provider state.
- **No diff / preview.** There is no before/after view of the
  config file.
- **No undo / redo.** `Esc` discards the working copy; there
  is no per-tab history.
- **No `/settings reset` / per-tab revert / per-row edit.**
- **No migration** of existing user `config.toml` files.
  `#[serde(default)]` on `ui` makes the new field a no-op when
  absent.
- **No external plugin dialog for settings.** The dialog is
  host-owned; only the command surface is contributed through
  the plugin catalog.
- **No audio playback or theme loading.** Items are stored as
  names/booleans/durations only.

## Capabilities

### New capability

- **`ui-settings`**: open, navigate, and persist settings
  selections through the TUI modal. Owned entirely by the
  presentation layer + the binary's persistence helper.
  Specs will live at `.agents/sdd/specs/ui-settings/spec.md`
  after this proposal is approved.

### Modified capabilities

- None. `providers-ui`, `usage-history`, etc. are untouched.
  If a later change wires `[ui.theme]` into the renderer, that
  change will modify those capabilities explicitly.

## Approach

Five high-level steps (detailed design follows in `design`):

1. Add `SettingsDialog` and a `settings: Option<SettingsDialog>`
   field to `UiState`; teach `View::snapshot` and `draw_frame`
   to project it; teach `Driver::input` to dispatch it ahead
   of `plugin_dialog`; teach `request_decision` to dispose of
   `settings` alongside `plugin_dialog` and `workflow`.
2. Add the host-side short-circuit in `Ui::dispatch_command`
   (and a matching entry in `refresh_completion`) so
   `/settings` opens the dialog and is never sent to the model.
3. Add `UiConfig` (with five sub-tables, all
   `#[serde(default)]`) to `apps/huginn/src/config.rs`;
   add `toml_edit` to the workspace in root `Cargo.toml` and
   to `apps/huginn/Cargo.toml`.
4. Add `apps/huginn/src/ui_settings.rs` with `persist(&UiConfig,
   &Path)` that patches the existing `config.toml` via
   `toml_edit::DocumentMut`, preserving comments, ordering, and
   foreign tables. Wire it as the closure passed into
   `Ui::new`/`Ratatui::with_ui_config`.
5. Add the three test targets above; run `just ci-local`.

## Key Questions for User

None. Decisions D1–D9 are closed by the orchestrator and
incorporated verbatim in the Scope and Approach above. If the
`design` phase discovers a technical blocker (e.g. a conflict
between `deny_unknown_fields` and a hand-edited `[ui]` table with
unknown inner keys), that is the right phase to surface it.

## Risks

| Risk                                                                                                                                                                                                                                 | Likelihood | Mitigation                                                                                                                                                                                                                                                                                     |
|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `Config` has `#[serde(default, deny_unknown_fields)]` at the struct level but `ui` deserialization must be tolerant of inner unknowns written by users. Today a user-typed `unknown = true` under `[ui]` would break `Config::load`. | Medium     | The `ui` field uses `#[serde(default)]` and the inner sub-structs also derive `Default`. We make `ui` deliberately **not** `deny_unknown_fields` even though `Config` is — confirmed in `config.rs:7`. The test suite adds a fixture with `[ui] unknown_key = true` to confirm it still loads. |
| `toml_edit` is not currently a workspace dependency; `apps/huginn` only depends on `toml` (`Cargo.toml:29`). Adding it touches the root `Cargo.toml` and `apps/huginn/Cargo.toml`.                                                   | Low        | Reserved decision for the `design` phase: either add `toml_edit.workspace = true` at root or pull it transitively. The proposal just commits to the approach.                                                                                                                                  |
| `/settings` falls through and is sent to the model as a prompt because `dispatch_command` returns `false`.                                                                                                                           | Medium     | Host short-circuit is in `Ui::dispatch_command` (not just `Driver::input`), matching the `/quit` precedent at `state.rs:400–416`. Test asserts that `/settings` clears the editor without calling `try_send`.                                                                                  |
| Approval modal arrives while settings is open and the wrong one wins (or both stay open).                                                                                                                                            | Medium     | `request_decision` already disposes of `plugin_dialog` and `workflow` at `state.rs:675–676`; the change extends that to also dispose of `settings`, in the same locked-D8 precedence: `Modal > Settings > PluginDialog > completion`.                                                          |
| `Esc` in settings is misrouted to `cancel turn` (`terminal.rs:905–909`).                                                                                                                                                             | Medium     | The settings branch is its own arm in `Driver::input`, returns `true` after clearing `settings`; it never reaches the cancel-turn path. Test asserts `Esc` in settings does **not** cancel the running turn.                                                                                   |
| `apply` overwrites hand-written keys outside `[ui]`.                                                                                                                                                                                 | Medium     | The persistence helper uses `toml_edit::DocumentMut::patch`, replacing only the `[ui]` table. Round-trip test loads a fixture with `[provider]`, `[model]`, custom comments, applies `[ui.theme]`, asserts the bytes outside `[ui]` are identical.                                             |
| `(+) Add provider…` selected in `integrations` crashes or hangs.                                                                                                                                                                     | Low        | Spec says the only effect is a notice `Add provider not implemented yet`. No I/O, no channel send, no file read.                                                                                                                                                                               |
| `SettingsDialog::apply` writes through the closure while holding the `Ui` mutex; if the closure does I/O it could stall.                                                                                                             | Low        | The closure is invoked under the UI mutex the same way `choose_plugin_dialog` is. A slow disk would still block the UI; we accept the same trade-off as today and document it. A future change could move it to a background task.                                                             |
| Five tabs of mostly-TODO items look like an unimplemented UI.                                                                                                                                                                        | High       | This is the intended v1 contract (a shell for follow-ups). Out-of-scope list above is the canonical reference; the spec names the contract explicitly so future readers do not misinterpret persistence as live behavior.                                                                      |

## Success criteria

A reviewer can tick every box:

1. `cargo test -p huginn-presentation` passes, including the new
   `tests/settings.rs` covering tab cycling, selection cycling,
   `apply` building the expected `UiConfig`, `esc` not mutating,
   and a PTY render test for the header/marker/footer.
2. `cargo test -p huginn` passes, including a new round-trip
   test under `apps/huginn/tests/` that patches a real
   `config.toml` fixture via `toml_edit`, asserts `[ui.*]` is
   updated, and asserts unrelated tables + comments are byte-stable.
3. `/settings` appears in the `/`-completion catalog alongside
   `/help`, `/compact`, `/summarize`, `/quit`, with description
   `TUI settings`. Test in
   `crates/huginn/presentation/tests/plugins.rs`.
4. A `config.toml` with no `[ui]` section loads and yields the
   documented defaults (one-dark theme, bell off, toasts info/3s,
   all indicators on, integrations empty).
5. A `config.toml` with an existing `[ui]` section loads with
   those values, and `apply` in the dialog rewrites only the
   touched sub-table.
6. `Esc` closes the dialog without writing to disk and without
   cancelling the running turn.
7. `Enter` writes through the injected closure, shows notice
   `Settings saved`, and closes the dialog. A failing closure
   surfaces the error in the dialog and does **not** close it.
8. `just ci-local` (per AGENTS.md) is green.
9. `apps/huginn/config.example.toml` documents the `[ui]`
   schema.

## Next phase

`sdd-spec` writes delta specs for the new capability
`ui-settings` at `.agents/sdd/specs/ui-settings/spec.md`. Scenarios
will cover:

- Opening `/settings` from chat mode.
- Tab cycling with `←/→` and `Tab/BackTab`.
- Item cycling with `↑/↓` for each of the five tabs (theme,
  indicators, sound, toasts, integrations).
- `Enter` applying the working copy and showing the saved notice.
- `Esc` discarding the working copy without touching disk.
- Round-trip TOML with `Config::load` + patch + reload.
- Modal preemption of an open settings dialog.
- `(+) Add provider…` selecting a notice, no mutation.
- `/settings` appearing in the completion catalog with the right
  description.

No technical design or task breakdown is written in this
proposal phase.
