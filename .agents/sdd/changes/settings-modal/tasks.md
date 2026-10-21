# Tasks: Settings Modal in Huginn TUI

> **Change**: `settings-modal`
> **Phase**: 5 — Tasks
> **Artifact store**: `.agents/sdd/`
> **Owner**: presentation crate + binary helper (see `proposal.md` + `design.md`)

## Project Test Mode and Runners

- `.agents/sdd/config.yaml` sets `strict_tdd: false`; the global RED→GREEN→REFACTOR cycle is **not** enabled. Tests below are functional: each task is paired with the focused `cargo test` command that verifies it; the project-wide gate is `just ci-local`.
- Focused Rust runner: `cargo test -p <package>` for unit tests and `cargo test -p <package> --test <target> [filter]` for integration targets. Workspace gate: `cargo test --workspace --all-features`.
- The change adds new test targets: `crates/huginn/presentation/tests/settings.rs`, `.../tests/dispatch_settings.rs`, `.../tests/preemption.rs`, `apps/huginn/tests/ui_settings.rs`, plus two PTY fixtures appended to `crates/huginn/presentation/tests/pty.rs` and one host catalog entry appended to `crates/huginn/presentation/tests/completion.rs`.
- No `.agents/sdd/quality-runner.json` or `scripts/sdd-quality-runner.mjs` is configured; do not claim deterministic SDD runner enforcement.
- Markdown lint for tasks.md is `pnpm exec markdownlint-cli2 ".agents/sdd/changes/settings-modal/tasks.md"` (this file). The full documentation set is gated by `just ci-lint`.

## Tareas

Numbering follows the dependency order the design implies (deps → state → Ui fields → host short-circuit → catalog → view → input → config + persistence → wiring → docs → gate).

- [ ] **T-01 — Add `toml_edit` to the workspace and to `apps/huginn`.**
  Update `[workspace.dependencies]` in root `Cargo.toml` to add `toml_edit = "0.22"` (matching the version `toml 1.x` already pulls in transitively at `Cargo.lock:4689`). Update `apps/huginn/Cargo.toml` to add `toml_edit.workspace = true` in `[dependencies]`. **No behaviour change yet**; runtime code still uses `toml`.
  - **Verificación**: `cargo check -p huginn` (compiles cleanly because the workspace pins the version).
  - **REQ cubierto(s)**: REQ-09 (sets up the dependency used by the persistence helper).
  - **Riesgos específicos**: Bumping `Cargo.lock`. The pin to `0.22` matches the version `toml 1.x` already pulls in transitively, so the lockfile should not change dramatically. CI rejects duplicate major versions of `toml` family crates, so re-confirm with `cargo update -p toml_edit --dry-run` shows no version jump.

