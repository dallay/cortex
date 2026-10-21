# Tareas: sistema de overlays de Huginn

## Contrato de ejecución y revisión

Cierre de Apply 2026-10-20: 21/21 tareas completas, incluidas 6.2–6.3.
`just ci-local` final PASS 9/9 (431 s); detalles, fallos previos y límites en
`apply-validation.md` y `apply-progress.md`. Verify/QA independientes siguen
pendientes; ningún checkpoint anterior sustituye esta evidencia nueva.

Estado del worktree validado el 2026-10-19 sobre `HEAD=442e34b`: 1.1–1.3, 2.1–2.6 y 3.1–3.3 completas como implementación, con evidencia propia en `apply-progress.md`. El HEAD sin estos cambios y los checks históricos no acreditan esta implementación. Staging previo preservado, incluidos `event.rs`, `overlay.rs` y artifacts. El contrato 2.1 permite Help/Settings desnudos y exige workflows reales para las demás superficies; 2.2 conecta aperturas, payloads, continuaciones, render e input a `UiState.overlays`, sin slots activos paralelos. 2.3–2.6 endurecen/cubren esos contratos; Clippy focal all-targets/test-driver pasa tras refactor pequeño de event.rs y limpieza de lints en tests del cambio. Verify/QA y CI completa no ejecutados; no se afirma aceptación de producto ni preparación para PR. Prevalecen propuesta,
cuatro delta specs y diseño sobre sugerencias incompatibles de exploración. Solo el
orquestador modifica `state.yaml`; este documento no autoriza commits, ramas,
publicación, merge ni archive.

### Previsión de carga de revisión

- Presupuesto: 400 líneas modificadas, adiciones más eliminaciones.
- Carga estimada: alta; aproximadamente 2500–5500 líneas, incluidos consumidores,
  pruebas, fixtures y documentación. Estimación de planificación, no diff medido.
- Estrategia: `size-exception`, una entrega P0+P1 en un PR, autorizada expresamente
  por la misión y la propuesta. Justificación: migración transversal con rollback
  conjunto de contratos y consumidores, sin imponer otra decisión de entrega.
- Las cadenas serían recomendables por tamaño; **no se adoptan** contra la entrega
  única autorizada. Los paquetes siguientes son unidades de revisión dentro del PR,
  no PRs nuevos ni commits obligatorios.

Decisión de entrega pendiente: No; se mantiene la autorización de un solo PR P0+P1.
La autorización del usuario integró 2.2 en una sola misión, ahora implementada y
probada técnicamente; no equivale a aceptación de producto ni inicia automáticamente tareas dependientes. La estimación sigue
siendo planificación, no diff medido.

Paquetes revisables después del rebaselining:

- A: eventos/categorías/deduplicación (1.1–1.3), completo en worktree con evidencia focal; no implica cambios ya integrados en HEAD.
- B1: contrato plugin (2.1), completo en worktree con pruebas de invocación, metadata, continuación y rechazo sin reemplazo.
- B2: autoridad host, adaptadores, lifecycle e input/render (2.2), completo en el
  worktree con pruebas host y PTY propias; pendiente de revisión del orquestador,
  sin sustituir verify/QA posteriores.
- B3: endurecimiento de workflow/Approval/input y providers (2.3–2.6), completo
  como implementación y suites focales; no sustituye verify/QA.
- C–F: Settings, conexión/barra y retiro de legacy; dependen de la autoridad común.

No se crearon PRs ni commits separados; los paquetes son puntos de revisión dentro
de la entrega única autorizada.

Paquetes revisables, dependencias y esfuerzo orientativo:

- A: eventos/categorías/dedup (1.1–1.3), 2–3 sesiones.
- B: contratos, autoridad integrada, adaptadores y flujos de overlay (2.1–2.6), depende de A, estimación original 5–7 sesiones; revisar tras el rebaselining.
- C: Settings con persistencia asíncrona (3.1–3.3), depende de B, 2–3 sesiones.
- D: selección observable y lifecycle (4.1–4.4), depende de A; integración con B/C,
  3–5 sesiones. Contratos nuevos y todos sus implementadores cambian juntos.
- E: barra/viewport real (5.1–5.2), depende de B/C/D, 2–3 sesiones.
- F: retirada legacy y cierre verificable (6.1–6.3), depende de A–E, 1–2 sesiones.

Cada sesión cubre una unidad lógica; si excede la sesión, subdividir conservando
pruebas y comportamiento juntos, sin cambiar entrega. Los puntos de pausa seguros
son finales de tareas con consumidores adaptados y verificación focalizada verde;
RED es un estado de trabajo local, nunca un checkpoint entregable deliberadamente
incompilable. Adaptadores strangler-fig temporales tienen un solo dueño por
transición: nunca doble input/render, emisión ni escritura. Rollback de entrega:
revertir el PR completo, sin migraciones destructivas ni reset de datos.

### Modo de pruebas y evidencia

La configuración heredada declara `testing.strict_tdd: false`; no hay activación
explícita de strict TDD en esta misión. No se impone RED global para cada tipo
nuevo. **Todo comportamiento incorrecto exige regresión que falle antes del fix**.
En cada tarea: preparar prueba/fixture, registrar RED de regresión, implementar,
comprobar GREEN y refactorizar volviendo a comprobar. Incorporar esas pruebas
con la tarea, nunca aplazarlas a un bloque final genérico.

Para cada RED registrar comando exacto, filtro, caso, código de salida y aserción
fallida saneada; un fallo de compilación por API inexistente no demuestra la
regresión. Si exige un seam nuevo, introducirlo compatible y demostrar el fallo
conductual antes del fix. GREEN registra la repetición del mismo comando y su
salida; ningún caso omitido se marca aprobado. Barreras/canales y reloj fake,
sin sleeps para demostrar orden, dedup o concurrencia; timeouts solo como watchdog.

Runners comprobados por lectura de manifests, targets y `justfile`, **no ejecutados**.
En tareas, `P`, `A`, `R`, `U`, `X`, `H`, `C` abrevian respectivamente los paquetes
`huginn-presentation`, `huginn-application`, `huginn-runtime`, `huginn-plugin-api`,
`huginn-reference-extension`, `huginn`, `huginn-core`. `T(P; state,workflows)`
significa `cargo test -p huginn-presentation --test state --test workflows`;
`L(P)` significa `cargo test -p huginn-presentation --lib`. Expandir siempre los
alias en los informes. Agregar un filtro existente para el RED y repetir el
target completo para GREEN.

