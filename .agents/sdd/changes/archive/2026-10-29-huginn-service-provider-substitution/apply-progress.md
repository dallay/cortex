# Progreso de implementación: `huginn-service-provider-substitution`

**Actualizado:** 2026-10-29<br>
**Estado:** Implementación completa hasta 5.12; Verify y QA PASS WITH WARNINGS; archive pendiente. No se afirma aceptación del operador.<br>
**Metadata de entrega:** El usuario autorizó continuar en este worktree sin metadata de layer. No se infirieron rama, base, parent ni topología. No se hizo commit, push ni publicación.

## Tareas completadas

- [x] Fase 1, tareas 1.1–1.8: leases por generación, introspección, hooks `drain`/`resume` y validación del kernel.
- [x] Fase 2, tareas 2.1–2.13: sustitución staged, rollback, diagnósticos, validación de servicios, dependientes multi-servicio y políticas de drenaje.
- [x] Fase 3, tareas 3.1–3.9: slots dinámicos, publicación sincronizada, rollback/timeout y validación completa de `huginn-core`.
- [x] Fase 3, tareas 3.10–3.12: seam de `PluginContext::service_slot_handle`, autorización por `Manifest.requires`, rebind con el mismo handle y validación de errores.
- [x] Fase 4, tarea 4.1: auditoría previa de `LoopPlugin::activate` y `main.rs`, que motivó las tareas 4.2–4.10.
- [x] Fase 4, tareas 4.2–4.10: `LoopExecution<'a>` compartido, adapter dinámico productivo, admission drain/resume, leases por operación y suite application.
- [x] Fase 5, tareas 5.1–5.6: readiness dinámica en CLI/TUI/doctor, tests runtime indicados, documentación y gates focalizados.
- [x] Seguimiento de verify, tareas 5.8–5.10: cobertura de run activo con sustitución exitosa de model, rechazo de leases tras cierre de gates, parejas de leases multi-slot y contrato de `resolve() -> Arc<T>`.
- [x] QA-01, tarea 5.12: cobertura in-process de los dos builders runtime con `SessionsService` sustituido.

El registro previo asignaba la auditoría y documentación a números de tareas de una descomposición anterior. La versión actual de `tasks.md` es la fuente de verdad: el handoff 5.11 está completo y Verify/QA se repitieron tras 5.12. Archive sigue pendiente. No se modificaron los campos, firmas ni struct literals de `StandardLoop`, ni `LoopPlugin(pub LoopConfig)`.

## Evidencia RED → GREEN → REFACTOR del batch 3.1–3.9

1. **RED, slots:** se añadieron tests para el proveedor vigente, la lease anterior, la carrera `acquire()`/commit y el timeout. Antes de implementar, `cargo test -p huginn-core --test service_lease service_slot` falló con `E0599`: `Supervisor::service_slot_handle` no existía.
2. **RED, limpieza precommit:** `cargo test -p huginn-core --lib staged_abort_tests` falló en los dos escenarios esperados. El contador de recursos staged permaneció en `0` cuando faltaba un gate después del staging y cuando el cierre fallaba por un mutex envenenado; los tests requerían `1`.
3. **GREEN:** se implementó `ServiceSlotHandle<T>` sobre la tabla global sincronizada. La adquisición lee valor y gate, valida el tipo y aumenta el contador antes de liberar el mutex. `ServiceHandle<T>` conserva su semántica ligada a una generación y `resolve() -> Arc<T>` no cambió.
4. **GREEN de commit y rollback:** el commit reemplaza todos los slots staged bajo una sola sección crítica. La admisión dinámica y el incremento de leases comparten la sincronización de tabla. Los abortos reabren gates, descartan el plugin staged, registran el resultado y reanudan los dependientes intentados. La limpieza incluye gates ausentes y fallas de cierre posteriores al staging.
5. **REFACTOR:** se centralizó el aborto precommit, se mantuvo el mutex fuera de cualquier `await`, y `LeaseGuard::drop` libera el contador antes de notificar. El kernel no incorpora conocimiento de `ModelService` ni `SessionStore`.

La primera ejecución del test de slot tuvo una expectativa inválida: intentó completar el commit mientras mantenía la lease anterior, aunque la sustitución debe esperar a que esa lease termine. Se ajustó el test para comprobar que la lease permanece activa durante la espera, liberarla y comprobar luego el provider nuevo.

