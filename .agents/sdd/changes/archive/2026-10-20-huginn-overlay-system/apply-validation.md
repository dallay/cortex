# Evidencia de Apply: aceptación y gates

## Corrección V-01: SET-05 (2026-10-20)

Este bloque sustituye el resultado final histórico de abajo para el candidato
actual. Verify encontró un fallo contractual real que esas suites no cubrían.
Se conserva `verify-report.md` con FAIL y se requiere reverify independiente;
esta corrección no acredita QA ni aceptación del usuario.

**Cierre V-01 de Apply:** regresión RED→GREEN, suite focal completa final PASS
y `just ci-local` final exit 0, 9/9, 477 s. Pendiente `sdd-verify` independiente.

Logs nuevos en `F/`:
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-fix-v01/`.
Comandos desde `/Users/acosta/Dev/dallay/cortex`:

- `red.log` / `green.log`:
  `cargo test -p huginn-presentation --test state settings_horizontal_arrows_cycle_toasts_without_leaving_the_row -- --exact`.
  RED exit 101, aserción conductual `SET-05: Right`, observado nivel 0 frente a 1;
  compilación correcta. GREEN exit 0, 1 PASS sin rebajar expectativas. El test
  navega `/settings` y `overlay_input` real, sin mutar directamente la copia.
- `settings.log` / `settings-refactor.log`:
  `cargo test -p huginn-presentation --test state settings_ -- --test-threads=1`.
  Exit 0, 10 PASS en ambas ejecuciones. Incluye tres tests nuevos y los existentes
  de receipts tardíos, serialización, errores/reintento, Approval, shutdown e
  input obsoleto. El segundo log corresponde a liberar guardias explícitamente.
- `focused-tests.log`:
  `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn-plugin-api -p huginn-reference-extension -p huginn --all-features -- --test-threads=1`.
  Exit 0, 391 PASS reportados en 57 bloques (incluidos subprocesos), 2 ignored,
  0 failed; presentation 158 PASS, state 35 y PTY 39. Ejecutado antes del ajuste
  de lifetime de guardias en tests; CI final vuelve a probar el código final.
- `ci-local.log`: `just ci-local`, exit 1 en 3/9, Clippy:
  `significant_drop_tightening` en tres guardias de los tests nuevos. No fue fallo
  de producto; se conserva este gate fallido y no se oculta con allow ni cambios
  de runner. Se añade `drop(state)` al terminar cada grupo de aserciones.
- `clippy.log`:
  `cargo clippy -p huginn-presentation --all-features --all-targets -- -D warnings`,
  exit 0 después del ajuste; sin rebajar assertions ni lint.
- `focused-final.log`: repetición adicional del comando de siete paquetes tras
  el ajuste de guardias, simultánea a CI; exit 101 en PTY, 38 PASS/1 FAIL.
  `stale_and_paste_confirmation_cannot_approve_then_fresh_code_can` falla en
  `approval_code`, `approval challenge contains eight characters`. El helper
  espera el prefijo "Type " antes de extraer ocho caracteres del transcript;
  la salida de ese fallo no acredita el challenge completo ni una decisión
  insegura del producto. No se atribuye causa definitiva a concurrencia/entorno
  ni se modifica el test de Approval fuera del alcance V-01.
- `approval-recheck.log`:
  `cargo test -p huginn-presentation --all-features --test pty stale_and_paste_confirmation_cannot_approve_then_fresh_code_can -- --exact`,
  exit 0, 1 PASS, sin cambios de código ni aserciones. Se conserva el fallo
  adicional como riesgo de observación PTY, no se presenta como corrida verde.
- `focused-recheck.log`: repetición completa del mismo comando de siete paquetes
  sobre código final, sin cambios tras el fallo PTY; exit 0. Incluye state 35/35,
  PTY 39/39 y todos los targets restantes; no se filtra ni ignora el caso fallido.
- `audit.log`: `cargo audit --no-fetch`, exit 0 sin máscara, 1314 advisories y
  538 dependencias; no hallazgos en salida.
- `static.log`: `cargo fmt --all -- --check`, `git diff --check`, lint de ambos
  artifacts y hashes de integridad. Primera repetición posterior de lint falló
  MD038 por representar el espacio final del prefijo del challenge como code
  span; se usan comillas sin cambiar el literal. No cambia código ni tests.
- `ci-local-final.log` / `ci-local-final.exit` / `ci-summary.txt`: segunda
  `just ci-local`, exit 0, 9/9, 477 s sobre el candidato final. Rust workspace
  all-features PASS, incluidos state 35, PTY 39 y el caso de challenge que falló
  en la corrida adicional; Vitest 24 archivos/210 tests y E2E 90/90 en
  Chromium/Firefox/WebKit (2.7 min). `dashboard-dev.log` conserva la salida del
  servidor; SIGTERM corresponde al cleanup después del éxito E2E.
  Requisitos Docker/browsers/imagen ya acreditados por la misión; no se ejecuta
  reset de datos ni se sustituye por `just ci-ci`. No se cambia paralelismo,
  runner, assertions ni timeouts. `cargo audit --no-fetch` directo también pasa
  sin la máscara `|| true` del gate audit. Código sin cambios tras este gate;
  solo se completa la documentación de Apply.
- `settings-cycle-build.log` / `settings-cycle.log`: repetición del mismo fuente
  externo de Verify, sin modificarlo. Compilación exit 0 y ejecución exit 0,
  1 PASS: `active_tab=3, level=1`. Es comprobación adicional de Apply, no un
  nuevo veredicto independiente. Comandos exactos:

```sh
rustc --edition=2021 --test /private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-verify/settings-cycle.rs -L dependency=target/debug/deps -o /private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-fix-v01/settings-cycle --extern huginn_presentation=target/debug/deps/libhuginn_presentation-f028ce01cd85534f.rlib --extern crossterm=target/debug/deps/libcrossterm-19462bd5fb984d15.rlib --extern huginn_core=target/debug/deps/libhuginn_core-b0fd28370cd72cd2.rlib
/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-fix-v01/settings-cycle --nocapture
```

### Contrato e impactos revisados

- **SET-05 — GREEN propio, pendiente reverify**: ambas flechas consumidas en
  pickers toasts, level info→warn→error→info y duration 3→5→10→off→3, labels
  exactos y tab/selección estables. Left también avanza, opción explícitamente
  permitida por REQ-05. Fix limitado al match de Settings; sin toast layer.
- **SET-03/04/06 — caracterización PASS**:
  `settings_section_keys_preserve_noncycle_values_and_escape_toasts` preserva
  flechas de sección en theme/indicators/sound/integrations, wrap y Tab/BackTab
  desde toasts; no edita booleanos ni temas mediante flechas horizontales.
- **SET-07/08/13 — PASS**:
  `settings_cycle_arrows_only_persist_displayed_values_on_enter` aplica warn/5s
  con Enter una vez y los recupera al reabrir; antes no escribe, Esc descarta y
  no cancela turno activo ni contamina conversación/editor. Suite existente
  conserva receipts/Approval, errores, Apply pendiente y serialización.
- **OVL-03/EVT-01 — PASS focal heredado del código actual**: suite completa
  reejecuta Approval, stale input, PTY streaming y providers_ui; no se modifican
  tests de seguridad ni gates. No equivale a proveedor o Keychain reales.

Entrega conserva `size-exception` aprobada sobre `main`; solo siete líneas
productivas nuevas y tres tests host, más estos dos artifacts. No spec rebajado,
ni edición de verify-report/state, ni QA/Archive, ni operaciones de publicación.
La evidencia y limitaciones históricas siguientes se conservan, no se borran.

## Histórico anterior a Verify V-01

Fecha: 2026-10-20. Alcance autorizado: 6.2–6.3, entrega `size-exception`.
Este documento no es `verify-report.md` ni `qa-report.md`. No acredita aceptación
del usuario. Se conservan las tareas completas y no se modifica `state.yaml`.

**Resultado final:** 6.2–6.3 completas, Apply 21/21. `just ci-local` final PASS,
9/9 etapas, exit 0, 431 s. No quedan fallos de gate sin superseder por evidencia
posterior; los fallos históricos y riesgos de infraestructura se conservan abajo.

## Logs y comandos

Todos los logs propios están en el directorio aprobado:
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-apply6/`.
Los nombres siguientes son relativos a ese directorio.