PTY significa `cargo test -p huginn-presentation --features test-driver --test pty`;
parser significa el mismo comando con `--test input_parser`. Ambos requieren Unix;
`pty` contiene las pruebas, `tui-fixture` es el bin auxiliar, no su runner.
`--all-features` **sí** activa `test-driver`; comprobar número de tests ejecutados,
no confundir compilación del fixture o cero tests con evidencia PTY.

Rutas abreviadas: `P/`, `A/`, `R/`, `U/`, `X/`, `C/` son
`crates/huginn/{presentation,application,runtime,plugin-api,reference-extension,core}/`;
`H/` es `apps/huginn/`. Son rutas concretas relativas al workspace.

## Fase 1 — Categorías, conversación y errores acotados

- [x] **1.1 Separar superficies explícitas** — `P/src/{event,lib,state}.rs`,
  `H/src/main.rs`, `P/tests/support/fixture.rs`, `P/tests/{state,extensions}.rs`.
  Dependencia: ninguna. EVT-01/02. Sustituir puerto `notice` por métodos object-safe
  de cinco categorías; adaptar todos los implementadores/callers del puerto en
  esta unidad. Renombrar `committed` a `conversation` con compatibilidad interna
  temporal solo donde la migración lo necesite. Pruebas: mismo texto en categorías
  distintas, Session/Workspace fuera de conversación, tool error completo,
  streaming, reproyección idempotente y mensajes idénticos legítimos.
  Aceptación/verificación: solo Conversation alimenta scrollback; `T(P; state,extensions)`.
  Riesgo: perder renderers o introducir clasificación por texto; prohibir ambos.
  Evidencia apply 2026-10-19: se reemplazó el puerto `notice` de Interaction con métodos object-safe para cinco categorías; migrados callers del host/app/fixture y `committed` renombrado a `conversation`. Session/Workspace se proyectan como Status; ToolFinished mantiene su payload íntegro en conversación/stream; errores engine se proyectan diagnóstico sin texto heurístico. Tests `state`, `extensions`, `workflows`, `providers_ui`, `apps/huginn providers_flow` pasan. Hay RED/GREEN observado en el histórico de aviso contaminando conversación; reproyección y repetición legítima están probadas. Resultado completo de las suites focales abajo en apply-progress.

- [x] **1.2 Implementar retención y dedup** — `P/src/{event,state}.rs`, pruebas
  unitarias de `event.rs` y `P/tests/state.rs`. Depende de 1.1. EVT-03/04.
  Probar hash+igualdad por operación, colisión forzada, t=0/1,999/2s,
  reanclaje solo en emisión visible e intercalación A/B. Probar FIFO 256/257,
  dedup 256 entradas/256KiB, 4096 bytes exactos y mensaje mayor no cacheado
  ni truncado para comparar; purga expirada y expulsión de la emisión más antigua.
  Aceptación/verificación: saturación nunca oculta error nuevo, ningún secreto
  retenido; `L(P)` y `T(P; state)`, reloj fake monotónico sin sleeps.
  Riesgo: dedup por hash solo o ventana extendida por supresión.
  Reconciliación (continuación 2026-10-19): criterios funcionales EVT-03/04 de 1.2 cubiertos por pruebas unitarias y de productor UI; `T(P; state)` corrido y pasa (17), pero el task requería explícitamente completar 1.3 antes de esta aceptación integrada; **1.2 queda [x]** tras separar sus criterios funcionales de los de integración 1.3; acceptance suites requeridas ejecutadas en este worktree (27 lib y 17 state). La propagación de scope y recuperación autoritativa son criterios propios de 1.3, no criterios añadidos a 1.2. El deduplicador conserva igualdad exacta cifrada AES-256-GCM con clave aleatoria por instancia, nonce de contador y tamaño cifrado contabilizado (mensaje + 28 bytes); mantiene hash+igualdad ante colisiones, límite de 256 entradas/256 KiB, ventana y limpieza. El oversized (4097 bytes) pasa sin cache/truncado, 4096 exactos se comparan, hay prueba de ausencia del secreto en estado retenido y saturación/expulsión. RED observado en esta continuación: `cargo test -p huginn-presentation --lib event::tests::dedup_nonce_exhaustion_emits_without_panicking_or_caching` falló por panic al agotar contador; GREEN tras cambiar agotamiento a emisión visible sin cache, limpiando entradas para impedir reutilización de nonce. RED adicional: `cargo test -p huginn-presentation --lib event::tests::dedup_enforces_entry_byte_and_oversize_bounds_without_truncation` mostró que la prueba de FIFO erróneamente asumía que el segundo-oldest seguía retenido después de reinsertar el oldest, que vuelve a expulsar otra entrada; se corrigió el test para comprobar lo requerido: oldest expulsado y reinserción visible. Checks exactos más recientes: `cargo test -p huginn-presentation --lib` (26 passed, antes de la última aserción de FIFO; focal de bounds pasó luego), `cargo test -p huginn-presentation --lib event::tests::dedup_enforces_entry_byte_and_oversize_bounds_without_truncation` (1 passed), `cargo test -p huginn-presentation --lib event::tests::dedup_nonce_exhaustion_emits_without_panicking_or_caching` (1 passed), `cargo test -p huginn-presentation --test state` (17 passed), `cargo test -p huginn-application --test contracts` (4 passed). No se reejecutaron todas las suites del workspace ni `L(P)` completa. `cargo fmt --all -- --check` y `git diff --check` pendientes al cierre de esta continuación. La propagación y verificación de 1.3 siguen siendo prerrequisito; no afirmar aceptación 1.2 antes de ese gate.