## Evidencia TDD de 3.10–3.12: seam de `PluginContext`

1. **RED, acceso desde `activate`:** se añadió primero un dependent que obtiene y retiene `ServiceSlotHandle<u64>` durante `activate`, conserva la misma instancia después de `substitute` y comprueba que adquiere el provider de reemplazo. `cargo test -p huginn-core --test service_lease dependent_retains_context_slot_handle_across_provider_substitution` falló antes de implementar con `E0599`: `PluginContext::service_slot_handle` no existía.
2. **RED, límites de acceso:** se añadieron tests para solicitar un slot existente fuera de `Manifest.requires` y para solicitar un tipo incompatible. Ambos compilaron inicialmente con el mismo fallo `E0599` antes de implementar el seam.
3. **GREEN:** `PluginContext::service_slot_handle<T>(&ServiceId)` comprueba primero que el ID esté declarado, luego consulta la tabla global compartida y valida existencia y tipo antes de crear el handle. Tanto `activate_plugin` como `stage_plugin` pasan el manifiesto/requisitos y el mismo `Arc<Mutex<ServiceSlots>>` del supervisor.
4. **Slot ausente:** el test principal elimina el provider de reemplazo después de verificar el rebind y confirma que el handle retenido ya no puede adquirirlo. No se fabricó un contexto de activación inválido: el kernel solo activa dependientes cuando su requisito tiene provider publicado.
5. **REFACTOR:** un constructor privado de `PluginContext` centraliza el filtrado de servicios, el conjunto de IDs requeridos y el cableado de la tabla compartida para ambos caminos. El constructor no altera `resolve() -> Arc<T>`, `ServiceHandle<T>` ni `huginn-plugin-api::v1`.
6. La primera ejecución de las pruebas de rechazo detectó que el texto de la aserción esperaba `not declared`, mientras el error implementado decía `undeclared service slot`; se ajustó la aserción al contrato y se repitieron correctamente las pruebas.

El spec activo está en `.agents/sdd/changes/huginn-service-provider-substitution/specs/huginn-kernel/spec.md`; los criterios de este batch también están trazados en `tasks.md` y `design.md`.

## Seguimiento TDD de 3.8–3.9: publicación y overflow

1. **RED, callback de commit:** `cargo test -p huginn-core --lib failed_slot_publication_does_not_run_commit_callbacks` reprodujo el fallo. Con `service_slots` envenenado, el callback corrió antes de detectar el fallo (contador `1`, esperado `0`); el test también exige cleanup del recurso staged.
2. **RED, contador:** `cargo test -p huginn-core --lib lease_counter_overflow_returns_an_error_without_poisoning_the_gate` reprodujo un panic de overflow en `active += 1`.
3. **GREEN:** `publish_staged_plugin` hace una adquisición de preflight de la tabla antes de ejecutar callbacks. Suelta el mutex antes de invocarlos para evitar deadlocks por reentrada y no extiende la sección crítica. Después vuelve a adquirirlo para publicar.
4. **Razón de sincronización:** entre el preflight y la publicación, una adquisición ordinaria no ejecuta código de plugin bajo el mutex de tabla; la única operación de contador usa `checked_add`. Por tanto, la concurrencia normal de `acquire()` no puede envenenar esa mutex en ese intervalo. Si la tabla ya está envenenada, el preflight falla antes de los callbacks y deja el contexto staged disponible para cleanup. No se afirma que cualquier panic excepcional sea recuperable.
5. **REFACTOR:** el overflow devuelve `AgentError` sin modificar el contador ni envenenar el gate ni la mutex global de slots. El test usa el camino real `ServiceSlotHandle::acquire` y verifica que ambas mutexes siguen sanas. Se ejecutaron los tests focalizados y luego la suite completa del paquete.

## Evidencia TDD de 4.2–4.10: rebind y drain de aplicación

