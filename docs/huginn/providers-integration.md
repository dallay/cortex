# Providers integration and safe recovery

## Scope and reproducibility

DALLAY-674 validates the composition of the existing Providers contracts,
connection manager, built-in adapters and host-owned presentation workflows.
Fixtures use temporary directories, loopback services and sentinel credentials;
they need neither paid services nor real credentials. Do not pass credentials as
process arguments or reset a user's database to run these checks.

Run the baseline suites from the repository root:

```bash
cargo test -p huginn-application
cargo test -p huginn-runtime
cargo test -p huginn-runtime --test providers_integration
cargo test -p huginn-presentation
cargo test -p huginn-presentation --features test-driver --test pty
cargo test -p huginn --test providers_recovery
cargo test -p huginn --test providers_flow
cargo test -p huginn --test tui
cargo test -p huginn --test cli --test config_compat --test session_ownership
```

The application suite injects persistence failures before and after writes,
suspends operations at deterministic barriers, and checks recovery without
damaging unrelated connections. Its `connections` target includes restart,
rotation/revocation, locked/ephemeral stores, model-selection rollback and stale
manager generations. Presentation `workflows` tests cover late results and
cancelled forms; `providers_ui` tests cover host-owned secret entry. Runtime
`adapter_security` and `inference_adapters` test redirect rejection, transport,
catalogs and authorization isolation. These boundary tests complement, rather
than replace, the composed integration paths.

The default presentation suite does not enable the PTY fixture. Use the explicit
`test-driver` command above for terminal masking, fragmented paste, scrollback,
stale contributions and terminal cleanup coverage.

### Actual runtime loop

`providers_integration` composes `StandardLoop`, `ConnectionManager`, SQLite,
ephemeral secrets and the real built-in HTTP adapters. Each scenario runs against
both OpenAI-compatible and Ollama protocol fixtures:

- `metadata_and_inference_send_credentials_only_to_their_own_destination` captures
  catalog/capability and inference requests separately, checking destination auth
  and secret-free request bodies.
- `same_connection_model_change_is_deferred_through_tools_and_compaction` uses
  `ConnectionManager::select_model` during tools and automatic/manual compaction;
  credentials stay unchanged, the active turn retains its model and the next
  turn adopts the new model.
- `actual_tool_continuation_pins_connection_and_model_until_next_turn` executes a
  tool, changes selection at a barrier, verifies the tool-result continuation
  stays pinned, then checks that the next turn uses the other connection/model.
- `automatic_and_manual_compaction_use_pinned_http_connection` exercises actual
  summarization and selection changes during compaction.
- `selected_http_and_stream_errors_never_fall_back_or_leak_credentials` injects
  HTTP and SSE errors containing sentinels and verifies sanitized failures.
- `revocation_and_rotation_between_tool_iterations_block_old_credentials` checks
  that later iterations cannot use the old authorization after a tool barrier.
- `revocation_and_rotation_cancel_a_locally_active_http_stream` checks bounded
  local cancellation, not remote cancellation.
- `selection_during_http_stream_only_changes_the_next_turn` changes selection
  while a response is gated and checks the subsequent turn separately.

### Nine-step technical gate

1. **Open unconfigured:** executable `tui::first_run_opens_tui_without_configured_model`
   and `cli::doctor_reports_unconfigured_without_api_key_or_network`; application
   `providers_contracts::unconfigured_core_has_diagnostics_but_no_inference`.
2. **Discover adapters:** composed
   `providers_flow::ui_two_connections_select_switch_pin_revoke_and_persist_without_secret_leaks`
   invokes `/providers` against the built-in catalog.
3. **Configure without TOML or credential arguments:** the same composed test
   drives `/login` through host-owned forms into the real manager and secret store.
4. **Select a model:** that test drives `/model`, then pins through the real router
   and live fixture catalog. Invalid/missing catalogs remain covered by runtime
   `inference_adapters` tests.
5. **Run the first task:** the composed test infers through the UI-selected
   connection; executable
   `providers_recovery::run_resume_line_and_json_recover_the_same_durable_router`
   covers actual headless processes, line mode, resume and JSON.
6. **Change without restarting:** composed selection tests and the actual-loop
   scenarios above verify the current turn stays pinned and the next turn changes.
7. **Restart safely:**
   `providers_flow::restart_locked_or_ephemeral_is_visible_fail_closed_and_never_falls_back`
   reopens managers using a persistent store double or fresh ephemeral store;
   `providers_recovery::restarted_ephemeral_selection_is_unconfigured_and_never_calls_inference`
   launches a new executable. Neither certifies an OS secret backend.
8. **Logout:** the composed lifecycle test drives `/logout`, rejects old pinned
   authorization, and checks the other connection remains usable. The rotation
   test `ui_credential_rotation_retires_pinned_authorization_until_explicit_reselection`
   covers replaced credentials separately.
