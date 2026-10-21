# Propuesta: sistema de overlays de Huginn

## Intención

Separar las interacciones de la TUI de la conversación: ayuda, formularios,
configuración y avisos no deben contaminar el historial. Unificar foco, render
y cierre de overlays, conservando los flujos funcionales y las garantías de seguridad.

Esta propuesta fija las decisiones aprobadas y sustituye las sugerencias
incompatibles de `exploration.md` (toasts, placeholders y dedup de conversación).
El plan RPI referenciado allí no existe en la ruta indicada; no se presupone su contenido.

## Alcance

### Incluido: P0 + P1, un solo PR

- Enum de overlays en presentation: Settings, Provider, Login, ModelPicker,
  Help, Confirm y Approval, además de alojar los diálogos/workflows existentes.
  Settings absorbe `SettingsDialog`; Approval conserva su `oneshot` propio,
  separado de las confirmaciones ordinarias.
- Cinco categorías: Conversation, Overlay, Notification, Status y Diagnostic,
  con métodos explícitos por categoría, sin `notice` genérico ni clasificación
  por prefijos. Notifications y diagnostics usan superficies no conversacionales.
- `CommandResult::Overlay(OverlayKind)` en plugin-api para solicitar overlays
  permitidos del host, sin exponer a plugins la facultad de aprobar acciones.
- Barra compacta siempre visible, también con overlay abierto; proveedor,
  modelo y modo actualizados mediante `Event::ConnectionChanged` en core.
- Renombrar `committed` a `conversation`; limitar el buffer de eventos de la
  proyección UI a un `VecDeque` de 256 entradas, sin truncar la sesión persistida.
- Deduplicar errores por hash de mensaje en ventana de 2 segundos, con reloj
  inyectable, ámbito de operación y comprobación de igualdad ante colisiones.
- Migrar consumidores, fixtures y pruebas junto al cambio; conservar providers,
  login y model funcionales, persistencia de Settings, secretos y leases/paste.

### Exclusiones

- Toast layer, aplicación live del tema y onboarding P2.
- Sustituir providers/login/model por placeholders, ampliar permisos de plugins
  o cambiar contratos de persistencia/configuración sin necesidad de la migración.
- Reabrir o modificar `settings-modal/state.yaml`, ni considerar sus gates cumplidos
  por figurar como archived sin informes verify/qa.

## Capacidades

- **Nuevas:** `huginn-overlays` (ciclo de vida, foco, comandos y Approval),
  `huginn-ui-events` (categorías, historial y dedup), `huginn-status-bar`
  (visibilidad y sincronización de conexión).
- **Modificada:** `ui-settings`, para integrar Settings y sus avisos sin perder
  working copy, apply/discard ni persistencia. `providers-ui` describe el dashboard
  Rook, no estos flujos TUI; no se modifica por coincidencia de nombre.

## Enfoque

Migración strangler-fig: introducir las nuevas superficies, trasladar cada flujo
con sus verificaciones y retirar los campos legacy solo al final. El host conserva
la autoridad de Approval y revoca el foco al cerrar, reemplazar o cancelar overlays.
El enrutamiento por categoría ocurre antes de deduplicar: una interacción UI nunca
se convierte en conversación, tampoco al vencer la ventana. No se deduplican
mensajes conversacionales legítimos ni se mezclan errores de operaciones independientes.

## Áreas afectadas

- `crates/huginn/presentation/src/{state.rs,terminal.rs}`: proyección, historial,
  input, snapshots y render; nuevos módulos de overlays, categorías y barra.
- `crates/huginn/presentation/src/{settings.rs,providers.rs,contributions.rs,workflow.rs}`:
  adaptación de flujos reales y sus resultados.
- `crates/huginn/plugin-api/src/v1.rs`: resultado declarativo de overlay.
- `crates/huginn/core/src/lib.rs` y
  `crates/huginn/application/src/connections.rs`: evento y publicación de selección.
- `apps/huginn/src/` y consumidores de eventos: cableado inicial y compatibilidad.
- `crates/huginn/presentation/tests/` y `apps/huginn/tests/providers_flow.rs`:
  regresiones de historial, overlays, conexión, seguridad y PTY.

## Riesgos y gaps técnicos

- **Alto — foco/Approval:** evitar aprobaciones accidentales, respuestas tardías y
  filtración de secretos al reemplazar overlays; cancelación/cierre/shutdown fail-closed.
- **Medio — contrato incompleto:** `ConnectionChanged` aún no existe; design debe
  concretar payload, emisor, snapshot inicial y orden frente a cambios asíncronos.
- **Medio — dedup:** concretar identidad de operación, límite y expiración del estado
  y anclaje temporal de la ventana; el hash no basta y su implementación queda a design,
  sin imponer una dependencia nueva como fxhash.
- **Medio — precedencia:** `ui-settings` REQ-12 contiene una cadena y un escenario de
  plugin/Settings contradictorios; spec debe hacer coherente el contrato de reemplazo
  sin cambiar la prioridad de Approval ni las garantías de foco.
- **Alto — regresiones y revisión:** un PR P0+P1 puede exceder el presupuesto de revisión;
  tasks debe estimarlo y dejar visible el riesgo, manteniendo la entrega única aprobada.

## Rollback

Revertir el PR P0+P1 completo, incluyendo plugin-api, evento core, consumidores y
pruebas, para evitar contratos a medias. No requiere migración de datos: conservar
sesiones, credenciales y `[ui]` existentes; nunca resetear la base de datos.

## Dependencias y validación pendiente

Reutilizar Settings y los servicios de conexión existentes, sin dar por validado
`settings-modal`: faltan informes verify/qa y no equivalen a un gate aprobado.
Los próximos verify/qa deben aportar evidencia propia. El target `pty` está
condicionado por `test-driver` y Unix; el bin `tui-fixture` no contiene esos tests.
`--all-features` sí activa `test-driver`. No atribuir fallos a la base ni a flakiness
sin comparación. Esta fase no ejecuta builds ni pruebas de runtime.

## Criterios verificables de éxito

- [ ] Abrir/cerrar ayuda, Settings y flujos providers/login/model no añade entradas
  a conversación, incluso repitiendo la acción tras más de 2 segundos.
- [ ] Mensajes de usuario/agente y resultados relevantes de herramientas siguen en
  conversación; avisos, errores UI y diagnostics permanecen fuera.
- [ ] Settings conserva apply/discard/errores de persistencia; providers/login/model
  completan sus operaciones existentes sin placeholders.
- [ ] Approval preempta otros overlays; cierre, cancelación, shutdown y callbacks
  obsoletos no conceden permisos. Plugins no pueden crear ni resolver Approval.
- [ ] Secretos no aparecen en historial/logs; input y paste obsoletos no cruzan leases.
- [ ] Barra visible con y sin overlay refleja proveedor/modelo/modo al cambiar conexión.
- [ ] Con reloj controlado, errores iguales de la misma operación se suprimen dentro
  de 2 segundos y reaparecen fuera; operaciones distintas y colisiones no se suprimen.
- [ ] La proyección conserva como máximo 256 eventos y elimina los más antiguos;
  sincronización, streaming e historial persistido mantienen su comportamiento.
- [ ] Se retiran campos legacy y se adaptan pruebas/fixtures al contrato nuevo;
  los textos PTY pueden moverse a overlays, nunca permanecer en historial solo por un test.
- [ ] Verify y QA propios documentan evidencia, fallos y casos no probados antes de archivar.