1. **RED, rebind:** `cargo test -p huginn-application --test service_slot_rebinding retained_loop_service_uses_replaced_model_and_store_for_run_and_compact` falló contra la composición estática con `Provider(Revoked)` después de sustituir model y sessions.
2. **RED, admission:** el test `active_operation_saves_only_to_old_store_and_next_run_uses_new_store` falló en un negative control que dejó `LoopAdmission` aceptando durante el drain. El intento entre commit y `resume` se admitió; el test requería rechazo pre-efectos.
3. **RED, segundo slot:** `second_slot_failure_has_no_effects_and_releases_first_lease` falló en un negative control que publicó `LoopPlugin(config)` en vez del adapter: el run no detectó el tipo incompatible del slot de sessions y produjo efectos. Los negative controls se retiraron antes de ejecutar la suite GREEN.
4. **GREEN:** los cuatro tests de `crates/huginn/application/tests/service_slot_rebinding.rs` pasan con la composición productiva. Comprueban rebind de un `Arc<LoopService>` retenido para `run` y `compact`; timeout sin cancelar un run; rechazo durante drain; reanudación tras abort/commit; fallo de la segunda lease sin efectos; y varias llamadas `save` en un solo store por operación.
5. **REFactor:** `LoopExecution<'a>` comparte la lógica entre `StandardLoop` y `LeasedAgentLoop`. `StandardLoop` mantiene los seis campos públicos, sus firmas y struct literals; `LoopPlugin(pub LoopConfig)` no cambió. `composition::build` publica el adapter privado con slots, no un `StandardLoop` estático.
6. El adapter productivo conserva referencias prestadas a `ModelService` y `SessionsService` dentro de `ServiceLease`s hasta completar `run`/`compact`; no clona ni almacena sus `Arc`s fuera de esas leases. La admisión y ambas leases se obtienen antes de invocar `LoopExecution`, que puede mutar session, emitir eventos, cargar skills o llamar model/store.

## Evidencia TDD de 5.1–5.6: readiness y límites runtime

1. **RED 5.1:** se añadió un test unitario en `apps/huginn/src/main.rs`. `cargo test -p huginn --bin huginn retained_readiness_slot_tracks_replacement_and_fails_closed` falló primero con `E0425` porque `current_model_ready` no existía.
2. **GREEN:** el helper adquiere `ServiceSlotHandle<ModelService>` en cada lectura y devuelve `false` si falla `acquire()`. El mismo handle informa provider A no configurado, provider B configurado después de `substitute` y `false` tras retirar el provider.
3. **REFACTOR/cableado:** doctor, readiness inicial y cada prompt CLI, callback Ratatui y cada prompt TUI leen el slot actual. Se eliminó la captura de `Arc<ModelProvider>`/bool estáticos para readiness. La app suelta el `Arc` inicial de model después de construir la composición.
4. **Regresión CLI/TUI:** los subprocess tests existentes se mantuvieron y pasaron dentro de `cargo test -p huginn`. Solo validan el comportamiento normal del proceso; no se presentan como evidencia de sustitución in-process.
5. **Persistencia/runtime:** se ejecutaron los dos targets pedidos. `SqliteSessions::lease/load/list` permanecen directos a `SqliteSessions`, fuera de `SessionsService`; no se añadió transferencia de locks, migración ni cambio de esquema.
6. **RED, builder runtime:** `cargo test -p huginn-runtime --test tool_lifecycle runtime_composition_build_retains_a_dynamically_rebound_loop_service` falló porque `build_native` seguía registrando el plugin estático y el store sustituido no recibió los `save`.
7. **GREEN del builder:** `build_native_plugin` ahora usa la factory `leased_loop_plugin`; la estructura `LeasedLoopPlugin` sigue privada y la ruta real de runtime pasa a publicar `LeasedAgentLoop`. El test in-process pasó.
8. **Docs:** `architecture.md` y ADR-0003 documentan tabla/leases, builders de aplicación y runtime que publican `LeasedAgentLoop`, compatibilidad legacy, drain/rollback, readiness actual y límites cooperativos.

La factory pública `huginn_application::composition::leased_loop_plugin` es un ajuste aditivo para que el crate runtime instancie el plugin sin exponer `LeasedLoopPlugin`. Esta es una desviación menor respecto a la lista de paths del diseño; conserva privado el tipo y mantiene intacto `LoopPlugin(pub LoopConfig)`.

## Seguimiento de cobertura 5.8–5.10

Este batch fue exclusivamente de cobertura. Escribí las pruebas antes de cambiar producción y no necesité un fix: los tests focalizados pasaron en su primera ejecución. No hubo una falla RED en este seguimiento y no se modificó código de producción.

