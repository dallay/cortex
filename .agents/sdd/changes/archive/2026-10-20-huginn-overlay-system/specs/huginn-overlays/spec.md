# huginn-overlays — Especificación

## Propósito

Unificar las interacciones de la TUI sin ampliar autoridad de plugins ni perder
los flujos existentes. No incluye onboarding, capa de toasts ni tema live.

## Requirements

### Requirement: OVL-01 — Un único overlay activo

El sistema MUST representar los overlays mediante un enum `Overlay` unificado
que incluya `Settings`, `Provider`, `Login`, `ModelPicker`, `Help`, `Confirm`
y `Approval`, y aloje los diálogos/workflows existentes. Render e input MUST
referirse al mismo overlay activo. Un overlay MUST consumir sus teclas sin
enviarlas al editor ni a completion. Cerrar MUST devolver el foco al editor
sin reenviar la tecla de cierre. La migración MUST terminar sin slots legacy
paralelos `modal`, `plugin_dialog`, `workflow` o `settings`.

#### Scenario: OVL-01-A — Ayuda y retorno al editor

- GIVEN el editor disponible y sin overlay
- WHEN se ejecuta `/help` y después se cierra `Help`
- THEN la ayuda se presenta en `Help`, no en conversación
- AND durante su apertura las teclas pertenecen a `Help`
- AND al cerrar se recupera el editor sin insertar ni enviar la tecla de cierre

### Requirement: OVL-02 — Solicitudes declarativas autorizadas

`CommandResult::Overlay(OverlayKind)` MUST permitir a comandos solicitar overlays
autorizados del host sin otorgar capacidades de input, render o permisos.
Los plugins MUST NOT crear ni resolver `Approval`, obtener su `oneshot` ni
usar `Confirm` como aprobación de herramientas. Solicitudes no autorizadas,
inválidas o revocadas MUST rechazarse sin efectos ni apertura alternativa.

#### Scenario: OVL-02-A — Solicitud permitida

- GIVEN un comando vigente autorizado para solicitar `Help`
- WHEN devuelve `CommandResult::Overlay(OverlayKind)` para `Help`
- THEN el host abre `Help` con sus controles y foco propios
- AND no se añade conversación ni se concede autorización

#### Scenario: OVL-02-B — Sin autoridad de aprobación

- GIVEN un plugin que solicita un overlay no permitido o muestra `Confirm`
- WHEN se procesa la solicitud o se acepta esa confirmación ordinaria
- THEN ninguna acción pendiente obtiene aprobación de herramientas
- AND la solicitud no permitida deja un error no conversacional y no reemplaza el overlay activo

### Requirement: OVL-03 — Approval exclusivo y fail-closed

Solo el host MUST abrir `Approval` y poseer su respuesta `oneshot`, separada
de `Confirm`. `Approval` MUST preemptar cualquier overlay ordinario, descartarlo
y revocar su foco sin restaurarlo después. Mientras esté pendiente, solicitudes
ordinarias MUST NOT reemplazarlo. Solo una aceptación explícita válida de la
solicitud vigente MUST conceder permiso. Cierre, cancelación, pérdida de canal,
revocación y shutdown MUST finalizar sin concederlo; callbacks obsoletos MUST
NOT resolver una solicitud posterior ni causar dobles respuestas.
Preemptar Settings antes de iniciar Apply MUST NOT escribir; si Apply ya inició
escritura, la preemption MUST NOT iniciar otra ni prometer rollback. Su receipt
MUST seguir REQ-07 de `ui-settings` sin reabrir Settings ni alterar Approval.

#### Scenario: OVL-03-A — Preemption y aceptación vigente

- GIVEN `Settings` con cambios sin guardar, sin Apply iniciado y una solicitud de aprobación del host
- WHEN se abre `Approval` y el usuario completa su aceptación explícita válida
- THEN `Settings` se descarta sin persistir y su foco queda revocado
- AND solo la solicitud vigente recibe una aceptación, una sola vez
- AND `Settings` no reaparece al terminar

#### Scenario: OVL-03-B — Cierre y shutdown