- [ ] **T-02 — Create `crates/huginn/presentation/src/settings.rs` with DTOs and state machine.**
  Define `UiConfig`, `UiTheme`, `UiSound`, `UiToasts`, `UiIndicators`, `UiIntegrations` with `#[derive(Deserialize, Serialize, Default, Clone, Debug, PartialEq, Eq)]` and `#[serde(default)]` on the struct and each sub-struct. **Do NOT** carry `deny_unknown_fields` on any of them (REQ-11). Non-empty defaults are built explicitly so `UiConfig::default()` matches the spec: `theme.name = "one-dark"`, `indicators.{scrollbar, breadcrumbs, line_numbers} = true`, `sound.bell = false`, `toasts.level = "info"`, `toasts.duration_secs = 3`, `integrations.entries = Vec::new()`. Define `SettingsDialog`, `WorkingCopy`, `Working*` structs, `SettingsSnapshot`, `SettingsItem`, `ItemKind { Pick, Bool, Cycle, Action }`, and the constants `TAB_NAMES`, `TAB_COUNT`, `TAB_THEME`, `TAB_INDICATORS`, `TAB_SOUND`, `TAB_TOASTS`, `TAB_INTEGRATIONS`. Implement `new`, `next_tab`, `prev_tab`, `select_next`, `select_prev`, `toggle_selected` (Bool flips the bit; Cycle advances; Pick selects the working copy value; Action no-ops), `projected_ui_config`, `apply`, `discard`, `handle_add_provider`, `snapshot`. Cycle mapping implemented here: `info → 0, warn → 1, error → 2` for `level`; `3 → 0, 5 → 1, 10 → 2, off → 3` for `duration_secs`. Add `pub mod settings;` in `crates/huginn/presentation/src/lib.rs`.
  - **Verificación**: `cargo check -p huginn-presentation` and `cargo test -p huginn-presentation --test settings` for tab cycling, item cycling, `projected_ui_config` matching `UiConfig::default()`, and `discard` not mutating. Keep these tests in the separate `tests/settings.rs` target.
  - **REQ cubierto(s)**: REQ-02, REQ-03, REQ-04, REQ-05, REQ-06, REQ-07, REQ-08, REQ-10, REQ-14. (The behavior is encoded; the dedicated scenarios are exercised in the integration tests added later.)
  - **Riesgos específicos**: This file declares the canonical struct layout referenced by every other task. Get the derive set right (`Eq` is fine for all sub-structs because each value is a string / bool / `u32` / `Vec<String>`; do **not** derive `Eq` on `SettingsDialog` because the working-copy bit fields don't need it and the trait can restrict future evolution). Add `#[must_use]` selectively on the cycle helpers if compile-time nudges are useful, but it's optional.

- [ ] **T-03 — Add `settings` and `settings_focus_epoch` to `UiState`.**
  In `crates/huginn/presentation/src/state.rs`, alongside the existing `pub workflow: Option<...>` (line 147) and `pub workflow_focus: u64` (line 149), add `pub settings: Option<huginn_presentation::settings::SettingsDialog>` and `pub settings_focus_epoch: u64`. Update `UiState`'s `#[derive(Default)]` impl so the two new fields default to `None` / `0` (the existing derive already covers this; no manual `Default` change required unless the type isn't `Default`).
  - **Verificación**: `cargo check -p huginn-presentation`.
  - **REQ cubierto(s)**: REQ-12 (state shape prerequisite).
  - **Riesgos específicos**: The placement determines which input lease branch wins. The convention is `Modal > Settings > PluginDialog > completion` — this ordering is enforced in T-08, but the field placement in `UiState` must precede that work. Do **not** nest the field inside a sub-module's re-export; import `SettingsDialog` directly from `crate::settings`.

- [ ] **T-04 — Extend `Ui` and `Ui::new` with `ui_config` and `save_ui_config`.**
  Add two new `pub` fields on `Ui` (`pub ui_config: Arc<UiConfig>`, `pub save_ui_config: Arc<dyn Fn(&UiConfig) -> anyhow::Result<()> + Send + Sync>`). Extend `Ui::new`'s signature to accept them and initialize them. Extend the cleanup block at `state.rs:675–676` (where `state.plugin_dialog.take()` and `state.workflow.take()` run on `request_decision`) to also `state.settings.take()` and bump `state.settings_focus_epoch`. Update the single internal call site where `Ui::new` is invoked (the `Ratatui::activate`/`terminal::Ratatui` constructor path). Existing tests must still pass.
  - **Verificación**: `cargo check -p huginn-presentation`. Then `cargo test -p huginn-presentation --tests` (existing test targets still pass; only the constructor signature changed).
  - **REQ cubierto(s)**: REQ-07 (closure type), REQ-12 (preemption path now disposes `settings`).
  - **Riesgos específicos**: The `Ui::new` signature change forces every call site (one in `terminal.rs`, possibly test calls in `tests/` if any) to compile. Search for `Ui::new` across the workspace before applying. The cleanup edit at `state.rs:675` is **inside** the lock acquisition block; do not introduce a new lock attempt there — extend the existing scoped lock.