- **5.8:** `successful_model_substitution_waits_for_active_run_then_rebinds_same_loop` bloquea el model A, espera el drain, comprueba que la run sigue activa y la sustitución está pendiente, libera A y comprueba que la misma instancia `Arc<LoopService>` usa B en la siguiente run.
- **5.9:** `multi_service_slots_reject_new_leases_while_old_generation_drains` mantiene una lease antigua, espera una notificación emitida después del cierre de gates, verifica rechazo para ambos handles, libera la lease y comprueba los valores nuevos para ambos servicios.
- **5.10, parejas:** `concurrent_multi_slot_lease_pairs_never_report_mixed_values` mantiene ambas leases mientras valida cada pareja adquirida durante la competencia. Cada pareja exitosa observada corresponde al conjunto antiguo `(1, 2)` o al nuevo `(10, 20)`.
- **5.10, `resolve`:** `resolved_arc_remains_on_old_value_after_provider_substitution` demuestra que el `Arc<u64>` retenido conserva `1`, mientras un `resolve()` posterior devuelve `2`.
- **Límite de observabilidad:** la API no ofrece snapshot/acquire multi-slot. Las pruebas validan parejas de leases que permanecen simultáneamente vivas y los valores anterior/nuevo; no observan una tupla atómica de la tabla durante una sección crítica ni afirman esa garantía más fuerte.
- `verify-report.md` no se modificó. El test completo combinado se repitió; 5.11 (Verify, QA y archive) sigue pendiente.

## Cobertura QA-01 (5.12)

Este sub-batch solo añadió tests en `crates/huginn/runtime/tests/tool_lifecycle.rs`; no cambió código de producción. Las dos pruebas retienen `Arc<LoopService>`, sustituyen `SessionsService` y comprueban que la siguiente run guarda el mismo ID de session en `CaptureSessions` B, no en A.

- `build_native_with_connections` usa `MockProvider`, `CaptureSessions`, `ConnectionAccess::new(Arc::new(HostSecrets::default()))` y no inicia una operación de conexión. `ConnectionAccess::new` solo crea estado en memoria; ningún método de `HostSecrets` se llama.
- `build_native_with_connections_and_guards` usa los mismos fakes y `Arc::new(Guardrails::default())`. El guardrail tiene listas vacías y el prompt no solicita herramientas.
- La primera ejecución del primer test usó un workspace inexistente y falló con `NotFound` en `FilesystemInstructions`. Se corrigió el fixture a un `tempdir`; no se observó un fallo de rebind ni se cambió producción.
- Ambos comandos de builder pasaron por separado; después pasó el target completo `tool_lifecycle` con 7 tests. No hubo RED de implementación: la cobertura pasó tras corregir el fixture.
- `verify-report.md`, `qa-report.md` y `state.yaml` no se modificaron. No se repitió la suite combinada de cuatro paquetes en este sub-batch; esa suite pasó en el batch anterior antes de agregar estos dos tests. Verify, QA y archive siguen pendientes.

## Verificación

