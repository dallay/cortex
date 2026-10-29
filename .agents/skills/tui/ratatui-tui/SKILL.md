---
name: ratatui-tui
description: Use when building, reviewing, debugging, or testing Huginn's Ratatui terminal interface (crates/huginn/presentation, inline rendering, PTY tests).
allowed-tools: Read, Edit, Write, Glob, Grep, Bash
metadata:
  author: cortex
  version: "1.0"
---

# Ratatui TUI — Huginn

Operational guidance for the interactive terminal in `crates/huginn/presentation`.
It complements `docs/huginn/adr/0010-ratatui-plugin-first-interactive-terminal.md`,
[ADR-0012](../../../../docs/huginn/adr/0012-hexagonal-application-and-plugin-lifecycle.md)
and `.agents/contexts/huginn.md`; it never overrides them.

## Stack (verify before assuming)

| Piece | Value | Source of truth |
|---|---|---|
| TUI framework | `ratatui = "=0.30.2"` | `crates/huginn/presentation/Cargo.toml` |
| Terminal backend | `crossterm = "=0.29.0"` | same |
| Width math | `unicode-width = "0.2"` | same |
| PTY tests | `portable-pty = "=0.9.0"` (dev) | same |
| Runtime | Tokio | workspace |
| Viewport | `Viewport::Inline(height)` | `terminal.rs` |

Versions are pinned on purpose. Do not bump them inside a feature change, and do
not swap the framework: ADR-0010 commits to Ratatui and to stock APIs first.

## Hard contracts

These are invariants, not style preferences.

1. **One terminal owner per process.** `TerminalGuard::acquire` uses an
   `AtomicBool` (`OWNER`) and fails closed with `terminal already owned`. Never
   open a second reader, event stream or `Terminal` in the same process.
2. **The host owns authorization.** Only `HostPolicy` approves effects. Views
   and plugins render proposals and collect input; they never grant permission.
   A stale or pre-modal keystroke must not approve anything.
3. **Plugins contribute declaratively.** They register commands, renderers,
   dialogs and notifications through `UiService`. They must not write escape
   sequences, seize `stdin`, install key handlers or read the TTY.
4. **Restoration is guaranteed.** Raw mode, bracketed paste and cursor visibility
   are restored by `TerminalGuard::restore` from `Drop` and from the panic hook.
   New failure paths need the same guarantee.
5. **The inline frame owns the screen.** History is rendered inside the inline
   viewport today. That is a deliberate trade-off; do not switch to native
   scrollback (`insert_before`, `Viewport::Fullscreen`) without an explicit
   decision and PTY coverage.
6. **`safe()` before display.** Any model, plugin or external text passes through
   `state::safe`, which removes control characters and bidi overrides while
   keeping `\n` and `\t`.

## Where responsibilities live

| Path | Owns |
|---|---|
| `src/terminal.rs` | TTY lifecycle, `TerminalGuard`, `HostBackend`, frame loop, layout, wrapping, markdown rendering |
| `src/state.rs` | `Ui`, `UiState`, `View` snapshot, `safe`, composer/editor, approvals, scroll state |
| `src/overlay.rs` | Overlay/focus stack, tickets and leases |
| `src/event.rs` | Projection of core `Event`s into UI state |
| `src/input.rs` | Unix input reader, paste assembly, revocation |
| `src/contributions.rs` | Built-in plugin using the public contribution API |
| `tests/pty.rs`, `tests/support/pty.rs` | End-to-end terminal behavior in a real PTY |

`terminal.rs` and `state.rs` are large because they concentrate several
responsibilities. Do not split them by line count; split along the boundaries
above, and only when the split removes real coupling. Any move must keep
behavior covered by tests.

## Rendering model

Ratatui is immediate-mode: each frame builds widgets, the terminal diffs cells
and writes only the changed ones. Therefore:

- Never print with `println!`, `eprintln!` or `execute!` from render code. Only
  the driver owns stdout.
- Do not hand-roll escape sequences or cursor arithmetic. `HostBackend` already
  tracks the cursor because Ratatui's initial cursor query must not restart an
  input parser.
- Build `Line`/`Span`/`Paragraph`/`List` widgets; do not concatenate ANSI.
- Work happens on a coalesced tick (currently ~33 ms). Keep frame work
  proportional to what is visible, not to total conversation length.

### Known cost: history is rewrapped every frame

`draw_frame` flattens `view.history` through `wrapped(...)` and then extends it
with the wrapped tail before selecting the visible window
(`DALLAY-665`). Cost grows with conversation length.

If you address it:

1. Measure first (frame time with a long transcript, not guesswork).
2. Cache wrapped lines keyed by content revision **and** width; invalidate on
   `Resize`, config/theme changes and history mutation.
3. Keep the visible-window selection O(visible), not O(history), or document
   why that is not yet possible.
4. Add a regression test for resize invalidation and for appended messages.