- `prerequisites.log`: `just --list`, `just doctor`, `docker info`, inspección de
  imagen y `pnpm --filter @dallay/rook-dashboard exec playwright install --list`.
  Docker operativo; Chromium/Firefox/WebKit instalados y dependencias disponibles.
- `e2e-build.log`: `just test-e2e-build`, exit 0. Imagen reconstruida; sin reset.
- `scrollback-red.log` / `scrollback-green.log`:
  `cargo test -p huginn-presentation --features test-driver --test pty support::screen_line_feed_at_bottom_scrolls_instead_of_overwriting -- --exact`.
  RED exit 101, aserción `FIRST != SECOND`; GREEN exit 0, 1 PASS.
- `stream-scrollback-red.log` / `stream-scrollback-green.log`:
  `cargo test -p huginn-presentation --features test-driver --test pty overlays_during_streaming_never_commit_ui_or_secrets_to_scrollback -- --exact`.
  RED exit 101, `missing "┌Help" on screen`; GREEN exit 0, 1 PASS.
- `repeated-ui.log`:
  `cargo test -p huginn-presentation --test providers_ui repeated_ui_commands_after_dedup_expiry_preserve_stream_and_conversation -- --exact`.
  Exit 0, 1 PASS. Reloj fake, avance de 2001 ms; sin espera temporal real.