- [ ] **T-05 — Add `/settings` host short-circuit in `Ui::dispatch_command`.**
  Add the new branch per the design's §Input Lease Placement, **before** the `contributions.command(...)` lookup at `state.rs:367`. The branch matches the trimmed head `"/settings"`; when matched, opens `state.settings = Some(SettingsDialog::new(&self.ui_config))`, bumps `state.settings_focus_epoch`, notifies `self.dirty`, and **always** returns `true` — even when a higher-precedence surface (modal or settings already open) blocks the dialog from appearing, so `/settings` is consumed and never sent to the model as a prompt. Acceptance: existing `tests/extensions.rs` and `tests/dispatch_settings.rs` (when added) pass.
  - **Verificación**: `cargo check -p huginn-presentation`. Existing `cargo test -p huginn-presentation` still passes. The new dedicated `tests/dispatch_settings.rs` target lands during apply; this task only adds the production branch.
  - **REQ cubierto(s)**: REQ-01, REQ-12.
  - **Riesgos específicos**: `dispatch_command` is the system-wide `/`-prefix entry point; ordering matters. The match must be **first** (before the existing `contributions.command` call) so a future plugin can never shadow `/settings`. Document this in a code comment per the design.

- [ ] **T-06 — Add `/settings` to the completion catalog in `Ui::refresh_completion`.**
  Add `CatalogCommand::host("/settings", "TUI settings")` to the unconditional `commands.extend([...])` block at `state.rs:407–410` — **outside** the `if model_configured` block, next to `/quit` and `/exit`, per REQ-16 ("regardless of model configuration state"). When wiring the helper, confirm the existing `state.modal.is_some() || state.plugin_dialog.is_some() || state.workflow.is_some()` early-return at line 430 still preserves the catalog generation during an open dialog.
  - **Verificación**: `cargo check -p huginn-presentation` and `cargo test -p huginn-presentation --test completion` (the new `settings_host_completion_catalog` test lands during apply; this task adds the entry only). Existing `tests/completion.rs` and `tests/plugins.rs` continue to pass.
  - **REQ cubierto(s)**: REQ-16.
  - **Riesgos específicos**: The catalog is keyed by `(name, generation)`; using `CatalogCommand::host` (not `contributed`) ensures `/settings` does not appear alongside plugin-contributed commands in `commands()` and is not overwritable by a plugin.

- [ ] **T-07 — Wire settings into `View::snapshot` and `draw_frame`.**
  Add `settings: Option<(String, [&'static str; 5], usize, Vec<(String, bool, u8)>, usize, Option<String>)>` to the `View` struct (`terminal.rs:354`). The tuple mirrors `(title, tabs, active, items, selected, error)`. Extend `View::snapshot` (`terminal.rs:367`) to project `state.settings.as_ref().map(|d| d.snapshot())` into the new field, converting `Cow<str>` labels to owned `String`s for the renderer. Add the render block in `draw_frame` (`terminal.rs:482`) per the design's §Render Layout: a `Layout::vertical([Length(1), Min(1), Length(1)])` slice, a tab-header line with the active tab rendered with `Style::default().reversed()`, a bordered body `Paragraph` titled `Settings` containing rows in the format `> label ✓` (or `> label` when `checked` is false), and the literal footer `↑↓ select · ←→ section · ↵ apply · esc close`. Place the new branch between the `workflow` arm (line 528) and the `plugin_dialog` arm (line 530).
  - **Verificación**: `cargo check -p huginn-presentation` and `cargo test -p huginn-presentation --test pty` (the dedicated `settings_pty_render_header_and_marker` fixture lands during apply; this task wires the production side).
  - **REQ cubierto(s)**: REQ-02 (modal structure).
  - **Riesgos específicos**: `View`'s field count grows by one; every pattern that builds or destructures a `View` must compile. The `Cow`→`String` conversion is per-frame, but the dialog's snapshot already allocates `Vec<SettingsItem>`; the `String` is small (a single label). Acceptable; documented in the design.

