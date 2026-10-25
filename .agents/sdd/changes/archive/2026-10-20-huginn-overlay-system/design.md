# Diseño: sistema de overlays de Huginn

## Enfoque técnico

Un slot `Option<Overlay>` gobierna foco, input y render. Se conserva la
implementación de formularios y servicios, no se sustituye por etiquetas vacías.
La proyección separa conversación, overlays, notificaciones, estado y diagnósticos.
El router publica observabilidad de selección local, no disponibilidad remota.
Alcance y precedencia: propuesta aprobada y los cuatro delta specs de este cambio.

Lectura comprobada: `state.rs`, `terminal.rs`, `workflow.rs`, `settings.rs`,
`providers.rs`, `contributions.rs` de presentation; contratos core/plugin-api;
conexiones/router de application; cableado app y manifests/tests relacionados.
Esta fase no ejecuta compilación, pruebas ni QA y no acredita gates anteriores.

## Decisiones de arquitectura

### D1. Profundizar el módulo de overlays, no añadir handlers con acceso a Ui

**Elección:** `overlay.rs` concentra transición, snapshot e input puro; devuelve
efectos que `Ui` ejecuta fuera del mutex. Eliminar los cuatro slots y leases
incompatibles al terminar. `Overlay` no recibe `&Ui` en sus métodos mutables. **Alternativa:** trait de handlers que vuelven a bloquear `Ui` o mantener slots
con prioridades. **Razón:** evita reentrancia bajo lock y concentra las garantías
de foco en un módulo testeable. No se reestructura el kernel ni el router de turnos.

### D2. Etiqueta de superficie no es payload ni autoridad

**Elección:** añadir `CommandResult::Overlay(OverlayKind)`; nunca existe
`OverlayKind::Approval`. Conservar `Workflow(Workflow)` y `Dialog(String)`.
Añadir metadatos de superficie al workflow, preservando su definición, callback,
registration exacta, campos y secretos. No reconstruir un workflow por título,
prefijo, nombre de comando ni lookup de una generación sustituta. **Alternativa:** devolver solo `OverlayKind::Login` en lugar del workflow actual. **Razón:** esa sustitución pierde callbacks, contexto y continuación asíncrona.

### D3. Un solo destino visible por error UI

**Elección:** categoría explícita antes de dedup; errores de formulario en su
cuerpo y otros errores en la zona de avisos. Sin toasts ni `notice` universal.
`CommandResult::Notice(View)` sigue siendo un resultado plugin compatible,
pero significa exclusivamente Notification, nunca conversación. **Alternativa:** clasificar texto o deduplicar historial. **Razón:** ocultaría
mensajes legítimos y permitiría contaminar historial al expirar una ventana.

### D4. Fuente versionada en el punto de serialización del router

**Elección:** el servicio de conexiones publica un snapshot completo y versionado
bajo su mutex de selección; `ConnectionManager` lo reenvía y la barra consume ese
canal. `UiState` no consulta el estado remoto. **Alternativa:** consultas periódicas
o texto emitido por comandos. **Razón:** ordena selección, ausencia y revocación
sin afirmar conectividad, y permite reflejar cambios de todos los caminos.

### D5. Vistas declarativas sin autoridad de ejecución

**Elección:** la proyección `UiView`/snapshot incluye categorías explícitas,
`OverlaySnapshot` y estado observable de la barra; no serializa capacidades,
handlers, callbacks ni secretos. La superficie activa se presenta como overlay,
no como contenido conversacional. **Alternativa:** emitir avisos y reconstruir la
vista desde texto. **Razón:** mantiene una frontera comprobable entre interacción,
conversación y datos de observabilidad.

### D6. Errores deduplicados por contenido, operación y tiempo

**Elección:** aplicar dedup solo a errores de presentación ya clasificados, con
hash y comparación de mensaje, reloj inyectable, operación y ventana aprobada. **Alternativa:** dedup global o basada en prefijo. **Razón:** evita suprimir fallos
independientes o alterar el historial; la política exacta permanece definida por
los specs existentes.

### D7. UI conserva eventos acotados, sesión conserva conversación completa

