# Verification Report

## Resultado

- Cambio: `huginn-overlay-system`.
- Fecha: 2026-10-20. Persistencia: openspec, ruta explícita de esta misión.
- **Veredicto actual: PASS WITH WARNINGS** tras reverify V-01 el 2026-10-20.
  V-01 y V-02 resueltos con evidencia independiente; no quedan defectos técnicos
  bloqueantes identificados. Se conservan los límites externos y de tooling V-03.
- Veredicto inicial: **FAIL** por REQ-05; historia y evidencia RED conservadas abajo.
- Verificación técnica independiente; no acredita aceptación del usuario ni QA.
- Modo de ejecución: **fallback**. No se encontró `sdd-quality-runner.mjs` en
  el proyecto ni en los recursos SDD locales consultados. No hubo enforcement
  determinista ni envelopes versionados. `testing.strict_tdd: false`; no se
  cargó el módulo Strict TDD.
- Skill resolution: `none`; no existe `.agents/skill-registry.md`.

| Completitud | Resultado |
|---|---|
| Tareas principales declaradas completas | 21/21 |
| Subchecklist 2.2 | 3/3 |
| Artifacts leídos | proposal, cuatro delta specs, spec base ui-settings, design, tasks, apply-validation |
| Conformidad técnica | Acreditada en los escenarios ejecutados; V-01 resuelto |
| QA / aceptación del usuario | Pendiente; no inferida de Apply ni de Verify |

Se inspeccionó el código final, no solo las etiquetas COMPLIANT de Apply.
No se modificó producción, tests del repositorio, staging ni `state.yaml`.

## Reverify V-01 — evidencia actual y decisión

Se leyó la corrección en `overlay.rs:140–151`, los tres tests host nuevos de
`state.rs:742–864` y el bloque nuevo de `apply-validation.md`. No se rebajaron
specs ni se sustituyó el escenario contractual por Space. La rama nueva se
limita a Settings en toasts: Left/Right invocan `toggle_selected`, mientras que
Tab/BackTab siguen cambiando de pestaña y las demás pestañas conservan las flechas
horizontales. REQ-05 permite que Left también avance el ciclo. Esta regla concreta
de pickers tiene precedencia sobre la navegación general de REQ-03; Tab/BackTab
permiten salir de toasts. No se añadió aplicación live ni capa de toasts.

Directorio independiente de reverify (en adelante `R/`):
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-reverify-v01/`.
Todos los comandos se ejecutaron desde `/Users/acosta/Dev/dallay/cortex`.

| Comando / evidencia actual | Exit | Resultado |
|---|---|---|
| `cargo test -p huginn-presentation --all-features -- --test-threads=1` | 0 | `R/presentation.log`: 158 PASS, 0 failed, 0 ignored; lib 38, state 35, PTY 39 y restantes targets |
| Recompilación del fuente original independiente `V/settings-cycle.rs` con rustc | 0 | `R/settings-cycle-build.log`; rlibs obtenidos del fingerprint del target state recién ejecutado; comando exacto en `R/independent-command.txt` |
| `R/settings-cycle --nocapture` | 0 | `R/settings-cycle.log`: 1 PASS; `active_tab=3, level=1`; mismo fuente y aserciones que fallaron antes |
| `cargo fmt --all -- --check` y `git diff --check` | 0 | `R/static.log` |
| `cargo clippy -p huginn-presentation --all-features --all-targets -- -D warnings` | 0 | `R/static.log`; incluye tests/fixture |

La suite actual repite todos los casos de presentation de la matriz inicial,
incluidos autoridad única, Approval fail-closed/resize/0x0, secretos, scrollback,
receipts tardíos y watch. Además ejecuta estas regresiones por el router host real:

| REQ → escenario del fix | Test ejecutado | Resultado |
|---|---|---|
| SET-05: level y duration, ambas flechas, ciclos completos y labels | `settings_horizontal_arrows_cycle_toasts_without_leaving_the_row` | PASS: no cambia tab/selección ni el otro valor |
| SET-03/04/06: tabs ordinarios, wrap, salida de toasts y valores intactos | `settings_section_keys_preserve_noncycle_values_and_escape_toasts` | PASS: Tab/BackTab siguen siendo navegación; no edita valores no cíclicos |
| SET-06/07/08/13 y EVT-01: no write antes de Enter, valores visibles guardados juntos, Esc descarta sin cancelar turno ni contaminar UI | `settings_cycle_arrows_only_persist_displayed_values_on_enter` | PASS: warn/5s, una escritura y reapertura con snapshot guardado |

Se revisó el log final y resumen de Apply en `huginn-fix-v01/`:
`ci-local-final.log`, `ci-local-final.exit` y `ci-summary.txt` certifican exit 0,
9/9, 477 s, 210 Vitest y 90 E2E. `focused-recheck.log` documenta la repetición
verde de los siete paquetes. Es evidencia heredada de Apply, no CI ejecutada
por reverify. La repetición independiente se limitó al paquete modificado y sus
tests; no fue necesario repetir CI amplia ni los paquetes sin cambios productivos.

Se conserva el fallo intermedio de Apply en PTY
`stale_and_paste_confirmation_cannot_approve_then_fresh_code_can`: el helper no
obtuvo ocho caracteres de challenge. No acredita autorización indebida y no se
establece una causa definitiva. Pasó después, sin modificar assertions ni ese
código, en la repetición de siete paquetes, CI final y los 39 PTY de este reverify.
Es riesgo de observación PTY, no un fallo vigente ni motivo para omitir el caso.

**Decisión:** V-01 corregido y V-02 cubierto. Los límites externos/opt-in de V-03
no bloquean el paso a QA porque OVL-05 exige servicios controlados, no credenciales
externas reales. No equivalen a integración operativa aprobada. Se permite el
handoff a **sdd-qa**, no Archive ni aceptación del usuario. Modo fallback y ausencia
de cobertura instrumental permanecen declarados; UNAVAILABLE no se convierte en PASS.

## Ejecución y evidencia iniciales (histórico)

Directorio de evidencia independiente (en adelante `V/`):
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-verify/`.
Todos los comandos se ejecutaron desde `/Users/acosta/Dev/dallay/cortex`.