9. **Rollback and regressions:** composed
   `failed_secret_write_rolls_back_and_cancelled_form_cannot_publish` and
   `cancelling_pending_login_prevents_stale_completion_and_next_save_cleans_orphan`
   inject failures/cancellation; application `connections` and provider-contract
   suites cover publication rollback and generation retirement. CLI/configuration,
   JSON, presentation fallback and PTY suites remain part of the technical gate.

The composed tests scan captured public UI, metadata and session data, plus their
temporary database files, for sentinel secrets. Credentials are supplied through
the secret form/store, never command arguments. A failed secret write may retain
a cleanup intent until recovery; this must not become a usable connection.

## Evidence boundaries

- Automated local HTTP services verify deterministic protocol and routing
  behavior, not compatibility with an installed Ollama model.
- An injected persistent secret-store double verifies the application's backend
  contract and restart composition, not macOS Keychain or Linux Secret Service.
  Native backend checks must be reported separately, including locked/unavailable
  prerequisites and ignored tests.
- Explicit ephemeral mode intentionally loses credentials on restart. It must
  not silently downgrade a locked secure backend to plaintext persistence.
- In-flight cancellation is best-effort and local: it does not prove remote
  cancellation, stopped billing or remote credential revocation.
- Real Ollama observations and their limitations remain in
  [inference adapters](inference-adapters.md#reproducible-real-ollama-checks).
  They are P0.4 evidence and are not rerun by these fixtures.
- Full pre-PR validation is `just ci-local`, subject to the prerequisites in
  [development](../development.md#validación-pre-pr) and
  [container E2E](../../dev/README.md#path-2--playwright-e2e-against-docker).
  Focused Rust tests are not full CI.

This delivery does not add OAuth, new providers, Rook integration or dynamic
loading. It does not close the independent onboarding or QA acceptance issues.

To opt into the separate native backend contract on a suitably configured host:

```bash
cargo test -p huginn-runtime --test connections os_secret_backend_contract -- --ignored
```

That ignored test writes isolated test entries to the OS store. The default
automated suite deliberately does not require an unlocked personal keychain or
a Linux session D-Bus service.

## Validation record

Initial checks on October 16, 2026 used Darwin arm64 and Rust/Cargo 1.99.0.
The three requested package suites passed. The native OS secret backend contract
was ignored; the default presentation suite did not enable PTY coverage.
Running the separate `test-driver` PTY target exposed a queued-credential-output
failure (33 passed, one failed). That failure must not be counted as passing
terminal secret isolation. The fix quarantines old-focus input across a quiet
driver tick before releasing public focus, with a final-output regression check.
This addresses the observed split transport delivery; it cannot identify
arbitrarily delayed unframed secret bytes as distinct from fresh public typing.
Bracketed-paste ownership has separate regression coverage. Final integration and
gate results from a subsequent Linux run are recorded below.

Independent security review confirmed this scope: a deliberately unframed suffix
sent 200 ms after cancellation can appear in the public composer, while the same
delayed suffix inside an unfinished bracketed paste remains quarantined. A quiet
tick is not a transport boundary; extending a timeout would not create one. This
change does not claim to protect arbitrary secret text typed after cancellation.
The post-fix presentation run passed 78 tests, including all 34 PTY tests; an
existing unused-variable warning remains in the PTY fixture.

Existing executable CLI/configuration/session-ownership checks passed (13 tests).
The optional `legacy_lock_interop` target was also invoked but its one test was
ignored because no `CORTEX_AGENT_LEGACY_BIN` was supplied. It is not evidence of
interoperability with a pre-rename executable.

### Final integration and gate results

The focused follow-up on October 16, 2026 used Amazon Linux x86_64 and
Rust/Cargo 1.99.0. The focused targets listed above produced these results:

| Package | Test target | Passed | Failed | Ignored |
| --- | --- | ---: | ---: | ---: |
| `huginn-runtime` | `providers_integration` | 8 | 0 | 0 |
| `huginn` | `providers_flow` | 6 | 0 | 0 |
| `huginn` | `providers_recovery` | 7 | 0 | 0 |
| `huginn` | `tui` | 7 | 0 | 0 |

All four targets ran without filters: 28 tests passed, none failed or were
ignored. The recovery target also confirms the CLI's distinct stderr markers:
`No model configured` after losing an ephemeral credential, and
`model is required` for an incomplete explicit connection.

`cargo fmt --all -- --check` and the Huginn architecture check (including its
negative control) passed. The first architecture invocation failed because Cargo
did not resolve the toolchain; rerunning with `RUSTUP_TOOLCHAIN=1.99.0` passed.
Markdown lint also passed across all 52 documentation files.

`just ci-local` was **not run**: this follow-up validates the focused recovery
test and documentation corrections. Workspace-wide Clippy, Cargo check/tests,
frontend tests, rustdoc, Cargo audit and Docker E2E were not run. The other
baseline package suites, separate presentation `test-driver` PTY target, native
OS secret backend contract and legacy-binary interoperability check were not
rerun here; their earlier results and limitations remain above. These focused
results do not establish a full CI pass.
