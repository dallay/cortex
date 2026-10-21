# Informe QA — huginn-overlay-system

## 1. Identidad y resultado

- Cambio: `huginn-overlay-system`. Modo: **openspec**. Fase: **qa**.
- Fecha: **2026-10-20**. Ejecutor: subagente QA dedicado.
- Veredicto: **PASS WITH WARNINGS**, limitado a los escenarios ejecutados.
- Gate QA para Archive: **AUTORIZADO CON ADVERTENCIAS**; no se ejecuta Archive.
- Ejecución: **fallback**. No se encontró runner/FSM QA configurado en el
  repositorio; no hay enforcement determinista ni envelope de ese runner.
  Los envelopes de `agent-tty` se conservan sin reinterpretar sus errores.

## 2. Fuentes y handoff técnico

Se leyeron `proposal.md`, `design.md`, `tasks.md`, `apply-validation.md`,
`verify-report.md`, `state.yaml`, los cuatro delta specs y la base
`.agents/sdd/specs/ui-settings/spec.md`. Se consultaron `.agents/AGENTS.md`,
`.agents/sdd/config.yaml` y los skills `agent-tty` y `dogfood-tui`.

Verify entrega **PASS WITH WARNINGS**, V-01/V-02 resueltos. Su reverify de
158 casos y la CI de Apply 9/9, 477 s, Vitest 210/E2E 90 son antecedentes,
**no pruebas ejecutadas por QA**. Este informe no los cuenta como aceptación.

La configuración local no añade una política QA/Archive más restrictiva;
`testing.strict_tdd: false` no exime aceptación. Se aplica el contrato de QA:
fallos, CRITICAL/P0/P1 abiertos y BLOCKED/NOT TESTED relevantes para aceptación
bloquean Archive. No se necesita ni invoca excepción para documentación/config:
este cambio contiene producto y se ha ejecutado su target real.

## 3. Target, entorno y límites

- Repositorio: `/Users/acosta/Dev/dallay/cortex`, macOS/Unix.
- Target: Huginn TUI inline, host/presentation real con `tui-fixture` compilado
  por `test-driver`, binario Huginn y servicios controlados de conexión/runtime.
- Capacidades reales: Cargo, PTY, termios, HTTP loopback, SQLite temporal,
  reloj/barreras controlados, renderer Ghostty y Chromium de `agent-tty`.
- No se conectó a proveedor externo ni al almacén seguro real del SO.
  Solo credenciales sintéticas; sin reset de datos ni acceso a secretos reales.
- El fixture prueba UI real con motor local, no inferencia de producción.
  Los servicios controlados ejecutan operaciones; no son placeholders visuales.
- El observador PTY del repositorio modela el protocolo inline; no es un emulador
  completo. La sesión exploratoria añade un renderer independiente como contraste.
- No se modifica código de producción/tests, `state.yaml` ni staging.