- [x] **1.3 Completar scopes y conservación autoritativa** — `P/src/{lib,state}.rs`,
  `H/src/main.rs`, `P/tests/support/fixture.rs`, `A/src/loop_engine.rs`,
  `A/tests/contracts.rs`, `R/tests/{coding_workflow,stream_persistence,sessions_persistence}.rs`.
  Depende de 1.2. EVT-01–04. `begin` retorna OperationId; fallback app/fixture y
  Event::Error comparten el id del turno. Continuaciones heredan el suyo; nuevas
  invocaciones y tareas independientes no adoptan el turno. Finalizar después de
  todos los productores/resultados; filtrar una vez antes de elegir destino UI.
  Probar error engine+fallback sin doble superficie y limpieza de operaciones;
  sesión >256 eventos recuperable íntegra, sin cambiar `record` ni Session.events.
  Aceptación/verificación: `T(A; contracts)`, `T(R; coding_workflow,stream_persistence,sessions_persistence)`
  y `T(P; state)`. Riesgo: liberar scope antes del fallback o capar persistencia.
  Evidencia apply 2026-10-19: flujo engine-error/fallback en `main.rs` pasa por el mismo `ScopedEventSink` antes de `finish_operation`; `engine_and_host_fallback_errors_share_one_operation_diagnostic` verifica una sola superficie. El runtime usa un sink prestado (`&dyn EventSink`) a través de awaits de loop, turn y ejecución secuencial de tools; no lo clona ni lo entrega a tareas `'static`, por lo que no hay productor UI independiente que pueda heredar scope implícitamente en el alcance actual. Test `operation_scope_survives_tool_continuation_and_is_distinct_per_run` captura `emit_for` con `OperationId` real de cada emisión: herramienta, respuesta de continuación y fin de turno guardan el mismo ID; segunda llamada `run` registra otro ID. `explicit_event_operation_scope_survives_interleaved_producers` verifica dedup por operación en la proyección de producción; emisión posterior a finish se rechaza. Persistencia de sesión conserva 300 eventos; `record`/`Session.events` no se capan. Suites focales requeridas ejecutadas: application contracts 4, runtime coding_workflow 28, stream_persistence 4, sessions_persistence 2, presentation state 18, todas pass. Por tanto 1.3 queda completa.

## Fase 2 — Un dueño de overlay, efectos y foco

- [x] **2.1 Añadir contratos plugin completos** — `U/src/v1.rs`,
  `U/src/v1/workflow.rs`, `U/tests/{contract,workflows}.rs`, `X/tests/smoke.rs`,
  `P/src/state.rs` y `P/tests/workflows.rs`. Depende de 1.1. OVL-02/05.
  Añadir OverlayKind sin Approval, `CommandResult::Overlay`, metadata opcional y
  validadores exhaustivos en Command::run y Workflow::execute; actualizar ambos
  dispatch y smoke juntos. Help/Settings desnudos permitidos; los demás requieren
  workflow real, builder solo Provider/Login/ModelPicker/Confirm. Mantener Notice
  saneado y Dialog compatible, y rechazo de lookup de diálogo desde async workflow.
  Probar allowlist, rechazo sin reemplazo y binding de registration exacta.
  Aceptación/verificación: `T(U; contract,workflows)`, `T(X; smoke)`, `T(P; workflows)`.
  Riesgo: perder payload/callback o convertir Confirm en aprobación host. Evidencia 2026-10-19: validadores exhaustivos y builder ya presentes caracterizados sin RED ficticio; pruebas nuevas cubren las seis superficies por invocación, cuatro tipos de workflow, metadata propia de continuación (incluida None), definición/callback/campos, revocación y re-registro sin usar sustituta, Notice saneado/acotado y rechazo de Dialog async. RED conductual en presentation por categoría Overlay en lugar de Notification y mensaje async sin rechazo explícito; GREEN tras corregir ambos dispatch. Rechazos preservan payload, selección y workflow_focus actuales y conversación. Suites PASS: plugin-api contract 11/workflows 9, reference-extension smoke 1, presentation workflows 6/extensions 2. Apertura real y autoridad única siguen siendo aceptación de 2.2, no precondición circular de este contrato.

- [x] **2.2 — Migración vertical de autoridad overlay (misión coordinada; no marcar
  completa hasta satisfacer todo el checklist y la aceptación host-level)** —
  `P/src/{overlay,lib,state,workflow,contributions,approval,settings,terminal,input}.rs`,
  unitarias de `overlay.rs`, `P/tests/{state,workflows,extensions,input_parser,pty}.rs`,
  `P/tests/support/{fixture,pty}.rs`. Depende de 2.1; OVL-01–05, EVT-01, SET-12.
  Los subpasos a/b/c son fases internas dependency-ordered de esta única tarea, no
  tareas autorizables ni aceptables por separado. No detener ni declarar éxito tras
  crear la autoridad: ambos consumidores y payloads deben migrar en el mismo corte.

  Subchecklist interno — no marcar parcial como tarea completa:
    - [x] **a. Autoridad y transiciones:** crear `Overlay`, `ActiveOverlay`, origen,
      operación, ticket, revisión y lease; `UiState` dueño activo único. Abrir/cerrar
      realmente todas las superficies con payload exacto; stale revision/lease y
      rechazo sin alterar foco/copia. Snapshots seguros, sin exponer secretos.
    - [x] **b. Adaptadores y lifecycle:** migrar Dialog/Handle, HostWorkflow/Pending,
      Approval host-only con oneshot/epoch y preemption, SettingsDialog y widgets como
      payloads; preservar callbacks, continuación, registro exacto, workflow lifecycle
      y receipts. Cancelación/revocación/abort y Drop fuera del mutex. Settings conserva
      DTOs/imports públicos y semántica; sin slot paralelo ni persistencia accidental.
    - [x] **c. Render/input y aceptación host-level:** snapshot, render e input consumen
      la misma autoridad; sin ownership activo duplicado. Approval solo host, one-shot,
      preempta y rechaza aperturas ordinarias. Mantener stale revision/lease, ownership
      de paste y cierre consumido. Sin toast layer; `conversation` mantiene semántica.
      Efectos/callbacks/drop fuera del mutex. Adaptar fixture/PTY y todos los consumidores.

  Pruebas RED/GREEN dentro de la misma misión para reemplazo válido/rechazado, payload
  exacto, stale ticket/continuation, secretos en snapshot, callback/Drop reentrantes,
  cancelación local, Approval/preemption, workflow lifecycle/receipt, Settings sin
  escritura prematura, y equivalencia de fuente de render/input. La aceptación exige
  conjuntamente `L(P)`, `T(P; state,workflows,extensions)`, parser y PTY con tests
  ejecutados, además de aceptación host-level: cada superficie se abre, actualiza y
  cierra a través de la autoridad común; payloads se preservan; no quedan escritores,
  renderers ni lectores de input activos alternativos. Un subpaso verde no acepta 2.2.
  Riesgo: migración incompleta, doble autoridad o pérdida de funcionalidad.

  Evidencia apply 2026-10-19: `UiState.overlays: OverlayMachine<Payload>` es el
  único dueño; retirados slots y leases paralelos. Help real, Settings con copia y
  receipts serializados, Dialog con handle exacto, workflows con secretos/callbacks/
  continuaciones y Approval host-only conservan sus payloads. Tickets/leases se
  revalidan antes de efectos; recursos retirados, cancel/abort, callbacks, save y
  renderers se procesan fuera del mutex de UI. RED conductuales observados para
  Help, reemplazo por Settings, waiter abortado, renderer bajo mutex, agotamiento
  al cerrar y asignación de operación desde ticket obsoleto; GREEN del mismo
  comando para cada caso. Suites finales sin filtros de casos:
  `cargo test -p huginn-presentation --features test-driver -- --test-threads=1`
  (132 tests: 31 lib, 24 state, 12 workflows, 2 extensions, 9 parser, 35 PTY,
  19 adicionales), y `cargo test -p huginn --test providers_flow` (6), exit 0.
  Formato y diff check pasan. Clippy opcional falla exclusivamente en dos lints
  del `event.rs` previo, documentados en progreso; CI/verify/QA no ejecutados.