**Elección:** limitar a 256 entradas el buffer de proyección de eventos UI; la
persistencia de `Session.events` y los mensajes relevantes de conversación no se
truncan. **Alternativa:** aplicar el límite al registro durable. **Razón:** el
límite controla memoria de UI sin cambiar recuperación de sesión.

### D8. Integración progresiva de estado, sin doble autoridad

**Elección:** migrar primero la propiedad del estado activo al `UiState` host-owned;
durante la transición, los objetos legacy pueden ser adaptadores/payload exactos
dentro de `Overlay`, pero sus slots dejan de decidir visibilidad, foco o dispatch.
Snapshot/render y terminal input consultan ese mismo estado. **Alternativa:**
conservar cuatro slots y sincronizarlos con `ActiveOverlay<T>`. **Razón:** sincronizar
copias no permite garantizar orden ni lease único; una máquina aislada que no dirige
input/render tampoco completa OVL-01. Esta decisión precisa D1: eliminar slots
independientes implica migrarlos por adaptadores, no descartar sus flujos.

### D9. Precedencia sin pila y autorización separada

**Elección:** `Approval` solo se crea por la autoridad host, preempta y descarta
el overlay ordinario sin restaurarlo, cancela su tarea local si procede; una apertura
ordinaria durante Approval se rechaza. No hay pila. **Alternativa:** prioridad con
restauración o permitir que plugins reemplacen Approval. **Razón:** conserva
exclusividad de permiso y evita resucitar estado/foco revocados.

## Data Flow

```text
CommandResult / evento async
        │ validar origin + ticket + operation + revision
        ▼
UiState.active_overlay ──→ snapshot/render
        ▲                         │
        └──── terminal input ─────┘
                   │ lease/revision vigente
                   ▼
             transición + efectos
                   │ desbloquear antes de ejecutar
                   └── I/O / callbacks / cancelación / Drop
```

El snapshot es una proyección segura, no una serialización del payload de View.
Debe ocultar valores secretos y buffers de formulario, senders/receivers, callbacks,
handles de recursos, closures, referencias de renderer/servicio y tokens de
cancelación. La autoridad host retiene esos elementos hasta completar o invalidar
la operación; no salen a eventos JSON, logs, diagnósticos o conversación.

## Revisión de integración (2026-10-19)

### Estado y adaptadores

La autoridad activa vive en un único campo host-owned dentro de `UiState`. La
máquina conserva overlay, origen exacto, operación, revisión/ticket y `FocusLease`.
No existe estado activo paralelo en `modal`, `plugin_dialog`, `workflow` ni
`settings`. `FocusLease` y revisión avanzan con `checked_add`; agotamiento falla
cerrado, nunca hace wrap ni reutiliza una identidad. Cada evento de terminal se
enruta al estado activo y lleva el lease/revisión capturados al comienzo. La
aplicación de una tecla valida ambos contra el estado vigente; si hubo cierre,
reemplazo o preemption, se descarta. Esc consumido para cerrar no se reinserta.
Paste conserva epoch fresco por lease; fragmentos de un epoch revocado se descartan.

| Origen actual                           | Adaptación dentro de Overlay                                                                                                                   | Retención host / cierre                                                                                        |
|-----------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------|
| Modal/Approval                          | payload exacto de Approval, `oneshot` y epoch propios; nunca `OverlayKind` solicitado por plugin                                               | Solo host crea/resuelve; cerrar/shutdown invalida epoch y resuelve cancelado, sin concesión                    |
| `plugin_dialog` / `DialogHandle`        | `Dialog(String)` conserva exactamente handle, registration y renderer/callback asociado; superficies tipadas usan variante y payload propios   | Revocación cancela trabajo local; no se busca contribución sustituta por nombre                                |
| `workflow` / `HostWorkflow` / `Pending` | `Workflow` conserva definición registrada, campos, callback, continuation y contexto; superficie opcional no reemplaza workflow                | Pending/cancel/abort permanecen bajo host; receipt aplica solo si ticket de origen y operación siguen vigentes |
| `SettingsDialog`                        | se adapta a `Overlay::Settings` con copia editable exacta, sin una segunda autoridad                                                           | persistencia usa snapshot capturado; receipt tardío no cierra/edita overlay actual ni afecta Approval          |
| Widgets                                 | widgets que representan interacción modal se integran como payload de variante; widgets puramente informativos siguen en su región de `UiView` | ningún widget informativo posee foco/estado activo overlay                                                     |