- [ ] **T-08 — Wire settings input lease into `Driver::input`.**
  Route through the host's lease-checked `Ui::overlay_input` and settings payload, preserving approval precedence (REQ-12). Reduce input to effects under the `UiState` lock; execute effects after releasing it. The settings branch:
  1. Validates the captured transition ticket and focus lease, then reads pending state by comparing `settings_apply` with the active `OverlayId`.
  2. Matches `KeyCode::Esc` → `Effect::DiscardSettings(pending)`; dismiss through `close_overlay_with_message(ticket, focus, ...)`, with notice `Settings discarded` when idle or `Settings closed; the started write may still complete` when pending. Do not cancel an already-submitted write. While pending, consume other keys without editing or submitting again.
  3. Matches `KeyCode::Left | KeyCode::BackTab` → `dialog.prev_tab()`.
  4. Matches `KeyCode::Right | KeyCode::Tab` → `dialog.next_tab()`.
  5. Matches `KeyCode::Up` → `dialog.select_prev()`.
  6. Matches `KeyCode::Down` → `dialog.select_next()`.
  7. Matches `KeyCode::Enter` → if `dialog.handle_add_provider()`, emit `Effect::AddProvider` to close and notice `Add provider not implemented yet`; otherwise emit `Effect::ApplySettings`. After releasing the state lock, call `apply_settings_at(ticket)`. It revalidates the ticket and shutdown state, captures `projected_ui_config()`, marks the overlay ID pending, and queues one receipt and write-tail dependency under the lock, suppressing duplicate submissions. Run `save_ui_config` through `spawn_blocking` outside the lock after the previous write finishes; preserve this queue across dismissal and overlay replacement.
  8. Unmatched keys return without falling through (consume the event).
  Process `SettingsReceipt { id: OverlayId, config: UiConfig, result }` in submission order via `poll_settings_receipts()`, stopping at the first unfinished receipt. Clear pending only for the matching ID. Success updates `saved_ui_config`; close and notice `Settings saved` only when the receipt's ID is still current and pending. Failure sets `last_error` only for that same current ID, preserving its working copy and allowing retry (REQ-07). Late receipts cannot close, mutate, or notify a replacement overlay. Esc always dismisses locally and never reaches cancel-turn (REQ-08, REQ-13); an already-submitted write may complete.
  - **Verificación**: `cargo check -p huginn-presentation` and `cargo test -p huginn-presentation --test state settings_`. The host state tests cover duplicate suppression, callback execution outside `UiState`, failure/retry, stale receipts after dismissal/replacement, approval preemption, and serialized writes in submission order.
  - **REQ cubierto(s)**: REQ-03, REQ-04, REQ-06, REQ-07, REQ-08, REQ-13, REQ-14.
  - **Riesgos específicos**: Preserve the host overlay router's approval precedence and ticket checks (REQ-12). Never call `dialog.apply` from the input handler under `UiState`. Pending state must remain dismissible; queue lifetime must be independent of overlay lifetime. Test that approval preemption and replacement prevent old receipts from changing the current overlay.

- [ ] **T-09 — Add `UiConfig` to `Config` and update `Config::load`.**
  In `apps/huginn/src/config.rs`, add `pub ui: UiConfig` to `Config` (importing `huginn_presentation::settings::UiConfig`). Annotation: `#[serde(default)]` so a missing or partial `[ui]` table deserializes cleanly. **Confirm** the top-level `Config` keeps its existing `#[serde(default, deny_unknown_fields)]` (line 7) — the new `ui` field does **not** inherit that behaviour because it is a sub-deserializer. Update `Config::default()` to populate `ui: UiConfig::default()`. `Config::load` (line 134) needs no behavioural change; the new field deserializes transparently because `toml::from_str::<Self>` reads through the struct shape.
  - **Verificación**: `cargo check -p huginn`. Existing `cargo test -p huginn --test config_compat` still passes.
  - **REQ cubierto(s)**: REQ-10, REQ-11.
  - **Riesgos específicos**: `Config` carries `deny_unknown_fields` at the **struct** level — annotating `pub ui` with the same would force `UiConfig` to also reject hand-added keys. Use only `#[serde(default)]` on the `ui` field; the inner structs (`UiConfig`, `UiTheme`, ...) already carry their own `#[serde(default)]`, which `deny_unknown_fields` does **not** propagate to child deserializers when the parent struct is itself `default`-tolerant. The new `apps/huginn/tests/ui_settings.rs::config_load_with_inner_unknown_key` test is the regression.