- [x] **2.3 Endurecer workflow/dialog y resultados exhaustivos residuales** —
  `P/src/{contributions,workflow,state,terminal}.rs`,
  `P/tests/{workflows,extensions,completion}.rs`. Depende de la aceptación completa de 2.2; OVL-01/02/04/05,
  EVT-01. No volver a migrar ownership ni crear otro router: cubrir `prepare_submission`,
  datos capturados, campos vacíos autoejecutables, secretos Zeroizing hasta 4096 bytes,
  `FormInput.secrets`, máscara constante y routing exhaustivo de resultados/continuaciones.
  RED/GREEN para revocar y re-registrar mismo nombre y completion tardía. Verificar
  `T(P; workflows,extensions,completion)`. Riesgo: callback ligado a registro sustituto.
  Evidencia 2026-10-19: se reutilizan regresiones de revocación/re-registro y resultado
  tardío de 2.2; nueva caracterización comprueba UTF-8/4096 bytes, máscara constante,
  borrado/saneado, FormInput separado, captura inmutable y submission única. Conducta
  ya implementada: sin RED artificial ni segundo router. Suites completas 13/2/9 PASS.

- [x] **2.4 Endurecer Approval host-only y fail-closed** —
  `P/src/{approval,overlay,state,terminal}.rs`, `P/tests/{state,pty}.rs` y
  `P/tests/support/fixture.rs`. Depende de la aceptación completa de 2.2 y 2.3; OVL-03/04, SET-12.
  Mantener sender único, requests serializadas, epoch/challenge/armed y resolución
  exactamente una vez; no reconstruir ni reubicar Approval. RED/GREEN para lease/epoch
  viejo, callback tardío, cancelación prioritaria, Esc/Ctrl+C, canal cerrado, shutdown,
  driver drop y aceptación solo con input fresco post-draw. Verificar `T(P; state)` y
  PTY con restauración síncrona. Riesgo: cualquier aceptación indebida bloquea.
  Evidencia 2026-10-19: caracterización determinista mediante poll de futures reales
  demuestra serialización, lease anterior inoperante, cancelación prioritaria con
  answer ya ready y shutdown de waiter activo/en cola. Se conservan pruebas de abort,
  epoch/challenge, preemption y PTY de armado/input fresco/paste/restauración. Sin
  duplicar Approval ni inventar RED para garantías existentes. State 26 y PTY serial 35 PASS.

- [x] **2.5 Endurecer input/paste sin segundo ownership** —
  `P/src/{input,terminal,overlay}.rs`, `P/tests/{input_parser,pty}.rs` y fixture.
  Depende de la aceptación completa de 2.2 y 2.4; OVL-04. Conservar generación terminal, drenaje hasta tick
  quieto, dueño de paste hasta delimitador, cancelación local por Ctrl+C y rechazo/
  zeroización de paste obsoleto. RED/GREEN para centinela secreto partido y teclas
  desconocidas/textuales y no textuales consumidas en todas las superficies; nunca
  entregar fragmentos al editor/completion ni aceptar Approval. Verificar parser y
  PTY. Riesgo: fuga al cambiar foco o armado con input previo.
  Evidencia 2026-10-19: regresión RED real por sufijo CSI desconocido >64 bytes
  liberado como texto; GREEN al conservar cuarentena hasta byte final, incluso con
  nuevo foco. Se mantiene parser/ownership existente; pruebas reutilizadas cubren
  centinela partido, Ctrl+C local, zeroización y nueva generación. Se ampliaron
  superficies tipadas con eventos desconocidos/no textuales/release consumidos.
  Parser 10, workflows 13, PTY serial 35 PASS.

- [x] **2.6 Preservar flujos Provider funcionales sobre autoridad integrada** —
  `P/src/{providers,overlay,terminal}.rs`, `P/tests/providers_ui.rs` y
  `H/tests/providers_flow.rs`. Depende obligatoriamente de 2.2 aceptada y de 2.4–2.5;
  OVL-05, EVT-01. Etiquetar carga y cada continuación de login/connection_form,
  providers, model_form y logout Confirm sin sustituir payloads ni workflow real.
  RED/GREEN para apertura host, recover/list/status/save/select_model/revoke,
  endpoint/modelo inválido, elección Secure/Ephemeral explícita, fallo sin fallback,
  OAuth no soportado y cancelación recuperable. Verificar `T(P; providers_ui)` y
  `T(H; providers_flow)`; cero placeholders y resultados fuera de conversación.
  Riesgo: Provider funcional antes de tener autoridad integrada o perder continuación.
  Evidencia 2026-10-19: RED real Workflow != Login en apertura del flujo existente;
  GREEN etiquetando carga y continuaciones Login/Provider/ModelPicker/Confirm sin
  sustituir callbacks ni datos. Pruebas reales de save/select/update/revoke ampliadas
  con etiquetas, Confirm desde Providers/cancelación recuperable y endpoint/modelo
  inválidos antes de escribir credenciales. Reutilizados secure failure sin fallback,
  Ephemeral explícito, OAuth y persistencia/revocación. Providers UI 7 y app flow 6 PASS.