- `presentation-final.log`:
  `cargo test -p huginn-presentation --features test-driver -- --test-threads=1`.
  Exit 0, 155 PASS: lib 38, completion 9, extensions 2, files 2, parser 10,
  plugins 2, providers_ui 8, PTY 39, state 32, workflows 13. Fixture y doctests
  con cero casos no se contabilizan como PTY ni como evidencia adicional.
- `app-acceptance.log`:
  `cargo test -p huginn --test tui --test providers_flow -- --test-threads=1`.
  Exit 0: tui 8 (4 escenarios app y 4 del observador), providers_flow 6.
- `static-final.log`: `cargo fmt --all -- --check`, `git diff --check`,
  `cargo check -p huginn-application --examples` y
  `python3 .github/scripts/check-huginn-architecture.py`; salida sin fallos,
  arquitectura con control negativo PASS. La revisión final repetirá fmt/diff.
- `huginn-all.log`: primera consolidación de los siete paquetes, exit 0;
  anterior a los tests y fix de esta continuación. No es el gate final.
- `huginn-all-final.log`: consolidación de los siete paquetes, exit 0, con
  `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn-plugin-api -p huginn-reference-extension -p huginn --all-features -- --test-threads=1`.
- `ci-local.log`: primera `just ci-local`, exit 0, nueve etapas, 479 s;
  Vitest 24 archivos/210 tests y E2E 90 tests en tres browsers. Anterior al fix;
  no se usa para afirmar gate final del candidato modificado.
- `ci-local-final.log`: segunda `just ci-local`, exit 1 en 5/9. El test
  `actual_terminal_resize_preserves_bar_and_restores_overlay_surface` comprobaba
  la nueva selección al aparecer el título, antes del consumo asíncrono del watch.
  Pantalla de fallo: formulario nuevo y barra aún en `fixture/alpha`.
  `resize-watch-green.log`: repetición focal exacta exit 0, 1 PASS tras esperar
  `Idle|fixture/Bar-Provider` con el mismo timeout, manteniendo la aserción exacta.
  No se atribuye a fallo productivo de la publicación; se corrige la barrera del test.