| Comando | Resultado |
|---|---|
| `cargo test -p huginn-core --test service_lease service_slot` | Pasó; 2 tests focalizados de slots. |
| `cargo test -p huginn-core --lib staged_abort_tests` | Pasó; 2 tests de cleanup staged ante gate ausente y cierre fallido. |
| `cargo test -p huginn-core --test service_lease` | Pasó; 17 tests de sustitución, leases, slots, drenaje y rollback. |
| `cargo test -p huginn-core --lib failed_slot_publication_does_not_run_commit_callbacks` | RED reprodujo callback=1; después de la corrección pasó con callback=0 y cleanup staged. |
| `cargo test -p huginn-core --lib lease_counter_overflow_returns_an_error_without_poisoning_the_gate` | RED reprodujo panic de overflow; después de la corrección pasó sin envenenar el gate. |
| `cargo test -p huginn-core --lib staged_abort_tests` | Pasó; 4 tests focalizados de abort y publicación. |
| `cargo test -p huginn-core --test service_lease dependent_retains_context_slot_handle_across_provider_substitution` | RED: falló antes de implementar con `E0599`; GREEN y REFACTOR: pasó con el handle retenido obteniendo el provider sustituido. |
| `cargo test -p huginn-core --test service_lease context_rejects_slot_handle` | Pasó; rechaza ID no declarado y tipo incompatible. |
| `cargo test -p huginn-core --test service_lease` | Pasó; 23 tests de lease, sustitución, contexto y follow-up 5.9–5.10. |
| `cargo test -p huginn-core` | Pasó; suites completas del paquete y doctest. |
| `cargo test -p huginn-application --test service_slot_rebinding` | Pasó; 5 tests de rebind, drain/timeout, segunda lease, multi-save y run activo durante sustitución exitosa de model. |
| `cargo test -p huginn-application --test service_slot_rebinding successful_model_substitution_waits_for_active_run_then_rebinds_same_loop` | Pasó en primera ejecución; A termina la run activa y la misma `Arc<LoopService>` usa B después del commit. |
| `cargo test -p huginn-core --test service_lease multi_service_slots_reject_new_leases_while_old_generation_drains` | Pasó en primera ejecución; ambos handles fallan cerrados mientras el provider multi-servicio espera una lease antigua. |
| `cargo test -p huginn-core --test service_lease concurrent_multi_slot_lease_pairs_never_report_mixed_values` | Pasó en primera ejecución; valida los valores de parejas con ambas leases vivas, con límite de snapshot documentado. |
| `cargo test -p huginn-core --test service_lease resolved_arc_remains_on_old_value_after_provider_substitution` | Pasó en primera ejecución; Arc retenido conserva valor antiguo y `resolve()` nuevo devuelve el alternativo. |
| `cargo test -p huginn-application -p huginn-runtime --no-run` | Pasó; compilaron las suites y struct literals actuales de `StandardLoop`. |
| `cargo test -p huginn-application` | Pasó sin filtro; suites unitarias, integración y doctests del paquete. |
| `cargo test -p huginn --bin huginn retained_readiness_slot_tracks_replacement_and_fails_closed` | RED: `E0425` antes del helper; GREEN: pasó leyendo provider A, luego B desde el mismo handle y fallando cerrado tras retirar el provider. |
| `cargo test -p huginn-runtime --test tool_lifecycle runtime_composition_build_retains_a_dynamically_rebound_loop_service` | RED: falló con el builder estático; GREEN: pasó después de registrar el adapter leased desde `build_native_plugin`. |
| `cargo test -p huginn-runtime --test tool_lifecycle connections_builder_rebinds_retained_loop_to_new_sessions_store` | Pasó por separado; `build_native_with_connections` guarda en store B desde el `LoopService` retenido. |
| `cargo test -p huginn-runtime --test tool_lifecycle connections_and_guards_builder_rebinds_retained_loop_to_new_sessions_store` | Pasó por separado; `build_native_with_connections_and_guards` guarda en store B desde el `LoopService` retenido. |
| `cargo test -p huginn-runtime --test tool_lifecycle` | Pasó; 7 tests, incluyendo los dos builders QA-01. |
| `cargo clippy -p huginn-runtime --all-targets -- -D warnings` | Pasó. |
| `cargo test -p huginn-runtime --test session_locks --test sessions_persistence` | Pasó; 6 tests de locks y 2 de persistencia. |
| `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn` | Pasó; suites completas de esos cuatro paquetes y doctests. Dos tests fueron `ignored` por dependencias externas documentadas. |
| `cargo fmt --all -- --check` | Pasó. |
| `cargo clippy -p huginn-core --all-targets -- -D warnings` | Pasó. Una ejecución inicial detectó tres advertencias; se refactorizó y la ejecución final pasó. |
| `cargo clippy -p huginn-application -p huginn-runtime --all-targets -- -D warnings` | Pasó. |
| `cargo clippy -p huginn-core -p huginn-application -p huginn-runtime -p huginn --all-targets -- -D warnings` | Pasó. |
| `just huginn-architecture` | Pasó; límites de arquitectura y negative control. |
| `pnpm exec markdownlint-cli2 docs/huginn/architecture.md docs/huginn/adr/0003-minimal-service-composition-kernel.md .agents/sdd/changes/huginn-service-provider-substitution/tasks.md .agents/sdd/changes/huginn-service-provider-substitution/apply-progress.md` | Pasó; 0 issues. |
| `git diff --check` | Pasó. |

Los subprocess tests `cli` y `tui` pasaron dentro de `cargo test -p huginn`, como regresión normal; no prueban sustitución in-process. No se ejecutó `just ci-local` ni CI completa. Verify y QA dieron PASS WITH WARNINGS; archive sigue pendiente y no se afirma aceptación del operador. No se modificó `huginn-plugin-api::v1` ni se repitió `cargo test -p huginn-core` por separado después del seguimiento; el paquete sí se incluyó en la suite conjunta.

## Límites y riesgos

