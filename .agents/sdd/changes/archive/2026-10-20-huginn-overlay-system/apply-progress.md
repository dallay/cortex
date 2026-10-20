# Apply progress — huginn-overlay-system

## Corrección V-01 tras Verify fallido (2026-10-20)

Estado acumulado: se conservan las 21/21 tareas y toda su evidencia histórica.
Este bloque sustituye el cierre anterior para el candidato modificado: Verify
detectó V-01 real en SET-05; no se infiere conformidad de las suites anteriores.
Alcance autorizado: corregir V-01 y añadir regresión host, sin ampliar diseño.
Entrega `size-exception` previamente aprobada, base/HEAD `main`, sin cadena ni PR.
El worktree sucio es el candidato explícitamente recibido para esta corrección;
no se restauró ni sustituyó trabajo previo. Solo se editan `overlay.rs`,
`tests/state.rs`, este artifact y `apply-validation.md`.

- Leídos completos `verify-report.md`, spec base ui-settings, delta ui-settings,
  diseño, tareas y progreso previo. REQ-05 es la excepción específica a REQ-03:
  las flechas horizontales editan los dos pickers de toasts; en las otras pestañas
  navegan secciones. Tab/BackTab siempre navegan. Se elige la opción contractual
  de avanzar también con Left; no se cambia footer, spec ni valores de ciclo.
- RED real en repo: `settings_horizontal_arrows_cycle_toasts_without_leaving_the_row`
  vía `/settings` → input host con ticket/focus real → Tab×3 → Right, exit 101,
  nivel observado 0 frente a 1. GREEN del mismo comando exit 0. Comprueba ambos
  ciclos completos, ambas flechas, labels, selección y permanencia en tab 3.
- Fix mínimo: siete líneas en el match Settings de `overlay.rs`, antes de la
  navegación genérica. No cambia leases, receipts, Approval, streaming ni I/O.
  Refactor de pruebas libera explícitamente guardias tras las aserciones;
  no cambia expectativas ni añade supresiones de lint.
- Caracterización adicional por host: `settings_section_keys_preserve_noncycle_values_and_escape_toasts`
  conserva flechas, wrap y Tab/BackTab sin cambiar valores; async
  `settings_cycle_arrows_only_persist_displayed_values_on_enter` comprueba cero
  writes durante edición/descarte, Enter único con warn/5s, receipt y reapertura,
  sin conversación/editor ni cancelación del turno activo.
- Diez tests Settings PASS antes y después del refactor; siete paquetes Huginn
  con all-features: 391 PASS reportados en 57 bloques (incluyen subprocesos),
  2 ignored, 0 failed. Presentation 158 PASS, incluidos state 35 y PTY 39.
- Primera `just ci-local` nueva falla en 3/9, Clippy: tres guardias de los tests
  nuevos requerían liberación explícita (`significant_drop_tightening`). Se añaden
  `drop(state)` tras las mismas aserciones; Settings 10/10 y Clippy focal
  all-features/all-targets PASS. Segunda CI completa terminada: `just ci-local`
  exit 0, 9/9, 477 s (`ci-local-final.log` y `ci-summary.txt`); Rust workspace
  all-features PASS, incluidos state 35 y PTY 39 con paralelismo del runner,
  Vitest 24 archivos/210 tests y E2E 90/90 en tres browsers (2.7 min).
  No se usa la CI histórica para acreditar este candidato ni se cambia el runner.
- Corrida adicional de siete paquetes tras el refactor (`focused-final.log`),
  simultánea a CI, exit 101 en PTY (38 PASS/1 FAIL): el helper `approval_code`
  observó "Type " sin ocho caracteres disponibles. No se modifica Approval ni
  se atribuye causa definitiva a entorno/concurrencia. Repetición exacta del
  caso sin cambios exit 0, 1 PASS (`approval-recheck.log`); se conserva el riesgo.
  Repetición completa posterior sin cambios (`focused-recheck.log`) exit 0,
  incluidos state 35/35 y PTY 39/39; no se omite el caso ni se cambia su timeout.
  El mismo caso también pasa en la CI completa final. `cargo audit --no-fetch`
  directo y fmt/diff-check/lint de artifacts exit 0. Código final cubierto por
  estos gates; cambios posteriores limitados al cierre de estos dos artifacts.