- `ci-local-verified.log`: tercera `just ci-local`, exit 0 con esa sincronización;
  nueve etapas PASS, 431 s. Tests Rust workspace/all-features PASS, incluidos
  39 PTY sin filtro ni serialización añadida, providers_ui 8, state 32 y tui 8.
  Vitest 24 archivos/210 tests y E2E 90/90 en Chromium/Firefox/WebKit (3.2 min).
  `ci-summary.txt` conserva el resumen leído; `dashboard-dev.log` copia la salida
  del servidor de desarrollo E2E al temp aprobado. El cierre por SIGTERM de ese
  servidor corresponde al cleanup del runner después de pasar E2E, no a un fallo.
  No se ejecutó `just ci-ci`: omite E2E y no sustituye este gate.
- `audit-unmasked.log`: `cargo audit --no-fetch`, exit 0 sin `|| true`; 1314
  advisories cargadas y 538 dependencias comprobadas, sin hallazgos en salida.
- `providers-headless.log`: `cargo run -p huginn-application --example providers_headless`,
  exit 0. Ejecuta el ejemplo offline: turno fijado, siguiente selección Unmanaged,
  snapshot seguro y revocación sin fallback ni promesa de salud remota.
- `huginn-clippy-allfeatures.log`:
  `cargo clippy -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn-plugin-api -p huginn-reference-extension -p huginn --all-features --all-targets -- -D warnings`,
  exit 0. Incluye fixture y PTY `test-driver`, no solo las features por defecto
  que usa el paso Clippy del runner CI.
- `artifact-lint.log` / `artifact-lint-green.log`: lint adicional de los tres
  artifacts editados; primer exit 1 por un espacio terminal en code span histórico,
  corregido representando el literal con comillas sin alterar el dato; repetición
  `pnpm exec markdownlint-cli2 .agents/sdd/changes/huginn-overlay-system/{apply-validation,apply-progress,tasks}.md`
  exit 0, tres archivos sin incidencias.
- `doctor.log`: `just doctor`, exit 1: la receta no existe en el justfile actual.
  Divergencia del manual, no tooling ausente: las herramientas y dependencias se
  comprueban por comandos reales y gates ejecutados. No se inventó la receta.
- `pty-concurrent-green.log`: corrida adicional PTY con paralelismo por defecto,
  38 PASS/1 FAIL, exit 101 al crear una PTY (`failed to openpty`, OS -6).
  Coincidió con la CI completa y no llegó a ejecutar el escenario fallido;
  no es GREEN ni evidencia de fuga de secretos. No se rebajan aserciones.
  `pty-resource-diagnostic.log` registra límite PTY 511 y snapshot de procesos;
  no permite atribuir por sí solo una causa definitiva al error transitorio.

## REQ → pruebas y evidencia propia

Estado `COMPLIANT` significa cobertura técnica observada en Apply, no veredicto
de Verify/QA. Los nombres remiten a los targets de los logs anteriores; la
consolidación final incluirá además los targets core/application/runtime/plugin.

- **EVT-01 / OVL-01 — COMPLIANT**: nueva PTY
  `overlays_during_streaming_never_commit_ui_or_secrets_to_scrollback` abre y
  cierra Help, Settings, Provider, Login, ModelPicker y Confirm con un stream
  retenido hasta cancelación explícita. Compara scrollback completo antes y
  después, con control positivo de 45 líneas de conversación. Comprueba máscara,
  pantalla y transcript sin centinela. La integración
  `repeated_ui_commands_after_dedup_expiry_preserve_stream_and_conversation`
  recorre los comandos reales, incluido Logout, dos veces con reloj fake a
  0/2001 ms; conserva contenido/cantidad de conversación, tail y turno.
  `explicit_ui_categories_do_not_depend_on_message_text` cubre categorías.
