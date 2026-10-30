# Tareas: rebind de proveedores de servicios de Huginn

> **Cambio**: `huginn-service-provider-substitution` · **Issue**: #509 · **Epic**: DALLAY-718
> **Fase**: tasks · **Diseño**: rebind en vivo aprobado

## Forecast de carga de revisión

450–650 líneas estimadas; riesgo: High.

Decision needed before apply: No
Chained PRs recommended: Yes
Chain strategy: feature-branch-chain
400-line budget risk: High

Batch aprobado aquí; worktree sin branch metadata. Sin commit/push/publicación ni base/rama inferidas.

### Unidades sugeridas

| Unidad | Entrega              | Depende |
|--------|----------------------|---------|
| 1      | Slots `core`         | —       |
| 2      | Loop rebind/drain    | 1       |
| 3      | Readiness/docs/gates | 2       |

## Fases 1–2: Kernel y sustitución (completadas)

- [x] 1.1–1.8: leases/introspección; 2.1–2.13: staging/rollback/drain.

## Fase 3: Slots dinámicos y seam de contexto (completada)

- [x] 3.1–3.9: slots, commit multi-slot, rollback y gates verificados.
- [x] 3.10 RED: `crates/huginn/core/tests/service_lease.rs` — `activate` recibe solo `PluginContext`; handle requerido ve provider nuevo. Rechazar ID no declarado, slot ausente o tipo incompatible.
- [x] 3.11 GREEN: `crates/huginn/core/src/kernel.rs`, `crates/huginn/core/src/lib.rs` — limitar `PluginContext::service_slot_handle<T>(&ServiceId)` a `Manifest.requires`; cablear en `activate_plugin`/`stage_plugin`; validar ID/slot/tipo.
- [x] 3.12 REFACTOR: simplificar cableado/validación; ejecutar `cargo test -p huginn-core`.

## Fase 4: `StandardLoop` en vivo

Dependencia: 4.2–4.10 requieren 3.10–3.12. No cambiar API pública de `StandardLoop` ni `LoopPlugin`.

- [x] 4.1 Auditoría: `LoopPlugin::activate` y `main.rs` retenían `Arc`s; no probó rebind.
- [x] 4.2 RED: `crates/huginn/application/tests/service_slot_rebinding.rs` — `LoopService` retenido ve slots A→B en siguiente `run`/`compact`, sin reconstrucción.
- [x] 4.3 GREEN: `crates/huginn/application/src/loop_engine.rs` — compartir `LoopExecution<'a>` por operación; preservar seis campos/firmas y delegación estática de `StandardLoop`; `LeasedAgentLoop` privado usa slots, no lo envuelve.
- [x] 4.4 REFACTOR: compilar struct literals con `cargo test -p huginn-application -p huginn-runtime --no-run`: `providers_turns.rs`, `coding_workflow.rs`, `providers_integration.rs`, `stream_persistence.rs`.
- [x] 4.5 RED: mismo test — `drain` cierra admisión, espera activa y rechaza operaciones antes de efectos; timeout no cancela; abort reabre.
- [x] 4.6 GREEN: `crates/huginn/application/src/composition.rs` — `LeasedLoopPlugin` privado comparte `Arc<LoopAdmission>` con `LeasedAgentLoop`; hooks privados `drain`/`resume`; conservar `LoopPlugin(pub LoopConfig)`.
- [x] 4.7 REFACTOR: verificar drain, rechazo, abort/reanudación y deadlocks.
- [x] 4.8 RED: mismo test — leases cubren `run`/`compact`; falla segunda sin mutación/efectos; varios `save` comparten generación/store.
- [x] 4.9 GREEN: adquirir admisión/slots antes de efectos; leases model+session abarcan streams, herramientas, aprobaciones y saves; RAII libera primera ante fallo de segunda.
- [x] 4.10 REFACTOR: `cargo test -p huginn-application`; confirmar RED y diagnosticar `resume`.

## Fase 5: Runtime, readiness y verificación