## Fase 3 — Settings, copia y receipts

- [x] **3.1 Integrar Settings sin cambiar su shell** —
  `P/src/{settings,overlay,state,terminal,contributions}.rs`,
  `P/tests/{completion,plugins,state}.rs`, unitarias de `settings.rs`.
  Depende de 2.6 y usa exclusivamente la autoridad aceptada en 2.2. SET-01–06/08/12–16. Mantener DTOs/imports públicos en settings.rs,
  cinco tabs, filas/orden/markers, navegación y copia sin persistir al moverse;
  `Settings` es payload de la autoridad compartida, nunca un slot `SettingsDialog`.
  Esc consume tecla, no cancela turno; antes Apply no escribe. `/settings` y
  whitespace no envían prompt; catálogo independiente de modelo; Add provider
  conserva cierre/aviso Notification sin tocar providers ni guardar.
  Probar reemplazo autorizado y rechazo conservando copia/lease, sin restauración.
  Aceptación/verificación: `L(P)`, `T(P; completion,plugins,state)`.
  Riesgo: confundir selección con valor guardado o activar onboarding/tema live.
  Evidencia 2026-10-19: shell integrado en 2.2 conservado; nueva caracterización
  host de catálogo con/sin modelo, whitespace, navegación por cinco tabs, selección
  distinta del marker, copia descartada y Add provider sin persistencia ni apertura
  Provider. Se reutiliza reemplazo Settings→Dialog y rechazo/preemption de 2.2/2.4.
  Lib 31, completion 9, plugins 2 y state 27 PASS al checkpoint de esta tarea.

- [x] **3.2 Coordinar Apply fuera del lock** — `P/src/{settings,overlay,state}.rs`,
  `P/tests/state.rs` y unitarias de `settings.rs`. Depende de 3.1. SET-07/08/12,
  OVL-03/04. Worker bloqueante invoca closure inyectada; coordinador separado
  serializa guardados y receipts retienen solo copia/resultado necesarios.
  Regresiones RED previas: closure reentrante no bloquea input, disk full conserva
  copia/error solo en cuerpo y Enter repetido no duplica. Barreras para orden de
  dos guardados, fallo/reintento, Esc/reemplazo/Approval tras iniciar escritura,
  receipt éxito/error con otro Settings abierto, shutdown y completion tardía.
  Aceptación/verificación: `L(P)`, `T(P; state)`; éxito actualiza snapshot, cierra y
  notifica solo overlay vigente; tardío nunca reabre ni altera nueva copia/Approval;
  fallo tardío no publica valores fallidos. Riesgo: rollback prometido o doble write.
  Evidencia 2026-10-19: coordinador real ya integrado en 2.2, sin duplicarlo. Pruebas
  de barreras existentes cubren closure reentrante, Enter único, error/reintento,
  orden entre dos guardados y receipt con otro Settings/Approval. Se amplía Esc con
  turno activo durante escritura y se añade shutdown con éxito/error tardío sin
  reapertura/aviso ni segunda escritura. Lib 31 y state 28 PASS; sin RED ficticio.

- [x] **3.3 Preservar persistencia/config y modo línea** —
  `H/src/{config,ui_settings,main,terminal}.rs`, `H/tests/{config_compat,cli,tui}.rs`,
  `H/config.example.toml`; conservar/regresar unitarias del helper.
  Depende de 3.2. SET-09/10/11/15 y 07. Probar TOML completo/parcial/ausente,
  unknown internos, comentarios/orden y bytes fuera de ui, Apply conjunto;
  line-mode sin catálogo/overlay Settings. No mover UiConfig ni modificar schema.
  Aceptación/verificación: `T(H; config_compat,cli,tui)` y `cargo test -p huginn --bin huginn`.
  Riesgo: prueba antigua de Settings-modal no ejecutada tomada como evidencia;
  obtener evidencia propia y no activar consumidores live de preferencias.
  Evidencia 2026-10-19: regresión RED Config::load con ui ausente cargaba strings
  vacíos/indicators false en vez de defaults documentados; GREEN al unificar
  UiConfig::defaults/Default y defaults de DTOs anidados, sin mover tipos/schema.
  Ausente/parcial/nested parcial probados por Config::load; helper ampliado con
  round-trip completo, unknown anidados y bytes externos estables. Modo línea
  recibe /settings sin overlay ni secuencias TUI. Bin 15, config_compat 5, cli 7 y
  tui serial 7 PASS, sin evidencia prestada de settings-modal ni consumidores live.

## Fase 4 — Selección confirmada observable y lifecycle

- [x] **4.1 Publicar snapshot versionado desde router** — `C/src/{lib,connections,providers}.rs`,
  `A/src/{providers,connections}.rs`, `A/tests/{providers_contracts,connections}.rs`,
  `A/examples/providers_headless.rs`. Depende de 1.3. BAR-02/03.
  Añadir ConnectionChanged y snapshot serializable seguro, UUID source por
  activación, revision creciente, phase y selección opcional; watch con suscripción
  y send_replace bajo mismo mutex. Ampliar ambos puertos/implementadores/wrappers,
  StageSelection y publish_selection exacto, mantener Select directo Unmanaged y
  Legacy distinguido. Probar ausencia/candidato/confirmación y serde sin secretos.
  Aceptación/verificación: `T(A; providers_contracts,connections)` y
  `cargo check -p huginn-application --examples`. Riesgo: anunciar Connected por selección.

- [x] **4.2 Generalizar guardias de publicación a todas las rutas** —
  `A/src/connections.rs`, `A/tests/connections.rs::PublicationFault`,
  `A/tests/providers_turns.rs`, `R/tests/providers_integration.rs`.
  Depende de 4.1. BAR-02/03, OVL-05. StageSelection instala candidata sin anunciar
  Selected; confirmar solo tras persistencia/publicación y comparación exacta
  bajo operación serializada. ModelPublication cubre select_locked, cambio modelo,
  restore/recover y resolve explícito; error/drop/cancel limpia fail-closed.
  Wrapper reenvía subscribe/publish e intercepta StageSelection además de Select.
  Probar con barreras fallo antes/después de publicar, Err/Pending después de
  efecto, drop/cancel en cada ruta y comparación invalidada; MetadataMutation e
  invalidate_all no toman mutex async desde Drop. Aceptación/verificación:
  `T(A; connections,providers_turns)`, `T(R; providers_integration)`; nunca B ficticia.
  Riesgo alto: fault injection inoperante o guard desarmado antes de confirmar.

