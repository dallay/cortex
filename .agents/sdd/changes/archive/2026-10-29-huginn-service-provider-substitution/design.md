# Diseño: Rebind dinámico con API compatible de `StandardLoop`

> **Cambio**: `huginn-service-provider-substitution` · **Issue**: #509 · **Fase**: design reabierta

## Evidencia y decisión

`loop_engine` es público (`crates/huginn/application/src/lib.rs`); `StandardLoop` y sus seis campos son públicos (`crates/huginn/application/src/loop_engine.rs`). Hay struct literals en `crates/huginn/application/tests/providers_turns.rs` y `crates/huginn/runtime/tests/coding_workflow.rs`, `crates/huginn/runtime/tests/providers_integration.rs` y `crates/huginn/runtime/tests/stream_persistence.rs`. Cambiar tipos/campos rompería esos consumidores. La solución compatible conserva exactamente `StandardLoop`, campos y firmas; no convierte `model`/`sessions` en slots.

| Opción                                                      | Coste                                               | Decisión                                                                        |
|-------------------------------------------------------------|-----------------------------------------------------|---------------------------------------------------------------------------------|
| Cambiar campos públicos de `StandardLoop` a handles         | Rompe struct literals y el contrato de tipos.       | Rechazada.                                                                      |
| Mantener `StandardLoop`; wrapper dinámico + core compartido | Añade un adaptador privado y factoriza ejecución.   | Elegida; conserva API y evita duplicar comportamiento.                          |
| Locks por slot                                              | Más concurrencia, pero requiere protocolo de grupo. | Rechazado frente al mutex global, que sincroniza lectura valor/gate/incremento. |

En `crates/huginn/core/src/kernel.rs`, `PluginContext::resolve` usa servicios capturados; `Supervisor::start` activa plugins listos en una pasada y `LoopPlugin::activate` solo recibe contexto (`crates/huginn/application/src/composition.rs`). Por eso `Supervisor::service_slot_handle` no es accesible en `activate`. Se mantiene A: `PluginContext::service_slot_handle<T>(&ServiceId)` crea un handle solo para IDs de `Manifest.requires` y rechaza requisito no declarado, slot ausente o tipo incompatible. `activate_plugin` y `stage_plugin` pasan `required_services` del manifiesto y `service_slots: self.service_slots.clone()`; sin ramas de capability.

La tabla almacena `ServiceId` → `ServiceSlot { value, gate }`; `ServiceSlotHandle<T>` retiene tabla e ID. `acquire()` lee valor+gate e incrementa lease bajo la sincronización compartida. Tras stage y drain, un commit sustituye todos los slots y revoca gates anteriores de forma conjunta; no publica generaciones parciales. `resolve() -> Arc<T>`, `ServiceHandle<T>` y `huginn-plugin-api::v1` permanecen sin cambios.

## Ejecución compartida y ciclo de vida

Factorizar la ejecución en `LoopExecution<'a>`: referencias por operación a model/sessions y referencias estables a tools, instructions, config y lifecycle. Compartir allí `pin_model`, `record`, `request`, `context`, `summarize`, `collect_response`, `execute_tool`, `turn`, `run`, `compact`, `run_with_skills` y `run_with_skills_and_catalog`; `skill_commands` conserva lógica común sin leases. `StandardLoop` mantiene campos y firmas públicas, delega con servicios estáticos y deja intactos los struct literals de tests.

Producción usa `LeasedAgentLoop`, adapter de `AgentLoop` que no contiene `StandardLoop` ni retiene `Arc` antiguos de model/sessions. Los builders públicos `huginn_runtime::composition::{build_native, build_native_with_connections, build_native_with_connections_and_guards}` llaman al helper privado `build_native_plugin`, que registra el loop leased mediante la factory pública `huginn_application::composition::leased_loop_plugin`. El builder público `huginn_runtime::composition::build` delega en `huginn_application::composition::build`, que usa esa misma factory. La factory es el seam compartido: ambos caminos productivos garantizan rebind dinámico y leases. El adapter adquiere admisión y ambos slots antes de efectos; crea `LoopExecution` con sus servicios y conserva las leases durante todo `run`/`compact`, incluidos streams, herramientas, aprobaciones y saves. Si falla la segunda lease, RAII libera la primera sin mutar la sesión.

`LoopAdmission` rechaza operaciones nuevas y espera las activas; durante el cierre, un run falla antes de efectos. El timeout no cancela runs activos; abort reabre admisión y conserva el provider anterior. Se preservan las APIs públicas de `StandardLoop` y `LoopPlugin(pub LoopConfig)`. Excepción legacy: el uso directo de `StandardLoop` o `LoopPlugin` conserva el loop estático, sin rebind dinámico. El tipo `LeasedLoopPlugin` es privado e implementa `Plugin::supports_drain`, `Plugin::drain` y `Plugin::resume`; esos hooks coordinan `Arc<LoopAdmission>` con `LeasedAgentLoop`. No se requiere exponer el adapter ni cambiar las APIs legacy, y no hay decisión humana pendiente sobre un nombre de tipo público.

## Readiness, persistencia y archivos

`apps/huginn/src/main.rs` consulta `ServiceSlotHandle<ModelService>` para readiness de doctor, CLI y TUI; no retiene `Arc<ModelProvider>` ni un `bool` obsoleto. `SqliteSessions::lease`, `load` y `list` (`crates/huginn/runtime/src/sessions.rs`) siguen directos al runtime, fuera de `SessionsService` y del rebind; no hay transferencia de leases ni migración de estado/esquema.

| Path                                                                | Cambio previsto                                                                                                                                                                                                                                          |
|---------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `crates/huginn/core/src/kernel.rs`                                  | Slots, contexto y publicación sincronizada.                                                                                                                                                                                                              |
| `crates/huginn/application/src/loop_engine.rs`                      | Helper común y `LeasedAgentLoop`; preservar `StandardLoop`.                                                                                                                                                                                              |
| `crates/huginn/application/src/composition.rs`                      | Factory pública `leased_loop_plugin`; implementación privada `LeasedLoopPlugin`, slots y admisión compartida.                                                                                                                                            |
| `crates/huginn/runtime/src/composition.rs`                          | Builders públicos `build_native`, `build_native_with_connections` y `build_native_with_connections_and_guards` llaman al helper privado `build_native_plugin`, que usa `leased_loop_plugin`; `build` delega en `huginn_application::composition::build`. |
| `crates/huginn/core/tests/service_lease.rs`                         | RED: handle retenido por dependent ve provider nuevo tras `substitute`; probar validación `Manifest.requires`/tipo/presencia.                                                                                                                            |
| `crates/huginn/application/tests/service_slot_rebinding.rs` (crear) | RED: wrapper, ambas leases, saves múltiples, drain/timeout y ausencia de mutación si falla la segunda.                                                                                                                                                   |
| `crates/huginn/runtime/tests/tool_lifecycle.rs`                     | Cubrir la composición runtime productiva y que el `LoopService` publicado mantiene el rebind dinámico tras sustituir `SessionsService`.                                                                                                                  |
| Struct literals actuales en tests application/runtime               | Sin cambios; prueba de compatibilidad por compilación.                                                                                                                                                                                                   |

Sin dependencias nuevas. Ejecutar tests focalizados y `just huginn-architecture`; no equivale a CI completa.

## Riesgos

El adapter privado evita rupturas de `StandardLoop` y `LoopPlugin`, con una capa de composición adicional. La garantía de leases es cooperativa y no cubre `Arc` retenidos fuera de lease ni trabajo detached.