- [ ] **T-10 — Create `apps/huginn/src/ui_settings.rs` with `persist(&UiConfig, &Path)`.**
  Implement the function per the design's §Persistence Strategy: open the file with `std::fs::read_to_string`, parse into `toml_edit::DocumentMut::from_str` (or `DocumentMut::new()` if the file is absent), upsert the `[ui]` table and each sub-table (`theme`, `sound`, `toasts`, `indicators`, `integrations`) and every leaf key, then `atomic_write` via tmp+rename. The `ensure_table` helper handles the absent-present-table cases. Surface errors via `anyhow::Result<()>` with `with_context`. Add `mod ui_settings;` in `apps/huginn/src/main.rs`.
  - **Verificación**: New `apps/huginn/tests/ui_settings.rs` round-trip target covers: defaults on missing, partial fill, inner unknown tolerance, byte-stability of unrelated tables + comments, byte-stability of inner unknown keys, and atomic-write on missing path. Run `cargo test -p huginn --test ui_settings`.
  - **REQ cubierto(s)**: REQ-09, REQ-10, REQ-11.
  - **Riesgos específicos**: `toml_edit::Item::Value` and `Array::from(...)` API surface is 0.22. Use `from_str` on `&str`, not `parse` (the public API exposes both — `from_str` is more explicit). The atomic-write tmp path uses `path.with_extension("toml.tmp")` which replaces `config.toml` with `config.tmp`; this stays inside the same filesystem (atomic rename) and is restored on failure because the rename has not run yet.

- [ ] **T-12 — Wire `apps/huginn/src/main.rs`.**
  After `Config::load`, resolve the canonical config path (use the same `dirs::config_dir().unwrap_or_else(...)` chain as `Config::load` does), build `let ui_config = Arc::new(config.ui.clone())`, then build the closure `let save_ui_config: Arc<dyn Fn(&UiConfig) -> anyhow::Result<()> + Send + Sync> = { let path = config_path.clone(); Arc::new(move |ui| ui_settings::persist(ui, &path)) }`. Pass both `Arc`s into `Ratatui::activate` (via a new `RatatuiPlugin::with_settings(ui_config, save_ui_config)` builder method) so they flow through `terminal::Ratatui` into `Ui::new`. Existing `tests/state.rs` and `tests/cli.rs` (if any) must still pass. Apply passes only the wiring; tests in `apps/huginn/tests/state.rs` exercise the running binary end-to-end.
  - **Verificación**: `cargo check -p huginn` and `cargo test -p huginn`. An end-to-end smoke test (state.rs) still passes; `Config::load` + closure build do not regress.
  - **REQ cubierto(s)**: REQ-01 (wires the `/settings` short-circuit into the real binary), REQ-07 (passes the save closure into `Ui`).
  - **Riesgos específicos**: The path resolution inside `main.rs` must match `Config::load`'s resolution at `apps/huginn/src/config.rs:140–145` — a mismatch would persist to a file the binary never reads on next start. Reuse the same chain exactly or extract a helper in `config.rs` (the design keeps them duplicated; review later). The `Arc<dyn Fn(...) + Send + Sync>` requires `+ Send + Sync` because the serialized writer invokes it through `spawn_blocking` outside the `UiState` lock.