- [x] **4.3 Cerrar observabilidad en todos los teardowns** —
  `A/src/{providers,connections}.rs`, `A/tests/{connections,providers_contracts}.rs`,
  `C/tests/{kernel_lifecycle,registration_ownership}.rs`.
  Depende de 4.2. BAR-02/03. RegistrationLease, revocación/Replace del seleccionado,
  cancelación/unload adapter, ManagerLease y CoreLease publican ausencia con
  Revoked/Unavailable antes de teardown. Comprobar owner: manager retirado no borra
  nueva generación; revocar otra conexión no borra seleccionada.
  Probar cada hook y reactivación source nuevo con barreras, incluyendo teardown
  concurrente a publicación. Aceptación/verificación: `T(A; connections,providers_contracts)`
  y `T(C; kernel_lifecycle,registration_ownership)`. Riesgo: publicación por owner obsoleto.

- [x] **4.4 Conectar watch sin carrera y migrar consumidores Event** —
  `H/src/{main,terminal}.rs`, `P/src/{lib,state,event}.rs`, `P/tests/state.rs`,
  `P/tests/support/fixture.rs`, `A/tests/{contracts,providers_turns}.rs`,
  `R/tests/{coding_workflow,providers_integration,stream_persistence}.rs`.
  Depende de 3.3 y 4.3. BAR-02/03, EVT-02. Inyectar receiver desde manager o router;
  cada open clona y usa borrow_and_update antes del primer draw, luego changed.
  Attachment fija source y acepta solo revisiones mayores; canal cerrado produce
  Unavailable únicamente si attachment vigente. Probar arranque concurrente,
  A→B y A tardío, duplicado B, reconexión/source distinto y cierre task retirado.
  Output::emit trata ConnectionChanged explícito: no conversación, JSON público
  seguro si recibe evento. Probar serde aditivo/sesión antigua sin backfill y sinks
  sin cap UI. Aceptación/verificación: `T(P; state)`, `T(A; contracts,providers_turns)`,
  `T(R; coding_workflow,providers_integration,stream_persistence)`, `T(H; providers_flow)`.
  Riesgo: snapshot independiente antes de suscribir o fallback antiguo borra B.

## Fase 5 — Barra persistente en terminal inline real

Checkpoint actualizado 2026-10-20 tras auditar pruebas/código. Se comprueba barra PTY
con selección en composer sin overlay y en Provider, además de matriz a 100x30 y 20x8
para Help, Settings, Provider, Login, ModelPicker y Confirm. Approval se verifica a
20x8 con challenge/campo; TestBackend 0x0 prueba no-armado y draw viable con identidad
vigente. El caso `approval_resize_below_minimum_disarms_until_a_fresh_draw` adicional
prueba PTY: captura challenge, resize a 1x1, envía el código válido y comprueba que no
se aprueba; al restaurar 8x20 el mismo código solo se acepta tras redraw. Drop comprueba
restauración síncrona de raw mode, ICANON y ECHO; cancelación/salida normal comprueba
ICANON/ECHO, bracketed-paste disable y ausencia de alternate screen. Caso PTY serial
37/37 anterior a esta actualización; ejecutar suite serial de nuevo para evidencia
consolidada. No queda vacío de escenario para los criterios de 5.1/5.2; confirmar
resultado de nueva suite y fmt/diff-check antes de marcar.

Evidencia de implementación fase 4 (2026-10-19): snapshot seguro con UUID por
activación, revisión monotónica y watch publicado/suscrito bajo mutex del router.
StageSelection no permite admisión ni anuncia selección; publish_selection exige
candidata exacta y activa. Select directo conserva modo Unmanaged; seed Legacy
distinguido. Guardias de publicación cubren select, modelo, restore/recover y
resolve explícito. Fault injection real antes/después de StageSelection (Err o
Pending/drop), Err después de commit y candidata invalidada cubren todas esas
rutas. Revocación/Replace, registration, cancelación propia, manager/core teardown
publican ausencia; lease viejo no borra nueva generación. Cada open clona watch,
lee borrow_and_update y atiende changed; attachment/source/revisión bloquean
actualizaciones y cierre retirados. Event aditivo fuera de conversación y sesión
antigua sin backfill probados. RED reales y comandos completos en apply-progress.
Suites focales: core 5, application 55, runtime 40, presentation state 31 y app
providers_flow 6 PASS; examples/check y Clippy focal all-targets PASS. Esto no
equivale a verify/QA, CI completa ni PR-ready.

- [x] **5.1 Reservar barra antes de renderizar contenido** —
  `P/src/{status_bar,overlay,terminal,state}.rs`, unitarias de status_bar/terminal,
  `P/tests/state.rs`. Depende de 4.4. BAR-01/03, EVT-01.
  Snapshot tipado seguro: actividad/uso/conexión/aviso/diagnóstico; no servicios
  ni callbacks bajo render-lock. Reservar primero fila inferior para todos los
  overlays y composer; panel largo no conversacional, resumen visible con Approval,
  widgets lexicales en estado/detalle. Probar actividad, error y ausencia explícita;
  selección «siguiente turno» nunca implica Connected ni descubrimiento remoto.
  Aceptación/verificación: `L(P)`, `T(P; state)`; renderers plugins fuera del lock.
  Riesgo: early return oculta barra o error UI se degrada a log invisible.
  Evidencia: `status_bar::tests` cubre actividad, diagnóstico/feedback, ausencia/no
  false Connected y clipping Unicode; PTY `overlays_keep_status_visible_at_minimum_inline_size`
  cubre overlays estrechos. Suite status_bar 5/5 y PTY serial 37/37.

