# Propuesta: Sustitución de proveedores de Huginn

> **Slice**: Huginn EPIC 01 · **Issue**: #509 · **Epic**: #442 / DALLAY-718<br>
> **Change**: `huginn-service-provider-substitution` · **Fase**: Propuesta<br>
> **Artifact store**: `.agents/sdd/`

## Intención

Permitir sustituir servicios sin reiniciar Huginn, con garantías para consumidores y ciclo de vida. Hoy retirar y registrar plugins por separado bloquea dependientes e impide restaurar el provider previo ante fallos.

## Alcance

### Incluye
- Sustitución transaccional en `huginn-core`, con publicación y rollback.
- Ownership, dependientes y leases cooperativas con drenaje acotado.
- Rebind del loop productivo; tests para model y session store.

### Excluye
- Catálogo, carga dinámica, hot reload, ABI estable, WASM o component model.
- Migración de plugins o datos; cambios TUI/UX o de aprobación.

## Capacidades

- **Nueva** `huginn-kernel`: sustitución transaccional, leases y rebind productivo.
- **Modificadas**: ninguna.

## Enfoque

Extender el kernel: mantener ownership/generaciones, activar alternativas mediante `Plugin` y publicar tras drenaje. El timeout aborta sin cancelar trabajo válido.

`huginn_application::composition::build` registra el adapter leased. `huginn_runtime::composition::build_native`, `build_native_with_connections` y `build_native_with_connections_and_guards` comparten la ruta de `leased_loop_plugin` mediante el helper privado `build_native_plugin`; el builder público `huginn_runtime::composition::build` delega en la composición application. El `LoopService` retenido lee slots vigentes en el siguiente `run`/`compact`; operaciones activas mantienen sus leases. No se garantiza rebind para `LoopPlugin(pub LoopConfig)` legacy ni composiciones personalizadas que solo capturan `Arc` con `resolve()`. No se migran datos: `SqliteSessions::lease/load/list` siguen fuera de `SessionsService`.

## Áreas afectadas

| Área | Impacto |
|---|---|
| `crates/huginn/core/src/{kernel.rs,lib.rs}` | Slots y sustitución. |
| `crates/huginn/application/src/composition.rs` | `huginn_application::composition::build` y adapter leased. |
| `crates/huginn/runtime/src/composition.rs` | Builders públicos `build`, `build_native`, `build_native_with_connections` y `build_native_with_connections_and_guards`; helper compartido privado. |
| `apps/huginn/src/main.rs` | Composición y readiness. |
| Tests Huginn; arquitectura/ADR-0003 | Rebind y límites. |

## Riesgos

| Riesgo | Mitigación |
|---|---|
| `Arc` retenidos no se revocan | Garantía solo con leases. |
| Drenaje/rollback incorrectos | Definir orden y probar dos servicios. |
| Conflicto con PR #508 | No tocar catálogo ni carga de componentes. |

## Plan de rollback

Revertir restaura el comportamiento previo; no hay dependencias ni migración de esquema.

## Dependencias

Sin dependencias externas nuevas; evitar conflictos con PR #508.

## Criterios de éxito

- [ ] Sustituir sin reinicio; rollback limpio e introspección actualizada.
- [ ] Drenaje con timeout que no cancela operaciones activas.
- [ ] El `LoopService` retenido por `huginn_application::composition::build` o por los builders públicos de `huginn_runtime::composition` (`build_native`, `build_native_with_connections`, `build_native_with_connections_and_guards`) usa slots nuevos en el siguiente `run`/`compact`, sin reconstrucción; la operación activa termina con sus leases.
- [ ] Sin garantía para `LoopPlugin(pub LoopConfig)` legacy ni composiciones que retienen `Arc` de `resolve()`.
- [ ] Tests model/session; sin migrar `SqliteSessions::lease/load/list`.
- [ ] API v1 compatible; pasan arquitectura y suites focalizadas, con evidencia.