- **EVT-02 — COMPLIANT**: state
  `session_reprojection_keeps_all_messages_beyond_ui_buffer_capacity`,
  `resume_and_synchronize_commit_messages_once_not_mutable_tail` y
  `tool_error_uses_contributed_label_and_marks_status`; runtime
  `sessions_persistence`/`stream_persistence` mantienen persistencia y recuperación.
- **EVT-03/04 — COMPLIANT**: lib `event::tests`, state
  `error_event_producer_uses_injected_clock_and_deduplicates_by_operation`,
  `explicit_event_operation_scope_survives_interleaved_producers` y
  `oversized_errors_are_visible_without_retaining_or_truncating_them`.
  Fronteras 1999/2000 ms, colisión, límites y finish no se prueban con sleeps.
- **OVL-02 — COMPLIANT**: plugin-api `contract`/`workflows`, reference-extension
  `smoke`; presentation `result_rejection_preserves_current_payload_focus_and_conversation`
  y `declarative_help_opens_real_host_overlay_without_notification`.
- **OVL-03 — COMPLIANT**: state serializa solicitudes, invalida leases/canales,
  cancela waiters y shutdown; workflows
  `approval_preempts_secret_workflow_and_rejects_all_ordinary_requests`.
  PTY `stale_and_paste_confirmation_cannot_approve_then_fresh_code_can` y
  `approval_resize_below_minimum_disarms_until_a_fresh_draw` verifican input fresco
  y rechazo de código válido por debajo del mínimo antes del redraw viable.
- **OVL-04 — COMPLIANT**: PTY de paste partido, Ctrl+C, unload/refresh y revocación
  de selección; workflows `revoked_origin_and_reregistered_name_cannot_mutate_replacement`,
  `late_result_after_cancel_does_not_replace_a_new_form` y
  `submission_moves_bounded_utf8_secrets_and_freezes_captured_input_once`.
  Providers_flow `assert_surfaces` inspecciona sesión JSON, metadata y archivos
  temporales, incluidos WAL/SHM; PTY inspecciona pantalla, transcript y scrollback.
- **OVL-05 — COMPLIANT**: providers_ui y app providers_flow ejercitan login,
  selección, cambio de modelo, Confirm/Logout, errores, almacenamiento Secure
  fallido sin fallback, Ephemeral explícito y OAuth no soportado. Turnos y
  revocación se cubren además en application/runtime. Son servicios controlados
  y HTTP local, no credenciales ni proveedores externos de producción.
- **BAR-01 — COMPLIANT**: PTY
  `overlays_keep_status_visible_at_minimum_inline_size` (100x30/20x8),
  `actual_terminal_resize_preserves_bar_and_restores_overlay_surface`, Approval
  estrecho y lib zero-area/clipping Unicode. Nueva PTY prueba scrollback separado.
- **BAR-02/03 — COMPLIANT**: state
  `connection_attachment_rejects_stale_sources_revisions_and_retired_close` y
  `watch_attachment_initial_read_coalesces_concurrent_updates_without_gap`;
  application `connections`/`providers_contracts`/`providers_turns` y runtime
  `providers_integration` cubren stage/confirm, faults, teardown y reconexión.
  Lib `status_bar::tests` distingue selección local de Connected, ausencia,
  actividad, feedback y diagnóstico. No se afirma disponibilidad remota.
- **SET-07/08/12 — COMPLIANT**: state
  `settings_apply_reenters_ui_once_and_late_receipt_cannot_alter_approval`,
  `settings_failed_apply_preserves_copy_and_error_and_can_retry`,
  `late_settings_receipt_preserves_new_working_copy_and_focus`,
  `replaced_settings_writes_remain_serialized_in_submission_order`,
  `settings_started_write_finishes_after_shutdown_without_reopening_or_notifying`
  y `stale_settings_input_cannot_mutate_replacement_or_cancel_turn`.
  Usan barreras/receipts reales, sin prometer rollback de escrituras iniciadas.
- **SET-01–06/13/14/16 — COMPLIANT**: lib settings, completion/plugins, state
  `settings_catalog_and_shell_navigation_never_persist_and_add_provider_only_notifies`,
  workflows de reemplazo y PTY de streaming. No se añade tema live ni onboarding.