- GIVEN una `Approval` pendiente
- WHEN ocurre cierre, cancelación, pérdida del canal o shutdown, en casos separados
- THEN la espera termina sin permiso y sin quedar bloqueada
- AND entrada o callbacks posteriores no conceden permiso ni reabren el overlay

#### Scenario: OVL-03-C — Reemplazo ordinario bloqueado y respuesta tardía

- GIVEN una `Approval` vigente y una respuesta tardía de una solicitud anterior
- WHEN llega esa respuesta y un comando solicita `Help`
- THEN `Approval` sigue activa y sin resolver
- AND ni el callback anterior ni el comando alteran su autoridad o foco

### Requirement: OVL-04 — Revocación, secretos y leases

Cerrar, reemplazar, cancelar o revocar un overlay MUST invalidar su lease de
input, descartar sus buffers editables y cancelar su tarea local cuando sea
cancelable. Una escritura de Settings ya iniciada MAY completar: MUST conservarse
solo lo necesario para consumir su receipt según REQ-07 de `ui-settings`, sin
restaurar foco ni retener secretos de formularios. Cerrar MUST NOT iniciar una
escritura adicional ni afirmar que la iniciada fue deshecha. El sistema MUST NOT buscar
una contribución sustituta por nombre para continuar una acción revocada.
Secretos MUST permanecer en campos secretos con máscara constante y MUST NOT
pasar por editor conversacional, historial, eventos JSON, logs o diagnósticos.
Paste y teclas capturados por un lease revocado MUST NOT cruzar al siguiente
overlay o al editor. Cancelar localmente MUST NOT afirmar rollback remoto o durable.

#### Scenario: OVL-04-A — Paste incompleto y cambio de foco

- GIVEN `Login` con un secreto centinela y paste incompleto bajo su lease
- WHEN se cancela con Ctrl+C y se abre otro overlay antes de llegar el resto del paste
- THEN ningún fragmento llega al nuevo overlay ni al editor
- AND el secreto no aparece en conversación, capturas de terminal, JSON ni logs
- AND Esc dentro del paste incompleto sigue tratándose como delimitador, no como aceptación

#### Scenario: OVL-04-B — Contribución revocada durante operación

- GIVEN un formulario pendiente de una contribución registrada
- WHEN se revoca y registra otra del mismo nombre y después llega el resultado anterior
- THEN se cierra el formulario, revoca el foco y cancela su tarea local
- AND no se ejecuta la sustituta ni se aplica el resultado tardío a la nueva UI
- AND cualquier aviso de cancelación no promete deshacer efectos ya realizados

### Requirement: OVL-05 — Flujos de conexión funcionales

`Provider`, `Login`, `ModelPicker` y las confirmaciones MUST conservar las
operaciones existentes de `/providers`, `/login`, `/model` y `/logout`, no
placeholders. MUST conservar validación, errores recuperables, cancelación,
selección para el siguiente turno, revocación local y tratamiento de credenciales.
Fallo del almacenamiento seguro MUST NOT seleccionar almacenamiento efímero
implícitamente. OAuth no soportado MUST seguir comunicado como no soportado.

#### Scenario: OVL-05-A — Configurar y seleccionar sin reiniciar

- GIVEN un servicio de conexiones controlado con un proveedor y modelo válidos
- WHEN se completa `/login`, se inspecciona `/providers` y se selecciona otro modelo con `/model`
- THEN se realizan las operaciones reales del servicio y el siguiente turno usa la selección confirmada
- AND un turno iniciado conserva su conexión/modelo salvo los bloqueos por revocación existentes
- AND ningún paso requiere un placeholder ni introduce conversación UI

#### Scenario: OVL-05-B — Error seguro y logout

- GIVEN almacenamiento seguro que falla y una conexión existente en casos separados
- WHEN se intenta guardar credenciales o se confirma `/logout`, respectivamente
- THEN el fallo se muestra fuera de conversación sin fallback efímero implícito
- AND logout conserva revocación local y limpieza de credenciales sin afirmar invalidación remota
- AND cancelar cualquiera de los formularios no confirma la operación