- [x] 5.1 RED: `apps/huginn/src/main.rs` readiness unit test; `crates/huginn/runtime/tests/tool_lifecycle.rs` rebind vía `build_native`.
- [x] 5.2 GREEN: `main.rs` usa slots, no `Arc`/bool, para doctor/CLI/prompt/TUI. Los builders en `crates/huginn/runtime/src/composition.rs` comparten helper privado `build_native_plugin`→`leased_loop_plugin`; `huginn_runtime::composition::build` delega a application.
- [x] 5.3 REFACTOR: `cargo test -p huginn-runtime --test tool_lifecycle` y `cargo test -p huginn`; CLI/TUI con conexiones/guards son regresión, no in-process.

- [x] 5.4 `cargo test -p huginn-runtime --test session_locks --test sessions_persistence`; sin migración ni tests de `SqliteSessions::lease/load/list` fuera de `SessionsService`.
- [x] 5.5 Actualizar `docs/huginn/architecture.md` y `docs/huginn/adr/0003-minimal-service-composition-kernel.md` sobre slots, leases, drain/rollback y límites runtime.
- [x] 5.6 Ejecutar suites core/application/runtime/apps, `just huginn-architecture`, fmt y clippy `-D warnings`; registrar omisiones.
- [x] Handoff: implementación apply completada hasta 5.6; `verify`, QA y archive son fases posteriores y siguen pendientes.

## Remediación Verify (V-01–V-03)

Si pasa, registrar PASS como evidencia; si falla, documentar RED y aplicar RED→GREEN→REFACTOR al fix mínimo.

- [x] 5.8 `crates/huginn/application/tests/service_slot_rebinding.rs`: bloquear `run` en model A; `ModelService` sustituye y espera sin cancelar; liberar A; mismo `Arc<LoopService>` usa B en la siguiente operación.
- [x] 5.9 `crates/huginn/core/tests/service_lease.rs`: handles de todos los servicios afectados rechazan leases con gate cerrado.
- [x] 5.10 `crates/huginn/core/tests/service_lease.rs`: competir adquisiciones emparejadas de ambos handles; cuando ambas leases simultáneas se obtienen, validar un par consistente; `resolve() -> Arc<T>` retenido conserva el valor antiguo.
- [x] 5.11 Handoff tests: pasó `cargo test -p huginn-core -p huginn-application -p huginn-runtime -p huginn`; sigue `sdd-verify`. QA/archive bloqueados hasta PASS/PASS WITH WARNINGS en Verify y QA.
- [x] 5.12 QA-01: tests in-process en `crates/huginn/runtime/tests/tool_lifecycle.rs`; llamar por separado a `huginn_runtime::composition::build_native_with_connections` y `huginn_runtime::composition::build_native_with_connections_and_guards`, retener `Arc<LoopService>`, sustituir `SessionsService` y confirmar saves posteriores en store B. Coverage-only; sin cambio de producción.

### Evidencia de QA-01 (5.12)

- `cargo test -p huginn-runtime --test tool_lifecycle connections_builder_rebinds_retained_loop_to_new_sessions_store` pasó.
- `cargo test -p huginn-runtime --test tool_lifecycle connections_and_guards_builder_rebinds_retained_loop_to_new_sessions_store` pasó.
- `cargo test -p huginn-runtime --test tool_lifecycle` pasó con 7 tests.
- La primera ejecución encontró un fixture test-only inválido: el workspace no existía y `FilesystemInstructions` devolvió `NotFound`. Se cambió el fixture a un `tempdir`; no fue un fallo de rebind ni requirió un fix de producción.
- Ambos tests usan `MockProvider`, `CaptureSessions`, `ConnectionAccess::new(Arc::new(HostSecrets::default()))` y, en el segundo builder, `Arc::new(Guardrails::default())`. El caso no ejecuta operaciones de credenciales, herramientas ni red.
- `Guardrails::default()` crea listas vacías; `ConnectionAccess::new` solo inicializa locks/mapas. Como el test no ejecuta acciones de conexión o herramientas, no hay acceso a Keychain, secretos ni red.