| Comando / evidencia | Exit | Resultado |
|---|---|---|
| `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn-presentation -p huginn-plugin-api -p huginn-reference-extension -p huginn --all-features -- --test-threads=1` | 0 | `V/focused-tests.log`; 388 PASS reportados en 57 bloques, incluidos resultados de subprocesos; 2 ignored, 0 failed |
| Presentation dentro del comando anterior | 0 | 155 PASS: lib 38, completion 9, extensions 2, files 2, parser 10, plugins 2, providers_ui 8, PTY 39, state 32, workflows 13 |
| `cargo fmt --all -- --check` | 0 | `V/static.log` |
| `git diff --check` | 0 | `V/static.log` |
| `cargo check -p huginn-application --examples` | 0 | `V/static.log`; comprobación adicional a los tests |
| Prueba independiente `V/settings-cycle --nocapture` | 101 | 0 PASS, 1 FAIL; `V/settings-cycle.log`, V-01 |
| Cobertura instrumental | — | UNAVAILABLE: no se ejecutó llvm-cov; no se declara porcentaje |

La prueba independiente se escribió fuera del repositorio en
`V/settings-cycle.rs`. Se compiló correctamente con el siguiente comando,
usando los mismos artefactos que `tests/state.rs` de esta ejecución:

```sh
rustc --edition=2021 --test /private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-verify/settings-cycle.rs -L dependency=target/debug/deps -o /private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-verify/settings-cycle --extern huginn_presentation=target/debug/deps/libhuginn_presentation-f028ce01cd85534f.rlib --extern crossterm=target/debug/deps/libcrossterm-19462bd5fb984d15.rlib --extern huginn_core=target/debug/deps/libhuginn_core-b0fd28370cd72cd2.rlib
```

Build exit 0 en `V/settings-cycle-build.log`. El primer intento de enlace había
seleccionado rlibs de features distintas y falló por tipos incompatibles; no fue
una regresión de producto. Se resolvió leyendo los hashes del fingerprint
`huginn-presentation-537756dcf8e42867/test-integration-test-state.json`.
El FAIL citado corresponde a una aserción conductual ejecutada, no a compilación.

Se leyó también el resumen y el log final de Apply en `huginn-apply6/`:
`ci-local-verified.log` / `ci-summary.txt`, 9/9 exit 0, 431 s, 210 Vitest,
90 E2E. Se conserva como evidencia heredada, no como CI ejecutada por Verify.
No se repitió CI amplia porque las modificaciones son de Huginn y la repetición
focal cubre sus siete paquetes. El fallo histórico openpty OS -6 no reapareció
en los 39 PTY de Verify; esto no establece su causa definitiva.

## Matriz de conformidad conductual

Los nombres siguientes corresponden a tests que pasaron en `V/focused-tests.log`.
Los casos de presentation volvieron a pasar en `R/presentation.log`; SET-05 se
actualiza con la evidencia de reverify. `lib` se refiere a huginn-presentation;
los targets se identifican con su paquete cuando no pertenecen a presentation.