La definición tipada puede ser `Overlay { kind, payload }` con payloads que
conserven sus tipos legacy durante esta migración; `OverlaySnapshot` solo expone
kind, texto/campos no secretos y estado de presentación necesarios para render.
No copiar handlers al snapshot ni convertir callbacks en strings.

### Transacciones y efectos externos

Bajo lock se valida autoridad/origen/revisión y se realiza una transición atómica:
se retira el overlay anterior, se instala el nuevo si la política lo permite, se
incrementan revisión y lease, se invalida ticket anterior y se extraen recursos
que deban finalizar. No se llama renderer, callback, I/O, cancel/abort ni destructor
bajo lock. Después de soltarlo se procesan efectos de forma determinista: primero
invalidación lógica ya publicada, luego cancelación/abort local y destrucción de
payload retirado; después callbacks/renderers o I/O asociados a la nueva operación.
Cada callback reentrante vuelve a validar origin/ticket/revision al intentar
publicar resultado; no conserva derecho implícito a reabrir. Approval preempta
atómicamente, extrae overlay ordinario, invalida su lease/ticket y cancela trabajo
local fuera lock. Ordinario durante Approval no muta estado. Al cerrar workflow,
se invalida ticket y se cancela ejecución local; completion tardía se consume y
descarta sin reabrir ni mutar overlay posterior.

Resultados async solo aplican si coinciden ticket, origin exacto, operation y
revision esperados. La identidad incluye registration/handle original, no nombre
de plugin. Un Apply de Settings iniciado puede terminar según SET-07: éxito vigente
actualiza snapshot persistido y cierra solo Settings/Apply aún vigentes; éxito
obsoleto puede actualizar únicamente el snapshot guardado, y fallos obsoletos no
alteran estado visible ni snapshot. Nada de ello restaura foco o desplaza Approval.

### Aceptación vertical propuesta para 2.2

Pruebas host, no solo unitarias de la máquina aislada, deben demostrar como mínimo:

1. `CommandResult::Overlay(Help)` abre una vista real y no una notificación, y la
   apertura/cierre no añade entradas de conversación.
2. Settings completo (edición, Apply pendiente, éxito/fallo) conserva su flujo real
   integrado y los receipts obsoletos no modifican overlay actual.
3. Snapshot y render, así como input terminal, reflejan el mismo `UiState`; el input
   requiere lease/revisión vigente, y Esc de cierre nunca llega al editor.
4. Ticket obsoleto, workflow cerrado, approval preemption y completion tardía se
   descartan; approval host-only no puede ser reemplazado por solicitudes plugin.
5. Workflow mantiene continuación/callback/registration exactos; cancelación local
   ocurre ante cierre/revocación y no declara rollback durable/remoto.
6. Renderer/callback reentrante y destructor de Pending/payload que consulta Ui no
   bloquean ni deadlockean: se invocan fuera del mutex.
7. Más de 256 eventos de UI no trunca la sesión persistida ni hace reaparecer una
   interacción UI en conversación, incluso después de expirar dedup.

Estos son criterios verificables derivados de OVL-01/02/03/04/05, SET-07/12 y
EVT-01/02; no agregan capacidades de producto.

### Cortes dependency-ordered y brecha de tareas

La tarea 2.2 actual combina máquina y snapshots pero exige no integración vertical;
2.3–2.5 migran flujo/render/input después, lo que deja sin propietario único el
estado y hace imposible aceptar OVL-01 durante el intervalo. Se propone conservar
la numeración y la autorización global, pero ordenar los paquetes de revisión así:

1. **2.2a — autoridad host integrada:** `UiState` pasa a poseer `ActiveOverlay`,
   transiciones, tickets/leases y snapshot seguro; conectar dispatch de resultados
   y apertura/cierre básicos de todas las superficies sin quitar los payloads.