- [x] **5.2 Asegurar clipping, resize y restauración PTY** —
  `P/src/{terminal,status_bar,overlay}.rs`, `P/tests/{pty,input_parser}.rs`,
  `P/tests/support/{fixture,pty}.rs`. Depende de 5.1. BAR-01, OVL-03/04,
  SET-02/05/13. Mantener Viewport::Inline real acotado (20x8: siete filas inline,
  una de barra), sin alternate screen; solo conversation va a insert_before.
  Probar cada Overlay y sin overlay en ancho normal/20x8, Unicode por celdas,
  clipping/zero-size y resize bajo mínimo: sin panic ni aceptación positiva hasta
  recuperar espacio/redibujar/armar. Mantener campo/controles, challenge y preview
  completo desplazable PgUp/PgDn; feedback por OverlayId+epoch.
  Aceptación/verificación: `L(P)`, parser y PTY; inspeccionar termios/bracketed paste
  tras Ctrl+C/drop/shutdown. Riesgo: snapshot de buffer artificial no prueba viewport real.
  Evidencia: matrix PTY ambos tamaños; Approval 20x8; TestBackend 0x0 no-arm + viable
  draw/identity; resize 1x1 → 8x20 con código válido rechazado durante tamaño inviable
  y aceptado tras redraw; preview desplazable; composer cancel/salida comprueba ICANON,
  ECHO, bracketed-paste disable y no alternate screen. Drop PTY comprueba cierre
  síncrono y `is_raw_mode_enabled()==false`; el assert de termios explícito está en la
  prueba de cancel/salida. Suites focales: PTY serial 37/37, status_bar 5/5, state 32/32.

## Fase 6 — Retirada y gates de la entrega

- [x] **6.1 Retirar adaptadores y slots legacy** — `P/src/{lib,state,terminal,workflow,settings}.rs`,
  `P/tests/{state,workflows,extensions,providers_ui,completion}.rs`,
  `P/tests/support/fixture.rs`, `H/tests/providers_flow.rs` y `docs/huginn/README.md`.
  Depende de 5.2. OVL-01, EVT-01/02. Borrar slots UiState/View modal/plugin_dialog/
  workflow/settings, workflow_focus/settings_focus_epoch, committed, notice genérico,
  status/status_error soberanos; actualizar comentario de FIFO no autoritativa.
  Conservar nombres legítimos de módulos/DTOs/CommandResult::Notice; ninguna búsqueda
  global ciega justifica borrarlos. Revisar con `rg -n` scoped a P/src y H/src:
  `\bnotice\s*\(`, `\bcommitted\b`, `workflow_focus|settings_focus_epoch|status_error`
  y referencias `(state|view)\.(modal|plugin_dialog|workflow|settings)\b`;
  inspeccionar definición completa de UiState/View y justificar cada excepción.
  Aceptación/verificación: cero slots paralelos y cero UI en conversation;
  `T(P; state,workflows,extensions,providers_ui,completion)`, `T(H; providers_flow)`.
  Riesgo: legacy oculto por alias; inspección manual además de búsqueda negativa.
  Evidencia 2026-10-20: inspección de `UiState` y `View` muestra solo
  `overlays`/`overlay_identity`, sin slots paralelos. Métodos `modal()`,
  `workflow()`, `plugin_dialog()` y `settings()` son proyecciones del payload
  único, no almacenamiento soberano. `notice()` en providers.rs retorna
  `CommandResult::Notice` legítimo. No hay `committed`, `workflow_focus`,
  `settings_focus_epoch` ni `status_error`; comentario FIFO actualizado como
  proyección no autoritativa. Suites state 32, workflows 13, extensions 2,
  providers_ui 7, completion 9 y huginn/providers_flow 6: PASS, log
  `/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/huginn-phase6-1-focused.log`.

- [x] **6.2 Ejecutar QA dirigido con evidencia propia** — usar y, si falta un caso,
  ampliar `P/tests/{pty,providers_ui,state}.rs`, `P/tests/support/fixture.rs` y
  `H/tests/{tui,providers_flow}.rs` dentro de la unidad que entrega esa cobertura.
  Depende de 6.1. Recorrer matriz siguiente durante streaming, todos los overlays,
  guardados bloqueados, resize y reconexión; repetir comandos UI tras >2s mediante
  clock fake en integración. PTY verifica ausencia de UI en scrollback, no solo
  substrings. Centinela secreto ausente de pantalla/capturas, scrollback, JSON/logs;
  paste partido, challenge fresco, revocar/re-registrar y cierre restaurado.
  Aceptación/verificación: PTY y `T(H; tui,providers_flow)` con evidencias saneadas
  para futuro qa-report propio. Riesgo: backend real inaccesible; registrar caso
  no probado, no inventar éxito ni usar archive Settings-modal como evidencia.
  Evidencia 2026-10-20: aceptación técnica dirigida de Apply, no fase QA independiente.
  Nueva PTY con streaming retenido por cancelación, seis overlays ordinarios y
  centinela secreto comprueba scrollback real reconstruido con control positivo
  conversacional; no compara únicamente substrings del transcript. RED de harness:
  LF inferior sobrescribía en vez de desplazar; GREEN conserva filas expulsadas.
  RED productivo: Help no abría durante streaming por guard `!busy`; GREEN permite
  comandos UI y mantiene bloqueado el envío de otro prompt. Integración con reloj
  fake repite Help/Providers/Login/Model/Settings/Logout tras 2001 ms sin alterar
  conversación, tail ni cancelación. Matriz previa cubre Approval, resize, paste
  partido, revocación/re-registro y restauración; state cubre writes con barreras
  y reconexión. Presentation completa: 155 PASS (39 PTY, 10 parser, 32 state,
  13 workflows, 8 providers_ui y 53 restantes). App tui 8 + providers_flow 6 PASS.
  Logs completos y relación REQ→prueba en `apply-validation.md`; sin proveedor
  externo ni almacén seguro real, sin evidencia prestada de Settings-modal.