| REQ → escenario | Test ejecutado / evidencia | Estado técnico |
|---|---|---|
| OVL-01-A | workflows `declarative_help_opens_real_host_overlay_without_notification`; lib `render_snapshot_and_input_share_authority_and_redact_secrets`; PTY streaming/scrollback | COMPLIANT |
| OVL-02-A | workflows `declarative_help_opens_real_host_overlay_without_notification`; plugin-api contract/workflows | COMPLIANT |
| OVL-02-B | workflows `result_rejection_preserves_current_payload_focus_and_conversation`, `approval_preempts_secret_workflow_and_rejects_all_ordinary_requests`; plugin-api validadores | COMPLIANT |
| OVL-03-A | state `settings_apply_reenters_ui_once_and_late_receipt_cannot_alter_approval`, `approval_retains_entire_diff_and_requires_fresh_armed_code`; PTY stale/fresh | COMPLIANT |
| OVL-03-B | state `approval_sender_loss_is_denial_and_late_input_has_no_authority`, `dropping_approval_waiter_retires_authority_and_sender`, `approval_cancellation_wins_ready_answer_and_shutdown_wakes_serial_waiters` | COMPLIANT |
| OVL-03-C | state `approval_requests_serialize_and_stale_lease_cannot_resolve_successor`; workflows preemption/rechazo | COMPLIANT |
| OVL-04-A | PTY `ctrl_c_cancels_incomplete_secret_paste_and_discards_its_remaining_bytes`, `closing_secret_form_discards_fragmented_paste_and_accepts_fresh_input`; app providers_flow | COMPLIANT |
| OVL-04-B | workflows `revoked_origin_and_reregistered_name_cannot_mutate_replacement`, `late_result_after_cancel_does_not_replace_a_new_form`, `reentrant_callbacks_cannot_commit_stale_tickets_and_pending_drop_is_outside_lock` | COMPLIANT |
| OVL-05-A | providers_ui 8; huginn/providers_flow 6; application/providers_turns; runtime/providers_integration | COMPLIANT con servicios controlados |
| OVL-05-B | providers_ui y providers_flow: Secure fallido, Ephemeral explícito, validación, logout/cancelación; runtime secret_store con doble | COMPLIANT con servicios controlados |
| EVT-01-A | providers_ui `repeated_ui_commands_after_dedup_expiry_preserve_stream_and_conversation`; PTY `overlays_during_streaming_never_commit_ui_or_secrets_to_scrollback` | COMPLIANT |
| EVT-01-B | state `explicit_ui_categories_do_not_depend_on_message_text` | COMPLIANT |
| EVT-02-A | state `resume_and_synchronize_commit_messages_once_not_mutable_tail`, `tool_error_uses_contributed_label_and_marks_status`; runtime stream_persistence | COMPLIANT |
| EVT-02-B | state `session_reprojection_keeps_all_messages_beyond_ui_buffer_capacity`; lib `ui_event_buffer_keeps_only_the_latest_capacity_in_fifo_order`; runtime sessions_persistence | COMPLIANT |
| EVT-03-A | lib `dedup_uses_half_open_two_second_window_and_reanchors_visible_emission`; state error producer | COMPLIANT |
| EVT-03-B | lib `dedup_interleaved_operations_do_not_suppress_each_other`; state `explicit_event_operation_scope_survives_interleaved_producers` | COMPLIANT |
| EVT-03-C | lib `dedup_scopes_operations_and_compares_full_text_on_hash_collision` | COMPLIANT |
| EVT-04-A | lib `dedup_enforces_entry_byte_and_oversize_bounds_without_truncation`, `dedup_purges_expired_entries_and_releases_retained_bytes`, `dedup_nonce_exhaustion_emits_without_panicking_or_caching` | COMPLIANT |
| BAR-01-A | PTY `overlays_keep_status_visible_at_minimum_inline_size`, `actual_terminal_resize_preserves_bar_and_restores_overlay_surface`, `approval_resize_below_minimum_disarms_until_a_fresh_draw`; lib zero-area y status_bar | COMPLIANT |
| BAR-02-A | state `watch_attachment_initial_read_coalesces_concurrent_updates_without_gap`; application/providers_contracts `staged_selection_requires_exact_commit_and_snapshot_is_safe` | COMPLIANT |
| BAR-02-B | state `connection_attachment_rejects_stale_sources_revisions_and_retired_close` | COMPLIANT |
| BAR-02-C | application/connections fault injection/publication/reopen; providers_contracts `selection_watch_tracks_exact_teardown_and_activation_source`, `adapter_self_cancellation_publishes_absence_before_unload`; core lifecycle | COMPLIANT |
| BAR-03-A | lib status_bar tests; application/providers_contracts staged snapshot; PTY barra | COMPLIANT |
| BAR-03-B | providers_contracts failure/revocation; application/connections publication faults; lib status_bar feedback/diagnostic | COMPLIANT: no se emite disponibilidad remota, ni siquiera tras éxito |
| SET-07-A/B/C | state `settings_failed_apply_preserves_copy_and_error_and_can_retry`, `settings_apply_reenters_ui_once_and_late_receipt_cannot_alter_approval`; lib projected config | COMPLIANT |
| SET-07-D | state `late_settings_receipt_preserves_new_working_copy_and_focus` (éxito/error) | COMPLIANT |
| SET-08-A/B | state receipt/preemption/streaming; PTY streaming; `settings_started_write_finishes_after_shutdown_without_reopening_or_notifying` | COMPLIANT |
| SET-12-A/B/C/D | state stale input y receipt/Approval; workflows authorized replacement, result rejection y preemption | COMPLIANT |
| SET-01, SET-16: registro, whitespace, catálogo y no prompt | state `settings_catalog_and_shell_navigation_never_persist_and_add_provider_only_notifies`; completion/plugins; PTY streaming | COMPLIANT |
| SET-02/03/04: estructura, tabs y filas/markers | lib settings snapshot/tab/item; state shell; PTY matriz de tamaños | COMPLIANT |
| SET-05: temas/defaults/booleanos | lib settings `snapshot_items_match_locked_d6`; huginn config_compat/bin defaults | COMPLIANT |
| SET-05: toasts, `→` sobre level: info → level: warn | Prueba independiente original recompilada: active_tab=3, level=1; tres nuevos tests host en `R/presentation.log` | **COMPLIANT, V-01 resuelto** |
| SET-06: navegación sin escritura | state shell; lib settings; closures contadas | COMPLIANT |
| SET-09/10/11: bytes externos, defaults ausentes/parciales/completos, unknown | huginn bin tests 15 y config_compat 5 | COMPLIANT |
| SET-13/14: Esc no cancela turno, Add provider sin I/O | state shell/stale input; PTY streaming | COMPLIANT |
| SET-15: exclusión de line-mode | huginn tui y cli | COMPLIANT |