2. **2.2b — adaptadores y lifecycle:** migrar Dialog/Handle, HostWorkflow/Pending,
   Approval/oneshot+epoch, SettingsDialog y widgets al payload; extraer efectos y
   cerrar/revocar/cancelar fuera lock, manteniendo callbacks y receipts exactos.
3. **2.2c — input/render vertical y aceptación:** terminal y renderer leen/escriben
   por la autoridad compartida; eliminar ownership activo legacy; ejecutar pruebas
   host de la lista anterior antes de avanzar.
4. Las tareas 2.3–2.5 existentes contienen parte de esas mismas migraciones (workflow/dialog, Approval y input/paste). El dueño de tasks debe rebaselinizarlas
   como endurecimiento/cobertura restante después de 2.2c, quitando duplicación y
   conservando sus contratos y gates; no pueden ser precondición para integrar la
   autoridad en 2.2. 2.6 conserva los flujos Provider reales, pero depende de la
   autoridad/adaptadores de 2.2c y del endurecimiento necesario de input, no de
   introducir un segundo dueño. 3.1 adapta Settings visual/funcional sobre esa misma
   autoridad; no reintroduce `SettingsDialog` como slot.

Los subcortes 2.2a–c y la re-baselina de dependencias/tareas son propuestas para el
dueño de ese artefacto; aquí no se marca ninguna tarea como completa ni se cambia
`tasks.md`, `state.yaml` o progreso. 2.2 debe permanecer bloqueada para apply global
hasta que el orquestador actualice el desglose y apruebe el orden/dependencias.
No implica una capa de toast: avisos y diagnósticos siguen en superficies no
conversacionales especificadas.

## Interfaces / Contratos

```rust
enum OverlayKind { Settings, Provider, Login, ModelPicker, Help, Confirm }
// Approval se origina exclusivamente en el host y no está en OverlayKind.
struct OverlayTicket { operation: OperationId, revision: Revision }
struct FocusLease { id: FocusId, paste_epoch: PasteEpoch }
struct OverlaySnapshot { kind: OverlayKind, revision: Revision, view: SafeOverlayView }
```

`open/close` validan origen y ticket; devuelven transición y efectos tipados,
no ejecutan efectos externos. Un `Overlay` puede alojar temporalmente el payload
tipado legacy, pero `UiState` es su único propietario activo. El snapshot omite
callbacks, senders, recursos y secretos. Ninguna interfaz ofrece a plugins una
capacidad de input/render ni resolución de Approval.

## Estrategia de pruebas

| Capa             | Qué probar                                                                                                   | Enfoque                                                           |
|------------------|--------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------|
| Unit             | autorización, ticket/origin/revision, monotonía, lease/paste, transiciones y snapshot redacted               | reloj/canales controlados, overflow falla cerrado                 |
| Integración host | apertura real, settings, workflows, preemption, cancelación, callbacks/Drop reentrantes y resultados tardíos | harness `UiState`/terminal/render, barreras para ordenar carreras |
| Contratos        | comandos declarativos, Approval inaccesible, registration exacta                                             | tests plugin-api y dispatch host                                  |
| Persistencia/PTY | categorías fuera de conversación, cierre sin tecla reinyectada, >256 sin truncar sesión                      | tests projection/session y PTY existentes                         |
| Selección/barra  | snapshot local versionado y eventos de conexión                                                              | unitarias de application/core y consumidor UI                     |

## Migración / Despliegue

No requiere migración de datos ni feature flag. Es un cambio de contratos compilados
que debe integrar consumidores y fixtures en la misma entrega autorizada.
La integración se hace por paquetes de revisión dependency-ordered, sin afirmar
que fases futuras estén completas.

## Preguntas abiertas

- [ ] El contrato técnico de snapshot admite `SafeOverlayView` sugerido; concretar
  campos no secretos contra tipos reales al implementar, sin ampliar datos visibles.
- [ ] El dueño de tasks debe decidir cómo formalizar 2.2a–c y las dependencias nuevas
  respecto de 2.3–2.6 antes de reanudar apply global.
- [ ] Verificar durante implementación si el destino de avisos existente cubre
  todos los errores de formularios sin una superficie adicional (no toast layer).