Do not add a cache without an invalidation test: a stale cache is worse than the
current cost.

## Layout

- Minimum accepted terminal is 20x8; `open_workflow` rejects smaller sizes with
  a message pointing to `--line-mode`. Preserve that escape hatch.
- The composer is `Length(4)`, or `Length(3)` when a completion popup is open and
  the area is shorter than 8 rows. The transcript gets `Min(1)`.
- Clamp scroll to `lines.len().saturating_sub(height)`. Never let the viewport
  scroll past the last line or jump when content growth outpaces the offset.
- Test at 20x8, 40x10, 80x24 and 120x40. Include a resize while a stream is in
  flight and while a modal is open.
- Prefer `Layout` constraints over absolute coordinates. Popups must be bounded
  by their parent area.

## Unicode and text

- Width is terminal cells, not bytes and not `chars().count()`. Use
  `UnicodeWidthStr::width` / `UnicodeWidthChar::width`.
- `wrapped()` currently iterates `chars()` and sums widths per char with tabs
  expanded to four spaces. That is adequate for simple width math but not for
  grapheme clusters: ZWJ emoji, variation selectors, skin-tone modifiers,
  combining marks and some scripts can occupy a different number of cells than
  the sum of their scalar widths.
- If you need cluster-accurate wrapping, do not write a new algorithm inline:
  evaluate `unicode-segmentation` (or Ratatui's text utilities) as a deliberate
  dependency, and cover it with width tests.
- Preserve zero-width characters that are part of a cluster; never split a
  cluster across lines. Handle `\n`, `\t` and a width of 1 column explicitly.
- Escape/sanitize untrusted text with `safe()`; keep bidirectional overrides out
  of the display path.

## Input, focus and approvals

- Focus and modal precedence live in the overlay stack (`overlay.rs`), not in
  ad-hoc booleans. A plugin can propose an action; the host arbitrates.
- Bracketed paste is assembled by `input.rs`. Fragments must not cross a focus
  boundary, a closed form, or a plugin unload; queued input must be discarded
  with the surface that owned it.
- Approvals bind to a specific, current request. Test that stale confirmations,
  pasted codes and keys buffered before a modal cannot approve.
- Secrets are zeroized on drop and masked in output; errors must not echo them.

## Lifecycle and failure

- Initialize before spawning readers: guard → raw mode → bracketed paste →
  `Terminal::with_options`. Partial initialization is cleaned by `Drop`.
- On exit: clear the inline area, show the cursor, restore raw mode and paste,
  then release `OWNER`. Keep the sequence in `Driver::drop`/`TerminalGuard`.
- Cancellation, plugin unload and panics must all end in a usable terminal.
  Recovered plugin panics are captured, not allowed to tear down the TUI.
- Keep `--line-mode` and headless modes working; the TUI is one presentation
  adapter, selected by the composition root.

## Testing

PTY tests are the contract for terminal behavior. Unit tests cannot prove
restoration, resize or scrollback behavior.

```bash
cargo test -p huginn-presentation --all-features
cargo test -p huginn-presentation --all-features --test pty
just huginn-architecture
```

Rules:

- Extend `tests/support/pty.rs` and the `test-driver` fixture instead of
  spawning processes ad hoc. Drive the app through keys and assert on the
  rendered screen and termios, not on timing sleeps.
- Prefer deterministic waits on output/state. If a timeout is unavoidable, make
  it generous and explain why.
- Cover, at minimum, when the change touches them: resize during streaming,
  resize with a modal open, terminal restore after error/cancel/panic, stale
  approval rejection, secret masking, paste fragmentation, plugin mount/unload,
  and 20x8 rendering.
- Add Unicode cases: ZWJ emoji, tone modifiers, combining marks, CJK, a single
  column, and a long token with no spaces.
- Do not snapshot raw ANSI without normalizing cursor-position queries and
  backend-specific sequences.

## Anti-patterns

- Manual printing or cursor movement outside the driver.
- A new Tokio task per frame, or locks held across `.await` in the render path.
- Collecting the whole history into widgets when only a window is visible.
- Indexing strings by byte offset or assuming one char equals one cell.
- Blocks that panic on tiny terminals instead of clamping.
- Adding a widget that reads `stdin` or sets its own raw mode.
- "Fixing" a flaky PTY test by removing the assertion, adding `sleep`, or
  loosening it to always pass.

## Definition of done

- [ ] Contract above preserved; no second terminal owner or authorization path.
- [ ] Focused unit tests plus PTY coverage for the affected interaction.
- [ ] Resize and minimum-size behavior verified.
- [ ] Unicode/width behavior covered for the text paths touched.
- [ ] `cargo test -p huginn-presentation --all-features` and `just huginn-architecture` pass.
- [ ] Rendering cost considered and, if changed, measured before/after.
- [ ] `--line-mode` still works; no machine-readable output changed.
- [ ] Follow-ups recorded instead of silently expanding the change.