## Corrección e inspección de diseño

| Área | Inspección independiente | Resultado |
|---|---|---|
| Autoridad única D1/D8 | `state.rs:158–192`: solo `overlays` posee payload activo; accesores modal/workflow/settings son proyecciones; `terminal.rs` snapshot e input consultan la misma máquina | Conforme |
| Host-only D2/D9 | API plugin no incluye Approval; máquina `overlay.rs:401–427` valida host/ticket/origen; Approval conserva sender separado de PluginDialog | Conforme |
| Resize fail-closed | `terminal.rs:523–549,909–918` desarma tamaño inviable y solo arma tras paint viable; PTY envía código válido a 1x1 y después a 20x8; lib cubre 0x0 | Conforme en escenarios ejecutados |
| UI y persistencia D3/D7 | `state.rs:284–300` separa categorías; `terminal.rs:564–575` solo inserta conversation; FIFO en event.rs es no autoritativa; runtime recupera sesión >256 | Conforme |
| Selección/watch D4 | providers publica con mutex/`send_replace`, exige candidata exacta; subscribe usa mismo mutex; ModelPublication y MetadataMutation cierran fail-closed; terminal usa borrow_and_update antes del draw y attachment al actualizar/cerrar | Conforme |
| Secretos D5/D6 | Snapshot de overlay solo identidad; workflow renderer enmascara; dedup conserva comparación cifrada, límites 256/256KiB/4096 y operación; pruebas de centinela/JSON/PTY pasan | Conforme al alcance de pruebas; no certifica todas las fuentes de errores futuras |
| Settings receipts | `state.rs:1051–1143` captura una vez, serializa workers y consume en orden; éxito tardío solo actualiza saved config; error tardío no modifica overlay actual | Conforme |
| Tamaño, clipping y scroll | Reserva barra antes de ramas; clipping por celdas; Approval preview completo con PgUp/PgDn; PTY 20x8/100x30, resize y restauración pasan | Conforme |
| Navegación Settings | `overlay.rs:140–151` da prioridad a los ciclos toasts; Tab/BackTab conservan navegación y las demás pestañas conservan flechas de sección; regresiones host y test independiente pasan | Conforme tras reverify; incumplimiento histórico SET-05 resuelto |