- El test concurrente ejercita adquisiciones de un slot mientras el commit ocurre y comprueba que cada lease obtiene un valor válido; también comprueba todos los handles afectados después del commit. La API no ofrece un snapshot/adquisición multi-slot, por lo que no se afirma que la atomicidad global de varios slots se haya observado concurrentemente. La atomicidad se implementa manteniendo el mutex global durante todas las actualizaciones del commit.
- El contrato de lease sigue siendo cooperativo. `resolve() -> Arc<T>` y referencias clonadas que sobreviven fuera de una lease no obtienen revocación fuerte.
- El timeout no cancela una lease activa. Conserva el provider anterior, reabre su admisión y permite reintentar después de liberar la lease.
- Los builders productivos de aplicación y runtime registran el adapter privado `LeasedAgentLoop`; el uso directo de `LoopPlugin(pub LoopConfig)` sigue siendo legacy/estático y no obtiene rebind.
- Se añadió la factory pública aditiva `huginn_application::composition::leased_loop_plugin` para que el crate runtime registre el tipo concreto privado sin exponer `LeasedLoopPlugin`.
- `StandardLoop` conserva sus seis campos públicos, firmas y struct literals actuales. La garantía de rebind del loop productivo aplica a `LeasedAgentLoop` y solo cubre las operaciones que mantienen las leases.
- La readiness consulta la generación vigente del slot en cada llamada; los procesos CLI/TUI independientes no demuestran una sustitución dentro del mismo proceso.
- No se migran sesiones al cambiar `SessionsService`; `SqliteSessions::lease/load/list` quedan fuera de ese servicio.
- Verify y QA permiten archive con advertencias; la aceptación del operador no se afirma.
- Antes de preparar una PR, sigue pendiente resolver metadata de layer/base para la estrategia `feature-branch-chain`; no se infirió ninguna topología aquí.

## Archivos modificados por este batch

- `crates/huginn/core/src/kernel.rs`: tabla global de slots, `ServiceSlotHandle<T>`, commit sincronizado, cleanup centralizado y tests unitarios de abort precommit.
- `crates/huginn/core/src/lib.rs`: reexport aditivo de `ServiceHandle`, `ServiceLease` y `ServiceSlotHandle`.
- `crates/huginn/core/tests/service_lease.rs`: tests de slot vigente, concurrencia de adquisición/commit, multi-slot tras commit, timeout sin cancelar lease y seam de contexto con rebind/autorización/tipo.
- En el seguimiento: añade gate-close multi-servicio, parejas concurrentes de leases y regresión de `resolve() -> Arc<T>` retenido.
- `crates/huginn/application/src/loop_engine.rs`: `LoopExecution<'a>` compartido; delegación desde el `StandardLoop` estático y ejecución por referencias prestadas.
- `crates/huginn/application/src/composition.rs`: `LeasedAgentLoop`, `LeasedLoopPlugin`, admisión `drain`/`resume` y factory `leased_loop_plugin`.
- `crates/huginn/runtime/src/composition.rs`: builders nativos registran el adapter leased en vez de `LoopPlugin` estático.
- `crates/huginn/runtime/tests/tool_lifecycle.rs`: cobertura in-process de `build_native`, `build_native_with_connections` y `build_native_with_connections_and_guards` con `LoopService` retenido.
- `crates/huginn/application/tests/service_slot_rebinding.rs`: cinco tests; incluye model A activo durante sustitución exitosa y rebind de la misma `Arc<LoopService>` a B.
- `apps/huginn/src/main.rs`: `current_model_ready`, cableado a doctor, CLI, prompts y callback TUI; test unitario RED→GREEN de sustitución in-process.
- `docs/huginn/architecture.md` y `docs/huginn/adr/0003-minimal-service-composition-kernel.md`: slots, leases, loop productivo, readiness y límites de persistencia.
- `.agents/sdd/changes/huginn-service-provider-substitution/tasks.md`: checks y evidencia de las tareas 3.1–3.12, 4.1–4.10 y 5.1–5.12.

Los documentos ya tenían cambios preexistentes; este batch también los actualizó para reflejar la implementación productiva actual.

## Tareas restantes

- [ ] Ejecutar `sdd-archive` tras Verify/QA PASS WITH WARNINGS y sin blockers P0/P1.

**Progreso:** tareas 3.1–3.12, 4.1–4.10, 5.1–5.6 y 5.8–5.12 completadas. Verify y QA permiten archive con advertencias; no se afirma aceptación del operador.