- **SET-09/10/11/15 — COMPLIANT**: bin huginn, config_compat, cli y tui;
  preservan bytes externos/unknown, defaults y modo línea sin Settings TUI.

## Causa raíz y alcance de los cambios

El observador PTY no desplazaba filas al recibir LF en el borde inferior. La
regresión de harness falló primero; ahora retiene filas expulsadas y tiene control
positivo. Es un modelo del protocolo `append_lines` del backend inline actual,
no un emulador completo de terminal. Resize conserva el scrollback existente.

El driver bloqueaba Enter y ejecución de completion cuando `busy` era verdadero,
incluidos comandos UI. La nueva PTY falló antes del fix. Ahora los comandos UI
se ejecutan durante streaming; enviar un prompt conversacional sigue requiriendo
que no haya turno activo. No se cambian autoridad, payloads ni Approval.

Se sustituyeron wildcards de consumo por variantes explícitas en Output (Event),
las dos validaciones de OverlayKind y el wrapper PublicationFault (ProviderAction).
La proyección UiState y el router ya tenían matches exhaustivos. Ejemplos y
fixtures conservan semántica; no se toca Rook ni los runners.

## Límites, carga y handoff

- `UNTESTED`: proveedor externo y almacén seguro del SO reales; las pruebas usan
  dobles, HTTP local y metadata/SQLite temporales. No se presenta esto como fallo
  de entorno ni se inventan llamadas remotas satisfactorias.
- `UNTESTED`: prueba manual del usuario y Verify/QA independientes; se delegan
  después con sus propios informes. No se ejecuta Archive.
- Dos pruebas opt-in permanecen ignoradas por sus runners: interoperabilidad con
  binario pre-rename (`CORTEX_AGENT_LEGACY_BIN`) y almacén de secretos del SO (Keychain/Secret Service). No se contabilizan aprobadas ni se escriben entradas
  de credenciales externas para activarlas.
- La receta audit incorpora `|| true`; su exit no acredita por sí solo seguridad.
  Se conserva esta limitación del gate real; ejecución directa sin máscara exit 0.
- Estrategia `size-exception` aprobada, base/HEAD `main` sin ramas nuevas. El diff
  real excede la estimación de 2500–5500 líneas: primera medición 8769 líneas
  tracked al entrar en esta continuación, más `status_bar.rs` no tracked: el exceso
  ya era previo a 6.2–6.3. Se informará medición final separando
  artifacts y lockfile, sin ocultarlos ni crear commits/PRs.
- Index preservado: SHA-256
  `180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`.
  `state.yaml` sigue bajo control exclusivo del orquestador.

Medición final frente a HEAD `442e34b`, incluidos no tracked: 55 archivos, 8894 adiciones + 667 eliminaciones = 9561 líneas; artifacts 2343, Cargo.lock 1, código/tests/docs 7217. Detalle reproducible: `review-size-final.txt`. Excepción de tamaño aprobada; no PR ni commit creado.

## Gate y siguiente responsable

- `COMPLIANT`: runners de las unidades y gate real `just ci-local` completo.
- `FAILING` histórico, resuelto: dos RED conductuales y barrera de test del watch;
  las mismas expectativas pasan después de los ajustes documentados.
- Fallo adicional `openpty` OS -6: escenario no ejecutado en esa corrida; pasó en
  CI final. Causa definitiva de asignación no establecida; riesgo de entorno
  conservado, no se presenta como regresión de producto ni se modifica el runner.
- `UNTESTED`: los casos externos/opt-in y aceptación independiente descritos arriba.

No quedan tareas Apply pendientes. El orquestador actualiza state y delega
`sdd-verify`; después `sdd-qa` produce su informe independiente. Sin publicación
ni Archive. Código final cubierto por CI; los cambios posteriores son únicamente
el cierre documental de estos tres artifacts.