El diseño permite kind + payload tipado en vez de imponer literalmente una
variante de payload por cada superficie; esta elección no es doble autoridad.
Los tests de reentrancia ejercitan callbacks y Drop fuera del lock. No se
observaron slots activos legacy independientes ni cambios de contratos durables.

## Hallazgos priorizados

| Finding | Judge A | Judge B | Severity | Status |
|---|---|---|---|---|
| V-01: SET-05 no cicla toasts con →, overlay.rs:140–144 original | No ejecutado | No ejecutado | CRITICAL histórico | RESOLVED: test independiente original PASS en reverify |
| V-02: cobertura original no acredita teclado del escenario SET-05 | No ejecutado | No ejecutado | WARNING histórico | RESOLVED: tres regresiones host PASS |
| V-03: sistemas externos/opt-in sin ejecución | No ejecutado | No ejecutado | WARNING | UNTESTED; no es PASS externo |
| V-04: helper PTY de challenge falló en una corrida intermedia de Apply | No ejecutado | No ejecutado | WARNING | Sin causa definitiva; caso íntegro PASS en reverify y gates finales |

No hubo dos jueces independientes; las columnas requeridas no simulan votos.

### CRITICAL histórico — V-01 resuelto

La especificación base conservada exige en REQ-05, líneas 132–139, que `→` sobre
`level: info` muestre `level: warn`. Reproducción: abrir `/settings`, pulsar Tab
tres veces para toasts y pulsar Right. El router real cambia a integrations
(`active_tab=4`) y mantiene info (`level=0`). La aserción independiente falla
con exit 101. Esto impide aceptar la preservación completa de Settings aunque
el comportamiento pudiera ser anterior a esta migración: el delta conserva
expresamente REQ-05. No se corrigió ni se cambió el spec en Verify.
La corrección posterior de Apply hace pasar exactamente este fuente externo:
exit 0, tab=3 y level=1, sin cambiar las aserciones. No queda abierto V-01.

### WARNING histórico — V-02 resuelto

`toggle_advances_toast_level_cycle` / `toggle_advances_toast_duration_cycle`
invocan `toggle_selected`; el test host navega las pestañas con Tab y no prueba
Right dentro de toasts. Así, los 155 tests de presentation pueden pasar mientras
el escenario contractual falla. Se requiere regresión por input host, además de
decidir cómo compatibilizar navegación horizontal de tabs y ciclo de pickers.
Apply añadió tres tests host que cubren teclado, ciclos, navegación y persistencia;
los tres pasan en este reverify. No queda abierto V-02.

### WARNING — V-03 y límites de aceptación

- No se usaron proveedor externo ni almacén seguro real del SO. OVL-05-A exige
  explícitamente un servicio controlado; OVL-05-B exige un fallo controlado de
  almacenamiento. Por tanto estos límites no invalidan esos escenarios técnicos,
  que sí se ejecutaron, pero tampoco acreditan integración operativa externa.
- Dos tests ignored siguen UNTESTED: lock con binario pre-rename y backend de
  secretos del SO. El primero no corresponde a un requisito nuevo de overlay;
  el segundo no sustituye la prueba de fallo Secure sin fallback, que sí pasó.
  Ningún spec de este cambio exige usar credenciales externas reales para aceptar
  esos escenarios. QA debe declarar su alcance operativo, no asumir éxito real.
- El spec base enumera un target `--test settings` que no existe. La conducta
  se distribuye entre lib/state/PTY; se verificaron esos targets reales. Esa
  distribución no excusó el escenario de teclado faltante V-01, ahora cubierto.
- Sin cobertura instrumental ni runner determinista, no se declara cobertura
  exhaustiva de ramas. La conformidad se limita a los escenarios ejecutados.
- Los casos llamados QA en specs tienen evidencia automatizada técnica; la
  aceptación del operador y `qa-report.md` siguen pendientes. No archivar.

### SUGGESTION

Conservar una prueba de regresión host para cada tecla contractual de Settings;
no sustituir el escenario Right por una prueba de Space sin cambio aprobado del
spec. Mantener las aserciones de seguridad y de PTY existentes.

## Integridad y siguiente fase

El hash final del staging coincide con el recibido:
`180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`.
No se ejecutó add/reset/unstage/checkout/commit/push/merge/PR, reset de datos,
Archive ni escritura de state.

**Handoff actual:** el orquestador puede delegar a **sdd-qa**, responsable de los
escenarios de aceptación y de `qa-report.md`. Reverify concluye **PASS WITH WARNINGS**;
el FAIL inicial queda supersedido por evidencia nueva, no borrado. No autoriza
Archive ni aceptación de producto. Staging comprobado sin cambios también al
final de la ejecución focal de reverify; state sigue bajo control del orquestador.