- Logs y comandos exactos en `apply-validation.md`, directorio aprobado
  `opencode/huginn-fix-v01/`. Staging inicial coincide con SHA-256 recibido
  `180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`.
- Pendiente: nueva verificación independiente `sdd-verify`; no se modifica su
  FAIL anterior, `state.yaml`, QA ni Archive. No staging/unstaging/reset/checkout,
  commit/push/merge/PR ni reset de datos. Skill resolution: `none` (sin registry
  del proyecto); procedimientos comunes y reglas compactas SDD consultados.

## Reanudación Apply 6.2–6.3 (2026-10-20)

**Cierre verificado:** 6.2 y 6.3 completas; estado acumulado 21/21 tareas.
`just ci-local` final exit 0, 9/9, 431 s; código final sin cambios posteriores.
Los párrafos de comandos en curso de esta sección son históricos y quedan
sustituidos por este cierre, sin borrar sus fallos ni resultados.

- Estado acumulado: 1.1–1.3, 2.1–2.6, 3.1–3.3, 4.1–4.4, 5.1–5.2 y 6.1 se conservan completas; sus pruebas y decisiones permanecen en `tasks.md` y en el histórico siguiente. No se repitió su implementación.
- 6.2 completa como aceptación técnica dirigida de Apply; no equivale a Verify ni QA independientes. Se añadió cobertura propia de scrollback PTY y comandos repetidos con reloj fake, conservando la matriz anterior.
- RED/GREEN de harness: `cargo test -p huginn-presentation --features test-driver --test pty support::screen_line_feed_at_bottom_scrolls_instead_of_overwriting -- --exact`: RED exit 101 (`FIRST` en lugar de `SECOND`); GREEN exit 0, 1 PASS. El observador conserva las filas desplazadas por LF inferior y mantiene scrollback al redimensionar.
- RED/GREEN productivo: `cargo test -p huginn-presentation --features test-driver --test pty overlays_during_streaming_never_commit_ui_or_secrets_to_scrollback -- --exact`: RED exit 101, falta `┌Help` durante streaming retenido. Causa raíz: Enter y completion exigían `!busy` incluso para comandos UI. GREEN exit 0, 1 PASS tras permitir dispatch UI y conservar la barrera busy para prompts conversacionales. Sin rebajar aserciones ni cancelar el streaming para abrir overlays.
- `cargo test -p huginn-presentation --test providers_ui repeated_ui_commands_after_dedup_expiry_preserve_stream_and_conversation -- --exact`: 1 PASS, avance fake de 2001 ms; Help/Providers/Login/Model/Settings/Logout conservan conversación, stream y turno. Caracterización sin RED ficticio.
- `cargo test -p huginn-presentation --features test-driver -- --test-threads=1`: 155 PASS, incluidos 39 PTY, 32 state, 13 workflows, 10 parser y 8 providers_ui. `cargo test -p huginn --test tui --test providers_flow -- --test-threads=1`: 8 + 6 PASS.
- Revisión exhaustiva: `Output` enumera los eventos ignorados, command/workflow validan las seis variantes OverlayKind explícitamente y el wrapper PublicationFault distingue Select/StageSelection de las otras cuatro acciones sin wildcard. Refactor sin cambio de comportamiento, apoyado en suites existentes y nueva consolidación.
- Requisitos leídos: `docs/development.md`, `dev/README.md`, manifiestos y recetas reales. Docker disponible, dependencias/browsers presentes, `just test-e2e-build` exit 0. Primera `just ci-local` terminó exit 0 (479 s, 90 E2E + 210 Vitest); corresponde al estado previo al fix y no acredita por sí sola el candidato final. Segunda ejecución completa en curso sobre el código final.
- Directorio de logs completos: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/huginn-apply6/`. Detalle por comando y REQ en `apply-validation.md`.
- Se conserva `size-exception` aprobada. Ninguna operación de staging, commit, push, branch, merge, PR ni archive. Hash del index sigue `180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`; no se modificó `state.yaml` ni se resetearon datos.
- Consolidación de siete paquetes (`huginn-all-final.log`) terminada exit 0: core, application, runtime, presentation, plugin-api, reference-extension y app, con `--all-features -- --test-threads=1`; dos pruebas opt-in ignoradas (binario legacy y Keychain/Secret Service), no acreditadas.
- Segunda CI (`ci-local-final.log`) terminó exit 1 en 5/9: PTY resize veía título nuevo antes de que el watch actualizara la barra. Causa de harness: dos tareas asíncronas no sincronizadas por ese título. Se conserva expectativa exacta y se espera `Idle|fixture/Bar-Provider` antes de afirmarla; focal exacto exit 0 en `resize-watch-green.log`. Tercera CI completa en curso, `ci-local-verified.log`; no se usa la primera corrida como sustituta.
- `cargo audit --no-fetch` sin máscara exit 0 (`audit-unmasked.log`), sin advisories reportadas. `just doctor` exit 1 porque la receta documentada no existe (`doctor.log`); tooling se verificó mediante comandos reales, sin tocar runners ni Rook. fmt/diff-check propios exit 0 tras los cambios.
- Corrida PTY adicional con paralelismo por defecto (`pty-concurrent-green.log`, nombre no implica éxito): 38 PASS/1 FAIL, exit 101 por `failed to openpty`, OS -6, antes de ejecutar el caso de refresh. Coincidía con CI completa; posible interferencia de recursos, no causa definitiva demostrada. Se conserva fallo y diagnóstico (`pty-resource-diagnostic.log`, límite PTY 511); no se cambia el runner ni se reduce paralelismo de CI.
- Clippy adicional de los siete paquetes Huginn con `--all-features --all-targets -- -D warnings`: exit 0 (`huginn-clippy-allfeatures.log`), incluido test-driver/fixture/PTY. Ejemplo offline `providers_headless`: exit 0. Lint de tasks/progress/reporte: exit 0 tras corregir un code span histórico sin cambiar su literal.
- Gate final leído completo y resumido en `ci-summary.txt`: `just ci-local` exit 0, nueve etapas PASS, 431 s; cargo test workspace/all-features PASS, incluidos 39 PTY (paralelismo por defecto), providers_ui 8, state 32, workflows 13 y app tui 8. Vitest 24 archivos/210 tests; E2E 90/90 en Chromium/Firefox/WebKit, 3.2 min. Logs `ci-local-verified.log`, copia `dashboard-dev.log`. Se conserva la corrida extra con fallo openpty OS -6; el caso refresh pasó en el gate final, sin cambiar runner ni afirmar causa definitiva del fallo de asignación.
- `apply-validation.md` contiene REQ→prueba/evidencia, RED/GREEN, gates reales, fallos previos/UNTESTED y revisión de consumidores exhaustivos. No hay tareas Apply pendientes. Handoff a `sdd-verify` y después `sdd-qa`; no se crean informes de esas fases ni se hace Archive. El orquestador conserva la transición de state.

## Estado actual (2026-10-20, continuación fase 5)

- Confirmé scope con tasks 5.1–5.2, cuatro delta specs relevantes y diseño; no modifiqué `state.yaml`, git staging ni branch.
- RED: agregué prueba PTY fixture Approval en terminal 20x8; falló porque el footer largo no cabía/ni aparecía como texto completo en screen model. RED adicional detectó el helper `wait_for("Permission #")` podía observar borde del panel antes de que pintara el footer; la prueba inspecciona el screen model y espera challenge/campo antes del cierre.
- GREEN: Approval estrecho usa instrucciones compactas `Approve: <challenge>` y `Code: <typed>` en las dos líneas disponibles. Prueba afirma challenge (que incluye el código fresco de ocho chars) y campo.
- `cargo test -p huginn-presentation --features test-driver --test pty minimum_terminal_approval_shows_challenge_and_code_field -- --exact` — 1 passed.
- `cargo test -p huginn-presentation --features test-driver --test pty -- --test-threads=1` — 35 passed, 0 failed; log completo: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-phase5-verified.log`. Evidencia PTY existente incluye `dropping_connection_restores_synchronously` y salida normal/cancel con termios restaurado, bracketed-paste disable, sin alt screen.
- `cargo test -p huginn-presentation status_bar::tests --lib` — 5 passed (ausencia/no false Connected, actividad, diagnóstico, clipping unicode).
- `cargo test -p huginn-presentation --test state` — 32 passed (Approval arm/disarm/lease y estados asociados).
- `cargo fmt --all -- --check` y `git diff --check` pasan; inicialmente fmt detectó formato de test nuevo, se aplicó rustfmt y ambos checks repitieron en verde.
- Sigue incompleto: matrix PTY real de todos los overlays y no-overlay a tamaño habitual/20x8, resize PTY real (incluido resize bajo mínimo→recuperación y re-arm solo post-redraw con identidad vigente), draw zero-size con backend real y prueba de panel largo no conversacional/resumen y barra conectada/no seleccionada. No cierro 5.1/5.2 ni inicio fase 6 hasta aportar evidencia focal de estos criterios.