- [ ] **T-13 — Update `apps/huginn/config.example.toml` with the `[ui]` schema.**
  Add a commented block mirroring the locked D1 (nested sub-tables, one per tab). Each sub-table documents the keys the TUI may persist (`name`, `bell`, `level`, `duration_secs`, `scrollbar`, `breadcrumbs`, `line_numbers`, `entries`) plus their defaults and the cycle/enum possibilities. No real values are written (everything is `# …`); a commented template is the audit-friendly form per AGENTS.md's `Config::default` posture.
  - **Verificación**: `cargo check -p huginn` (the example file is not consumed by Rust, but the binary's example-rendering `tests/cli.rs` if any must not regress). Visually confirm the block matches the locked D1 against `proposal.md:37–55`.
  - **REQ cubierto(s)**: REQ-10 (documented defaults), REQ-11 (documented shape).
  - **Riesgos específicos**: None — the file is documentation only.

- [ ] **T-14 — Update `docs/huginn/README.md` with `/settings` and the `[ui]` section.**
  Add a subsection under "Chat suggestions" (or equivalent TUI section) documenting `/settings`, the five tabs, the navigation keys, and the `[ui]` config keys. Mirror the wording in `proposal.md`. Cross-link the TOML example to `apps/huginn/config.example.toml`.
  - **Verificación**: `just ci-lint` (per AGENTS.md). Existing markdown checks pass.
  - **REQ cubierto(s)**: REQ-15 (documents line-mode exclusion), REQ-16 (catalog inclusion).
  - **Riesgos específicos**: Per AGENTS.md, do not duplicate tool inventories; keep the subsection operational and reference-only.

- [ ] **T-15 — Run the full validation gate.**
  `just ci-local` from the canonical `justfile` is required before claiming the full CI gate passed. The Rust commands `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace --all-features` are focused checks only and are not an equivalent full gate. Fix any failures across the 14 preceding tasks. The focus during iterate is `cargo test -p huginn-presentation --test settings`, `--test dispatch_settings`, `--test preemption`, `--features test-driver --test pty`, `--test completion`, and `cargo test -p huginn --test ui_settings --test config_compat`.
  - **Verificación**: `just ci-local` green; report exactly which checks were run, which passed, and which were skipped (per AGENTS.md's "Report which checks you ran" rule).
  - **REQ cubierto(s)**: All 16 REQs (the gate is the audit point).
  - **Riesgos específicos**: CI gate can surface issues that focused tests missed. Pre-PR validation per AGENTS.md.

> **Note**: The orchestrator prompt listed 14 tasks ending with the validation gate; this file numbers the validation task **T-15** to maintain `T-NN` consecutiveness (skipping the prompt's 11→12 typo). All other names and contents match the orchestrator prompt verbatim.

## Verification matrix

| REQ | Tasks implementing it | How it is checked |
|-----|-----------------------|-------------------|
| REQ-01 — `/settings` registration | T-02, T-03, T-05, T-12 | `tests/dispatch_settings.rs::dispatch_opens_settings_dialog` and `tests/plugins.rs`/`tests/dispatch_settings.rs::dispatch_does_not_invoke_prompt_sender` |
| REQ-02 — Modal structure | T-02, T-07 | `tests/pty.rs::settings_pty_render_header_and_marker` |
| REQ-03 — Tab navigation | T-02, T-08 | `tests/settings.rs::settings_tab_cycling`; PTY next/prev |
| REQ-04 — Item navigation | T-02, T-08 | `tests/settings.rs::settings_item_cycling`; PTY up/down |
| REQ-05 — Tab content | T-02, T-07 | `tests/settings.rs::settings_snapshot_items_match_locked_d6`; `tests/pty.rs::settings_pty_render_header_and_marker` |
| REQ-06 — Working copy | T-02, T-08 | `tests/settings.rs::settings_esc_does_not_invoke_closure` |
| REQ-07 — Apply on Enter | T-02, T-04, T-08, T-10, T-12 | `tests/settings.rs::settings_apply_invokes_closure`; `tests/ui_settings.rs::persist_round_trip_byte_stable_outside_ui` |
| REQ-08 — Discard on Esc | T-02, T-08 | `tests/settings.rs::settings_esc_does_not_invoke_closure`; `tests/pty.rs::settings_pty_esc_does_not_cancel_running_turn` |
| REQ-09 — TOML round-trip | T-01, T-10 | `tests/ui_settings.rs::persist_round_trip_byte_stable_outside_ui`, `persist_creates_missing_file` |
| REQ-10 — Defaults | T-02, T-09, T-10 | `tests/ui_settings.rs::config_load_no_ui_section`, `config_load_partial_ui_section` |
| REQ-11 — Inner-unknown tolerance | T-02, T-09, T-10 | `tests/ui_settings.rs::config_load_with_inner_unknown_key`, `persist_preserves_unknown_inner_keys` |
| REQ-12 — Modal precedence | T-03, T-04, T-08 | `tests/preemption.rs::settings_modal_preempts_by_approval`, `settings_preempts_plugin_dialog` |
| REQ-13 — Esc cancels nothing | T-08 | `tests/preemption.rs::settings_esc_does_not_cancel_turn`; `tests/pty.rs::settings_pty_esc_does_not_cancel_running_turn` |
| REQ-14 — `(+) Add provider…` | T-02, T-08 | `tests/settings.rs::settings_add_provider_shows_notice` |
| REQ-15 — line-mode exclusion | T-14 | README documents `--line-mode` exclusion; no production branch reaches `dispatch_command` from `LinePrompt` |
| REQ-16 — Completion catalog | T-06 | `tests/completion.rs::settings_host_completion_catalog`; PTY completion assertion |

## Out of scope reminder (repeated for reviewers)

Locked out of v1 so the persistence surface is not misread as live behaviour. Any future change that wires a persisted value into a runtime subsystem **must** explicitly modify the `ui-settings` capability — not slip in here.

- Live theme application (persisting `[ui.theme]` does NOT switch the rendered palette).
- Live sound, toast, or indicator toggling (`[ui.sound]` / `[ui.toasts]` / `[ui.indicators]` are persisted but not consumed by their subsystems in v1).
- Reading provider state for the `integrations` tab (v1 only renders the `(+) Add provider…` TODO).
- `/settings reset`, per-tab revert, or per-row edit (Esc discards the entire working copy).
- Migration of existing user `config.toml` files (`#[serde(default)]` on `ui` makes it a no-op when absent).
- External plugin dialog for the settings body (host-owned end-to-end; the command surface is also host-owned).
- `--line-mode` integration (the line prompt never reaches `dispatch_command`).
- A `Config::save` method (the runtime writer is `ui_settings::persist` exclusively; no full rewrite).
- Adding a `watch::Sender<UiConfig>` notification (ordered successful receipts update the host's saved snapshot for future opens).

## Commit strategy

Use Conventional Commits, **no AI attribution**, per AGENTS.md. Group the 15 tasks into 7 logical commits. Each commit lands green (its focused tests pass) and is independent at the production level — though the dependency arrows listed above mean the order is mostly enforced.

| Commit | Tasks | Conventional title |
|--------|-------|---------------------|
| 1 | T-01, T-02 | `feat(huginn): add ui config DTOs and settings dialog state machine` |
| 2 | T-03, T-04 | `feat(huginn): inject ui config and persistence closure into Ui` |
| 3 | T-05, T-06 | `feat(huginn): register /settings host command` |
| 4 | T-07, T-08 | `feat(huginn): render and input lease for settings modal` |
| 5 | T-09, T-10, T-12 | `feat(huginn): persist [ui] section via toml_edit` |
| 6 | T-13, T-14 | `docs(huginn): document /settings and [ui] schema` |
| 7 | T-15 | `chore(huginn): ci-local validation gate` |

Suggested branch strategy: **single PR** under the existing default model. The change is ~14 atomic tasks with no parallel reviews needed; budget is well under 400 lines when commit messages and `design.md` are excluded. If a reviewer requests chunks, split at commit 4 (render/input) and commit 5 (persistence) because those are the two natural seams.