- [x] **6.3 Consolidar validación y entregar reporte sin publicar** — comprobar
  todos los runners de las unidades y `just ci-local` completo con requisitos de
  `docs/development.md` y `dev/README.md`: tooling/dependencias, Docker disponible,
  browsers e imagen `just test-e2e-build`. No copiar el manual ni resetear datos.
  Depende de 6.2. Revisar diff/estadística real, consumidores exhaustivos Event,
  CommandResult y ProviderAction, incluidos ejemplos/fixtures/wrappers, sin wildcards
  que oculten variantes nuevas. `just ci-ci` omite E2E: no sustituye ci-local.
  Aceptación: reporte al orquestador con tarea/REQ/escenario→prueba/comando/evidencia,
  RED/GREEN, gates ejecutados/fallidos/no ejecutados, riesgos y tamaño real;
  distinguir COMPLIANT/FAILING/UNTESTED y conservar bloqueos para verify/qa propios.
  Riesgo: gate parcial presentado como CI completa. No commits vacíos de validación,
  publicación ni archive; faltan verify-report y qa-report hasta sus fases.
  Evidencia 2026-10-20: siete paquetes Huginn completos PASS con all-features;
  Clippy adicional all-features/all-targets PASS; ejemplo providers_headless PASS.
  CI final sin alterar runner/paralelismo: `just ci-local` exit 0, 9/9, 431 s,
  Rust PASS (39 PTY incluidos), Vitest 24 archivos/210 tests y E2E 90/90 en tres
  browsers. `cargo audit --no-fetch` directo exit 0 sin máscara. Logs completos
  `opencode/huginn-apply6/ci-local-verified.log` bajo el temp aprobado; relación
  REQ→test, fallos anteriores, `just doctor` inexistente, dos ignores opt-in,
  límites de entorno y tamaño real en `apply-validation.md`. Sin Verify/QA,
  publicación, archive, reset de datos, modificación de state ni del index.

## Matriz completa de cobertura y barreras

Los identificadores SET remiten a REQ de ui-settings; su delta sustituye solo
07/08/12. Cada asociación exige evidencia en verify, no es un veredicto anticipado.

- **EVT-01 A/B** → 1.1, 1.3, 2.2, 2.3, 2.6, 3.1, 5.1, 6.1/2: mismo texto por categoría,
  aperturas/cierres repetidos tras 2s sin prompt ni historial UI; diagnóstico visible.
- **EVT-02 A/B** → 1.1/3, 4.4: repetición legítima, streaming y tool error íntegros,
  FIFO 256/257 sin afectar Session.events, persistencia/replay ni reabrir sesión.
- **EVT-03 A/B/C** → 1.2/3: fake t=0/1,999/2s, A/B intercalados, colisión inyectada,
  fallback compartido; ninguna supresión prolonga ventana ni cambia de superficie.
- **EVT-04 A** → 1.2/3: límites de entradas/bytes/4096, L+1, expiración y finish;
  saturación visible, sin truncar comparación ni almacenar secretos.
- **OVL-01 A** → 2.2, 2.3, 3.1, 6.1: Help/foco/cierre consumido, autoridad única final.
- **OVL-02 A/B** → 2.1, 2.2, 2.3: allowlist/metadatos y coincidencia exhaustiva,
  Confirm sin autoridad, rechazo conserva overlay. Pruebas contract/workflows y smoke.
- **OVL-03 A/B/C** → 2.2, 2.4, 3.2, 5.2: sender único, input fresco,
  cancelación/Drop/canal/shutdown y respuesta anterior; ordinarias bloqueadas y nunca restauradas.
- **OVL-04 A/B** → 2.2, 2.3–2.5, 3.2, 5.2/6.2: paste partido,
  reentrancia incluso Drop de Pending, lease/epoch, revocación/re-registro, tarea tardía,
  centinela y PTY.
- **OVL-05 A/B** → 2.2, 2.3, 2.6, 4.2/4: callbacks/payloads/secretos conservados
  en cada continuación; login/providers/model/logout reales; Secure sin fallback,
  Ephemeral explícito, validación/OAuth y fijación del turno/revocación.
- **BAR-01 A** → 5.1/2, 6.2: todos los overlays incluida Approval, viewport inline,
  Unicode, 20x8, resize/zero-size, clipping y scrollback solo conversacional.
- **BAR-02 A/B/C** → 4.1–4: barreras snapshot/subscribe y A/B tardío; stage/confirm
  exactos en todas las rutas; faults antes/después de publicar y Pending/drop/cancel;
  adapter/registration/manager/core teardown, owner/source/revision y reconexión.
- **BAR-03 A/B** → 4.1–4, 5.1, 6.2: selección local/Legacy/Unmanaged sin Connected,
  ningún éxito anterior promete salud; ausencia/error/aprobación siempre observables.
- **SET-07 A/B/C/D** → 3.2/3: closure reentrante, write barrier, Enter único,
  disk full preserva copia, reintento y receipt tardío con otro Settings abierto;
  snapshot guardado solo en éxito, TOML conjunto sin live theme.
- **SET-08 A/B** → 3.1/2: Esc durante streaming antes/después de iniciar write,
  bytes intactos solo antes de Apply, un write y receipt sin rollback ni reapertura.
- **SET-12 A/B/C/D** → 2.2, 2.4, 3.1/2: reemplazo válido/rechazado, Approval antes/
  durante Apply y receipt éxito/error, lease obsoleto no cruza al sustituto.
- **SET-01** → 3.1 y completion/plugins: registro, descripción, whitespace, editor
  limpio y cero prompts; **SET-02** → 3.1/5.2: header/body/footer y markers.
- **SET-03/04/05** → 3.1/5.2: tabs/navegación/selección, filas concretas y pickers; **SET-06** → 3.1: copia independiente y movimientos sin I/O.
- **SET-09** → 3.3: round-trip, bytes externos/comments/orden; **SET-10** → 3.3:
  defaults sin ui, parcial y completo; **SET-11** → 3.3: unknown internos preservados.
- **SET-13** → 3.1/2, 5.2: Esc no cancela turno; **SET-14** → 3.1: Add provider sin
  mutación/providers/persistencia; **SET-15** → 3.3: line-mode excluido; **SET-16** → 3.1: catálogo chat con/sin modelo.

## Bloqueos y salida requerida

No hay gaps de producto bloqueantes identificados para planificar. No se presupone
tooling instalado, CI verde ni evidencia heredada: fallos conductuales, carreras,
filtraciones, ausencia de coverage de una ruta de publicación o teardown y QA
incompleto son bloqueos reales de aceptación que deben comunicarse, no silenciarse.
La excepción de tamaño ya está autorizada; no se añade otra pregunta de entrega.

Próximo paso del orquestador: revisar evidencia de Apply y delegar Verify, después
QA; actualizar state corresponde solo al orquestador. Los informes propios deberán documentar todos
los casos no probados y bloqueos antes de archive. Settings-modal archived sin
verify/qa no acredita ninguno de esos gates.