## Estado actual verificado (2026-10-20, continuación PTY)

- Reproduje la suite serial y los cinco casos del registro anterior. Resultado previo al ajuste de pruebas: 29/34, con los cinco fallos históricos.
- Causa por caso: `stale_and_paste_confirmation_cannot_approve_then_fresh_code_can` usaba `wait_for("END_OF_DIFF")`, aunque el render limita el preview al viewport visible; el texto de marcador existía en el buffer lógico, no en la pantalla. Corregido el criterio a `PREVIEW_FIRST`, elemento que sí es visible, conservando los asserts de código fresco/denegación.
- Los tres casos de selección revocada sí procesaban el flujo de entrada y mostraban Help, pero el helper enviaba Ctrl+C inmediatamente después de encontrar Help; eso cerraba el turno de render/UI, sin garantizar cierre del overlay y salida antes del timeout. Ahora se envía Esc y se espera a que Help desaparezca antes de Ctrl+C. No hay evidencia de regresión productiva en esos tres casos; se conservan las aserciones contra `NEW_COMMAND_EXECUTED`.
- `resizing_during_incomplete_secret_paste_does_not_start_another_reader`: el test dependía de buscar `Form discarded.` en el transcript ANSI bruto, cuyo texto está segmentado por movimientos de cursor. Se cambió a observar el screen buffer reconstruido. Se usa `Pty::resize` para actualizar también la grilla de observación al nuevo tamaño. La aserción de un único cursor query y el descarte de ambos fragmentos se mantienen.
- Resultado fresco: `cargo test -p huginn-presentation --features test-driver --test pty -- --test-threads=1` — **34 passed, 0 failed**. Log completo reproducido por segunda vez: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-final-verified.log` (34/34); corrida previa también verde: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-rerun.log`. Log de reproducción base (29/34): `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-current.log`; log anterior de referencia: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-final.log`.
- No se modificó código productivo: evidencia señala criterios/observación/race en el harness, no comportamiento defectuoso. No hice stage/unstage ni otras operaciones Git; `state.yaml` no se tocó. Las tareas 5.1–5.2 siguen abiertas y sin marcar; no se inició fase 6.

## Histórico de continuación PTY (2026-10-20; hipótesis y fallos supersedidos por el estado verificado arriba)

- Reanudé sobre el worktree con staging preexistente intacto; no hice operaciones de stage/unstage, no toqué `state.yaml` ni código externo de Rook/Settings.
- Investigué la matriz real. El render identifica Help/Settings por su título, pero workflows de Provider/Login/ModelPicker/Confirm se presentan como formulario genérico (p. ej. título real `Bar Provider`, credencial), y las notificaciones al cerrar ayudan a los casos inferiores. La PTY tiene limitación importante: su modelo de pantalla es una grilla de dimensiones iniciales; su `resize()` reinicializa el modelo y esa configuración de test driver no coincide necesariamente con la fila inferior inline de la aplicación. Observé salidas reales contradictorias al interpretar la posición con `status_line()`, por lo que no consolidé una prueba engañosa. La prueba temporal reportó status bar vacía bajo Help; el transcript PTY sí mostraba `Idle|fixture/alpha` en fila 13, lo que confirmó limitación del observador. Eliminé esas pruebas temporales.
- Validé la suite PTY serial completa tras los intentos: 29/34 pasan. Cinco fallan: `approval_does_not_erase_a_revoked_selection` y `revoked_palette_selection_cannot_fall_back_to_replacement_dispatch` completan el cierre de Approval, abren Help y luego el fixture no termina; `ctrl_enter_cannot_execute_a_revoked_selection` queda con `REPLACEMENT_READY` tras Help y no termina; `resizing_during_incomplete_secret_paste_does_not_start_another_reader` falla esperando que la cancelación ocurra durante resize; `stale_and_paste_confirmation_cannot_approve_then_fresh_code_can` no ve `END_OF_DIFF` en el buffer visible (preview recortado, puede ser mal criterio de PTY visible). Log: `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-final.log`. Reproduje en serie las cinco individualmente; no puedo atribuirlas al cambio ni afirmar flakiness: suite no queda verde y requieren triage.
- `cargo fmt --check` pasa. `cargo test -p huginn-presentation --lib status_bar::tests` y `cargo test -p huginn-presentation --test state` pasan (resultados focales consultados; el output quedó truncado por el wrapper).
- No logré implementar/validar una matriz final, un `TestBackend(0,0)` con renderer real y recovery/disarm, ni lifecycle PTY dedicado con bracketed-paste/termios después de shutdown. No se modificó producción ni se cambió el código de pruebas permanente en esta reanudación.
- Por evidencia insuficiente, 5.1 y 5.2 siguen pendientes en `tasks.md`. Esta continuación no está PR-ready. Siguiente paso técnico: aislar los cinco timeouts de PTY y ajustar/crear un harness que pueda comprobar el status de la fila inferior en ambas dimensiones sin depender de stale cells ni de una dimensión de grilla equivocada; luego cubrir el renderer real de tamaño cero y shutdown antes de reconsiderar el checklist.

## Continuación de aviso accionable/status (2026-10-20)

- Causa confirmada en código: `cancel_workflow_at` publica `Notification` (`Form discarded.`), que antes se proyectaba al único `status.notice`; `start_file_index` escribe después `Partial file index...` con `set_notice`, sustituyéndolo. El render de status bar lee ese mismo slot, así que la notificación se veía en un frame y luego desaparecía al refrescar completion/index.
- TDD: agregué primero `status_bar::tests::actionable_notification_survives_informational_completion_refresh`; RED confirmado (recibía `Partial file index...` en vez de `Form discarded.`). GREEN con slot separado de feedback accionable (`feedback`, diagnostic asociado) y notice informativo. Notificación/diagnóstico actualizan feedback y el nuevo `begin()` limpia feedback previo; refrescos informativos no lo borran. Mantiene tipos/eventos/APIs existentes.
- Unitarias focales `cargo test -p huginn-presentation status_bar::tests --lib`: 5 passed. `cargo test -p huginn-presentation --test state`: 31 passed. `cargo test -p huginn-presentation --test workflows`: 13 passed.
- PTY serial independiente posterior: `cargo test -p huginn-presentation --features test-driver --test pty -- --test-threads=1`, 40 passed, log `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-overlay-pty-1792479397.log` (13.51s). Un run serial anterior, `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-overlay-pty-serial-20261020.log`, falló 6/40 en 72.77s, incluyendo `long_plugin_dialog_window_keeps_selection_visible` y cinco tests de cancelación/secretos. El run repetido completo pasa, señal de flakiness/timeout transitorio compatible; no se atribuye causa definitiva ni se ocultan fallos.
- No añadí PTY de persistencia aviso + refresco: el fixture actual no controla/invoca el scanner/index parcial

## Reanudación Apply 5.1–5.2 (2026-10-20)

- Se mantuvo intacto el staging; no se ejecutaron operaciones git.
- Añadí `actionable_feedback_and_diagnostic_reset_on_new_operation` a `tests/state.rs`: verifica que el diagnóstico accionable de error se proyecta y que `begin()` limpia tanto contenido como flag diagnóstico.
- Expandí `overlays_keep_status_visible_at_minimum_inline_size` para ciclo de apertura/cierre e inspección de barra con Help, Settings, Provider, Login, ModelPicker y Confirm a 20x8; además el Approval se verifica a tamaño normal con Challenge/código real. No añadí cobertura extensa de cada overlay a ambos tamaños: queda pendiente para 5.2.
- `cargo test -p huginn-presentation --features test-driver --test pty -- --test-threads=1`: **40 passed**, log completo `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-overlay-pty-followup-final3.log` (13.49s). Incluye `approval_preview_scrolls_to_both_ends_at_minimum_inline_size`, `approval_resize_below_minimum_disarms_until_a_fresh_draw`, `inline_stream_composer_cancel_and_normal_exit_restore_termios`, `dropping_connection_restores_synchronously`, y pruebas de panic/restauración.
- También se ejecutaron `cargo test -p huginn-presentation status_bar::tests --lib` (5 passed), `cargo test -p huginn-presentation --test state` (32 passed), y `cargo fmt --all -- --check` (pasa).
- **Gaps que impiden completar 5.1/5.2**: la matriz completa de *cada* variante de `Overlay` y no-overlay en ambos tamaños no está demostrada; el nuevo chequeo cubre las variantes públicas principales en 20x8 y el Approval normal, complementado por PTYs existentes, pero no una matriz completa. La vista 20x8 no demuestra que el challenge de Approval esté visible (el cuerpo/footer se recortan); PgUp/PgDn de preview sí está cubierto independientemente. Sin test viable de `Rect::default()`/draw zero-size; no se validó recovery y re-arm explícito después de zero-size. Unicode cell clipping ya está en unitarias existente. Bracketed-paste/termios se verifica en salida/cancelación/drop y panic de suite, pero no un caso independiente de shutdown explícito.
- En consecuencia, 5.1 y 5.2 permanecen `[ ]`; no iniciar fase 6. No se afirma aceptación, verify/QA ni cierre de tareas.

## Continuación de Apply fase 5 (2026-10-20; cierre por evidencia)

- Se auditaron test source y logs históricos. El checkpoint anterior estaba desactualizado: ya existían matriz en ambos tamaños, TestBackend zero-area, matriz status bar, Approval 20x8 y pruebas terminales de cancel/drop.
- RED/GREEN: amplié `approval_resize_below_minimum_disarms_until_a_fresh_draw` para capturar el código vigente en 8x20, cambiar a 1x1, enviar el código válido sin obtener aprobación y recuperar 8x20; el mismo código se acepta tras el redraw. El intento inicial falló al buscar `"Type "` que no cabe en el viewport pequeño y luego al asumir que el submit desarmado debía resolver la solicitud; ajusté el test a su contrato observado: no aceptación bajo mínimo y autorización post-redraw.
- `dropping_connection_restores_synchronously` abre el fixture `drop`; fixture descarta la conexión y afirma que `is_raw_mode_enabled()` es falso antes de salir. No inspecciona ICANON/ECHO. `inline_stream_composer_cancel_and_normal_exit_restore_termios` sí verifica ICANON/ECHO después de cancelación + salida normal, bracketed-paste disable y ausencia de alternate screen.
- Ejecuté filtros exactos de resize, drop y cancel/salida (1/1 cada uno). PTY serial `cargo test -p huginn-presentation --features test-driver --test pty -- --test-threads=1`: 37/37, log completo `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-pty-apply-phase5-final.log`.
- `cargo test -p huginn-presentation status_bar::tests --lib`: 5/5, log `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-statusbar-phase5-final.log`; `cargo test -p huginn-presentation --test state`: 32/32, log `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-state-phase5-final.log`.
- `cargo fmt --all -- --check` y `git diff --check` pasan tras rustfmt; logs `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-fmt-phase5-final.log` y `huginn-diffcheck-phase5-final.log`.
- Evidencia combinada cierra 5.1/5.2 y corrige sus checkpoints, sin atribuir cobertura termios explícita al caso drop.