Toda evidencia propia está bajo **Q/**:
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-qa/`.
`reproduce.md` contiene comandos e interacciones; `create.json` identifica la
sesión `01M5CCGBF89CVHTSHK6S4MG08T`. Directorio de trabajo y home del instrumento
aislados en Q. `destroy.json` acredita el cierre de la sesión.

## 4. Inventario de capacidades

| Capacidad | Disponibilidad / selección | Motivo |
|---|---|---|
| PTY y TUI inline | available / selected | Entrada, foco, scrollback, resize y restauración observables |
| Host, barreras y reloj inyectado | available / selected | Carreras de receipts/watch y fronteras sin sleeps |
| Servicios/API HTTP local y SQLite | available / selected | Login/model/logout, errores, revocación y recuperación reales controlados |
| Snapshots y PNG | available / selected | `doctor.json` acredita renderer; capturas propias |
| Asciicast de salida | available / selected | Grabación válida y revisable de la exploración |
| WebM | available anunciado / selected, ejecución fallida | `recording.json`: `ok:false`, `EXPORT_ERROR`; no se acredita vídeo |
| Navegador web como producto | available / rejected | Target TUI; Chromium solo renderiza evidencia terminal |
| Teclado/accesibilidad básica | available / selected | Tabs, flechas, Escape, máscara y foco |
| Lector de pantalla/tecnologías asistivas | unavailable | Sin sesión AT; no se declara conformidad de accesibilidad |
| Terminal estrecha/Unicode | available / selected | 20x8, 1x1, 0x0 y clipping por celdas |
| Locale completo | rejected | Sin contrato nuevo de traducciones; Unicode no acredita todos los locales |
| Proveedor remoto/secure store SO | unavailable / rejected para ejecución | Sin credenciales ni permiso para escribir secretos externos |
| Inspección estática | available / selected como contexto | Nunca usada por sí sola para asignar PASS |

## 5. Ejecución propia

Comandos desde la raíz; logs incluyen comandos exactos y códigos de salida:

```sh
cargo test -p huginn-presentation --all-features -- --test-threads=1
cargo test -p huginn-application --test providers_contracts --test connections --test providers_turns -- --test-threads=1
cargo test -p huginn --test tui --test providers_flow --test config_compat -- --test-threads=1
cargo test -p huginn-runtime --test sessions_persistence --test stream_persistence --test providers_integration -- --test-threads=1
cargo test -p huginn --bin huginn --test cli -- --test-threads=1
```

- `Q/presentation.log`: exit 0, **158 PASS**, incluidos 39 PTY, 35 state,
  13 workflows, 10 parser y 8 providers_ui.
- `Q/services.log`: exits 0; application **50**, app **19**, runtime **14** PASS.
- `Q/settings-persistence.log`: exit 0; bin **15**, CLI **7** PASS.
- Total de casos reportados por esos runners: **263 PASS, 0 FAIL, 0 ignored**.
  No se cuentan targets vacíos como evidencia. Los dos opt-in no se seleccionaron.
- `Q/independent-observations.log`: **10 PASS** de comprobaciones nuevas sobre
  snapshots/grabación de la sesión exploratoria, script `check-evidence.py`.
  No se suman a los 263 casos Cargo ni representan cobertura exhaustiva.

### Matriz de escenarios de aceptación

`P` = `Q/presentation.log`, `S` = `Q/services.log`, `C` =
`Q/settings-persistence.log`. Los nombres de tests identifican evidencia ejecutada
de nuevo por QA, no resultados copiados de Verify.

| Escenario / requisito | Resultado | Evidencia propia / observación |
|---|---|---|
| Happy path Help y retorno de foco, OVL-01/02-A | PASS | P: `declarative_help_opens_real_host_overlay_without_notification`; sesión `stream-help.json` abre Help mientras Busy |
| Cada superficie conserva payload/callback/continuación | PASS | P workflows: `every_typed_surface_keeps_payload_secret_callback_and_continuation_operation` |
| Rechazo de solicitud plugin, Confirm sin autoridad y Approval host-only | PASS | P: `result_rejection_preserves_current_payload_focus_and_conversation`, `approval_preempts_secret_workflow_and_rejects_all_ordinary_requests` |
| Approval preempta Settings y no lo restaura, SET-12/OVL-03 | PASS | P state: `settings_apply_reenters_ui_once_and_late_receipt_cannot_alter_approval`; stale lease/serialización también ejecutados |
| Cierre, cancelación, sender perdido, waiter drop y shutdown deniegan | PASS | P state: `approval_sender_loss_is_denial_and_late_input_has_no_authority`, `dropping_approval_waiter_retires_authority_and_sender`, `approval_cancellation_wins_ready_answer_and_shutdown_wakes_serial_waiters` |
| Paste/stale challenge no aprueba; input fresco sí | PASS | P PTY `stale_and_paste_confirmation_cannot_approve_then_fresh_code_can`; exploración `approval-after-paste.json`: DENIED; `approval-approved-screen.json`: APPROVED solo con nuevo challenge tipeado |
| Paste fragmentado, Ctrl+C y cambio de lease, OVL-04-A | PASS | P PTY `ctrl_c_cancels_incomplete_secret_paste_and_discards_its_remaining_bytes`, `closing_secret_form_discards_fragmented_paste_and_accepts_fresh_input`; parser 10/10 |
| Centinelas fuera de terminal, conversación, scrollback, JSON y archivos | PASS | P PTY streaming/secret; S `ui_two_connections_select_switch_pin_revoke_and_persist_without_secret_leaks`; snapshots, PNG y asciicast propios sin centinela de exploración |
| Revocar/re-registrar mismo nombre y resultado tardío | PASS | P workflows `revoked_origin_and_reregistered_name_cannot_mutate_replacement`, `late_result_after_cancel_does_not_replace_a_new_form` |
| UI durante streaming no entra en conversación/scrollback | PASS | P PTY `overlays_during_streaming_never_commit_ui_or_secrets_to_scrollback`, control positivo conversacional y scrollback reconstruido; sesión propia Help/Login mantiene Busy |
| Repetir comandos tras 2001 ms no crea prompts ni conversación | PASS | P providers_ui `repeated_ui_commands_after_dedup_expiry_preserve_stream_and_conversation`, reloj fake |
| Mismo texto en categorías distintas, EVT-01-B | PASS | P state `explicit_ui_categories_do_not_depend_on_message_text` |
| Mensajes legítimos, tail y reproyección conservados, EVT-02 | PASS | P state `resume_and_synchronize_commit_messages_once_not_mutable_tail`, tool error; S stream_persistence/sessions_persistence |
| FIFO 256/257, sesión recuperada >256 | PASS | P lib FIFO y state reprojection; S `reopened_store_recovers_all_events_beyond_the_ui_retention_limit` |
| Dedup 1999/2000 ms, intercalación/colisión, límites y expiración | PASS | P event tests de ventana semiabierta, igualdad completa, bytes/oversize, purga y nonce exhaustion; errores nuevos visibles |
| Settings flechas en toasts y salida con Tab/BackTab, SET-03/05 | PASS | P tres regresiones host del fix; exploración Tab×3 Right muestra `level: warn`, Down Right muestra `duration: 5s`; permanece toasts |
| Settings navegación/copia/descartar sin write ni cancelación | PASS | P shell/catalog, stale input y `settings_cycle_arrows_only_persist_displayed_values_on_enter`; exploración descarta antes de Apply |
| Apply conjunto, una escritura, error disk full y retry | PASS | P `settings_failed_apply_preserves_copy_and_error_and_can_retry`, `settings_cycle_arrows_only_persist_displayed_values_on_enter`; valores guardados recuperados al reabrir |
| Apply pendiente, Esc/reemplazo, receipt éxito/error con nuevo Settings/Approval | PASS | P `late_settings_receipt_preserves_new_working_copy_and_focus`, `replaced_settings_writes_remain_serialized_in_submission_order`, `settings_started_write_finishes_after_shutdown_without_reopening_or_notifying` |
| TOML externo/unknown, defaults y shell sin aplicación live | PASS | C `config::tests::ui_defaults_fill_absent_sections_and_partial_nested_tables`, cuatro `ui_settings::tests`; P Settings shell y config proyectada |
| Settings excluido de line-mode, catálogo y Add provider sin I/O | PASS | S tui `non_tty_default_fails_clearly_and_explicit_line_recovery_works` envía `/settings`; P catálogo/Add provider |
| Login/providers/model/logout funcional sin reiniciar, OVL-05 | PASS | P providers_ui y S providers_flow 6/6: save/select/update/revoke, Secure fallido sin fallback, cancelación/OAuth; no proveedor remoto |
| Selección siguiente turno, pin de turno actual y revocación | PASS | S providers_turns/providers_integration; HTTP local demuestra que selección no cambia el turno admitido |
| Watch inicial, A tardío/duplicado, cierre retirado y source nuevo | PASS | P `watch_attachment_initial_read_coalesces_concurrent_updates_without_gap`, `connection_attachment_rejects_stale_sources_revisions_and_retired_close`; S staged commit/teardown/faults |
| Barra siempre visible; no falsa disponibilidad Connected | PASS | P status_bar y PTY matriz de overlays; sesión muestra fixture/alpha y luego Bar-Login Ephemeral, Busy/Approval/Idle sin Connected |
| Resize normal/20x8, min/1x1 y zero-size | PASS | P PTY matriz y `approval_resize_below_minimum_disarms_until_a_fresh_draw`; lib `zero_area_draw_does_not_arm_approval_then_viable_draw_can_arm`; exploración Login 100x30→20x8→100x30 |
| Preview completo desplazable y Unicode por celdas | PASS | P Approval preview/PgUp/PgDn y clipping status_bar; no se infiere anchura Unicode solo del snapshot textual |
| Restore normal, cancelación, drop/unload y nueva generación | PASS | P PTY termios/raw/bracketed-paste/inline; exploración compara `stty -g` completo antes/después y muestra TERMINAL_RESTORED |
| Exploración teclado y foco tras Escape | PASS | `stream-login.json` envía entrada demasiado pronto y no abre formulario; tras composer observable, `stream-login-retry.json` abre Login; entrada anterior no cruza lease |
| Exportación WebM | FAIL | `recording.json`: `ok:false`, `EXPORT_ERROR`; fallo del instrumento, no escenario funcional ni concesión de permisos |
| Accesibilidad con lector de pantalla | NOT TESTED | Sin sesión AT ni contrato nuevo de conformidad; teclado probado no certifica AT |
| Internacionalización de locales completos | NOT TESTED | No hay requisito de traducción nuevo; Unicode/clipping sí se ejecutaron |
| Navegador como superficie del producto | NOT TESTED | No aplicable: target terminal, no dashboard web |
| Proveedor externo / secure store SO reales | NOT TESTED | Sin credenciales/permiso; no se intentaron llamadas ni escrituras externas |
| Dos opt-in heredados | NOT TESTED | Binario pre-rename no suministrado y backend SO no autorizado; no incluidos en runners seleccionados |

Las pruebas de host y servicios son sondas de aceptación focales. La exploración
añade flujos nuevos propios; QA no se limita a renombrar reportes Apply/Verify.
No se afirma aceptación humana del usuario ni aceptación del propio harness.

## 6. Evidencia visual, incidencias del instrumento y alcance no probado

- PNG: rutas exactas en `settings-screenshot.json`, `login-screenshot.json`,
  `small-screenshot.json` y `approval-screenshot.json` de Q. La última muestra
  el estado posterior a paste denegado, no una Approval pendiente.
- Grabación válida: `asciicast.json` referencia
  `tty/sessions/01M5CCGBF89CVHTSHK6S4MG08T/artifacts/recording-199-asciicast.cast`.
  Captura 169 eventos de salida y dos resize; no se presenta como WebM.
- `doctor.json` anunció WebM disponible; la exportación terminó `EXPORT_ERROR`.
  Se conserva el FAIL del instrumento. PNG/asciicast y PTY siguen aportando
  evidencia observable; no se transforma ese envelope en PASS.
- Primer launch: falta respuesta DSR del instrumento, error de cursor position;
  se restauró terminal y se repitió con respuesta explícita ESC[9;1R.
  No se declara compatibilidad automática con todo emulador. Las capturas
  iniciales conservan texto del shell/error previo fuera del viewport inline.
- Dos batches quedaron `interrupted` por timeout externo: arranque sin DSR y
  comando enviado durante cuarentena tras Escape. No se cuentan como PASS;
  las ejecuciones posteriores se identifican separadamente, sin borrar originales.
- El centinela solo es sintético. La telemetría `events.jsonl` de agent-tty
  contiene entradas de input; no es un log del producto ni prueba de redacción
  del instrumento. Snapshots/PNG/asciicast son evidencia de salida limpia.
- CI amplia, audit, fmt/clippy y E2E dashboard no se repiten en QA; corresponde
  al handoff técnico vigente. No se declara una nueva CI completa.
- Para extender aceptación operativa: aportar credenciales de prueba y permiso
  explícito de escritura en Keychain/Secret Service; ejecutar los opt-in y el
  flujo remoto en entorno aislado. Para lock legacy: suministrar
  `CORTEX_AGENT_LEGACY_BIN`. Para AT/locales: definir target, contrato y sesión.

## 7. Hallazgos y riesgos

| ID | Severidad | Estado | Descripción / seguimiento |
|---|---|---|---|
| QA-01 | P3 | OPEN, no bloqueante | WebM falló pese a doctor disponible; usar PNG/asciicast válidos y diagnosticar exportador antes de exigir vídeo |
| QA-02 | P3 | OPEN, no bloqueante | agent-tty no respondió automáticamente al DSR inicial; reproducción requiere respuesta explícita documentada |
| QA-03 | P2 | NOT TESTED, riesgo operativo | Proveedor externo, secure store SO y dos opt-in sin ejecución; no acredita producción ni interoperabilidad legacy |

No se identificaron defectos funcionales nuevos ni hallazgos CRITICAL/P0/P1
abiertos en las ejecuciones. V-01 histórico no se reabre: Right se observó
funcionando en la sesión y las tres regresiones host vuelven a pasar.
La causa del fallo histórico del helper PTY de challenge no se establece aquí;
el caso íntegro pasó en los 39 PTY propios, sin filtros ni cambio de assertions.

## 8. Veredicto, política de Archive y handoff

**PASS WITH WARNINGS. Gate QA: autorizado con advertencias.**

La aceptación relevante especificada se ejecutó: OVL-05-A exige explícitamente
un servicio **controlado** y OVL-05-B un fallo **controlado** del almacenamiento
seguro. Ambos se probaron con operaciones locales reales y dobles controlados.
Ningún requisito nuevo exige credenciales externas reales, lock pre-rename,
lector de pantalla o traducciones completas. Por ello los NOT TESTED anteriores
no son aceptación contractual pendiente ni se convierten en PASS externo.
La barrera normal para NOT TESTED relevante permanece intacta; no se inventa
una excepción de documentación/configuración para este cambio de producto.

El FAIL de exportación es tooling suplementario: no invalida escenarios de
producto con evidencia PTY, snapshots, PNG y asciicast válida. Sigue visible
como advertencia, no oculto ni interpretado como vídeo aprobado.

Archive debe comprobar ambos informes, el estado vigente y ausencia de nuevos
CRITICAL/P0/P1/fallos funcionales antes de actuar. Este informe autoriza el gate
QA del candidato probado, no despliegue ni disponibilidad remota. El orquestador
puede avanzar a `sdd-archive` conservando estas advertencias. Si cambia el código,
debe volver a evaluar la evidencia afectada. No se cambia `state.yaml` aquí.

Staging SHA-256 inicial y final:
`180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`.
Sin add/stage/unstage/reset/checkout/commit/push/merge/PR, sin reset de datos.
Único archivo nuevo del repositorio en esta fase: este informe QA.
