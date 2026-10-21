# Delta de ui-settings

La integración conserva REQ-01 a REQ-16 salvo las sustituciones completas de
REQ-07, REQ-08 y REQ-12 siguientes. No añade tema live, capa de toasts ni
onboarding; tampoco convierte `(+) Add provider…` en un nuevo flujo funcional.

## MODIFIED Requirements

### REQ-07 — Apply on Enter

Al pulsar Enter para aplicar, el sistema SHALL capturar la copia de trabajo en
`UiConfig` e invocar una sola vez por Apply la función de persistencia inyectada
`Fn(&UiConfig) -> anyhow::Result<()>`, sin bloquear el input. Mientras ese Apply
esté pendiente, Enter repetido SHALL NOT iniciar otra escritura. Los guardados
SHALL serializarse para conservar su orden. En éxito SHALL actualizar el snapshot
de preferencias guardadas para próximas aperturas; solo si el overlay y Apply
siguen vigentes SHALL cerrar `Overlay::Settings` y mostrar `Settings saved` como
`Notification` no conversacional. En fallo vigente SHALL mantenerlo abierto,
conservar la copia enviada y mostrar el error en el cuerpo sin un segundo aviso
equivalente. Un receipt tardío SHALL consumirse sin reabrir Settings, cerrar o
modificar otro overlay, recuperar foco ni afectar Approval; un éxito tardío SHALL
actualizar únicamente el snapshot guardado, no la copia editable de otro Settings
ya abierto. Un fallo tardío SHALL NOT sustituir el snapshot guardado por valores
fallidos. SHALL preservar la aplicación conjunta de todos los valores y los
contratos TOML de REQ-09 a REQ-11; SHALL NOT aplicar tema live.

Anteriormente: modal independiente con aviso de éxito; ahora integrado como overlay
y con destino explícito no conversacional.

#### Scenario: SET-07-A — Aplicación conjunta sin conversación

- GIVEN `Settings` con tema `catppuccin` y valores modificados de indicators
- WHEN se pulsa Enter y la persistencia tiene éxito
- THEN se invoca exactamente una vez con todos los valores de la copia
- AND se cierra el overlay y aparece `Settings saved` fuera de conversación
- AND no hay escrituras parciales ni cambio live de tema

#### Scenario: SET-07-B — Error mantiene abierto

- GIVEN una copia modificada, un Apply vigente y persistencia que devuelve `disk full`
- WHEN se pulsa Enter
- THEN el overlay permanece abierto con la misma copia y error visible en el cuerpo
- AND no aparece otra representación equivalente del error ni entrada conversacional
- AND Esc sigue permitiendo descartar sin una nueva invocación de persistencia

#### Scenario: SET-07-C — Apply pendiente no duplica y permite reintentar tras fallo

- GIVEN un Apply cuya persistencia controlada permanece pendiente
- WHEN se pulsa Enter varias veces y después se completa con error
- THEN solo se ha invocado una vez la persistencia y Settings conserva la copia enviada con el error
- AND un nuevo Enter tras el fallo puede iniciar un único reintento
- AND el éxito de ese reintento actualiza el snapshot para la siguiente apertura

#### Scenario: SET-07-D — Receipt tardío con otro Settings abierto

- GIVEN un Apply iniciado, su Settings cerrado y otro Settings abierto con su propia copia editable
- WHEN llega el receipt anterior, con éxito y con error en casos separados
- THEN el nuevo Settings conserva copia, foco y estado de su propia operación
- AND el éxito actualiza el snapshot guardado que se usa en una apertura posterior
- AND el error no reemplaza el snapshot guardado ni inserta un error en el nuevo Settings

### REQ-08 — Discard on Esc

Al pulsar Esc fuera de un paste incompleto, el sistema SHALL cerrar
`Overlay::Settings`, descartar su copia editable y revocar el lease sin cancelar
el turno en curso. Antes de iniciar Apply SHALL mostrar `Settings discarded`
como `Notification` no conversacional y SHALL NOT invocar persistencia ni modificar
el archivo. Si Apply ya inició escritura, Esc SHALL NOT iniciar escritura adicional
ni prometer rollback o que el archivo permanece intacto. El receipt del Apply
iniciado SHALL seguir el contrato de REQ-07 sin reinstalar el overlay ni su foco.

Anteriormente: descarte de modal y aviso; ahora cierre con revocación de foco y
clasificación explícita, conservando REQ-13.

#### Scenario: SET-08-A — Descarte durante streaming

- GIVEN `Settings` con cambios sin guardar, sin Apply iniciado y un turno en streaming
- WHEN se pulsa Esc
- THEN se cierra sin persistir y el archivo conserva sus bytes
- AND el turno continúa y la tecla no llega al editor ni al cancelador del turno
- AND `Settings discarded` se muestra fuera de conversación

#### Scenario: SET-08-B — Esc durante escritura iniciada

- GIVEN un Apply cuya escritura ha iniciado y está detenida por una barrera controlada, con un turno en streaming
- WHEN se pulsa Esc y después se libera la escritura con éxito o fallo, en casos separados
- THEN Settings se cierra inmediatamente, revoca su lease y no cancela el turno
- AND la persistencia se ha invocado solo una vez, sin escritura de descarte ni rollback prometido
- AND el receipt no reabre Settings ni recupera foco ni añade conversación
- AND en éxito la siguiente apertura posterior al receipt usa el snapshot guardado; en fallo conserva el snapshot previo

### REQ-12 — Modal precedence

El sistema SHALL usar `Approval > overlay ordinario activo > completion`.
`Settings`, diálogos de plugin y workflows SHALL ser overlays ordinarios, no
slots con prioridades simultáneas. Una nueva solicitud ordinaria autorizada y
vigente SHALL reemplazar el overlay ordinario actual; la rechazada o revocada
SHALL NOT hacerlo. Reemplazar Settings SHALL descartar su copia editable y
revocar el lease; antes de iniciar Apply SHALL NOT escribir. Si la escritura ya
inició, reemplazar SHALL NOT iniciar otra ni garantizar rollback, y su receipt
SHALL cumplir REQ-07 sin alterar el overlay sustituto. Approval SHALL preemptar Settings y cualquier otro overlay;
ningún overlay ordinario SHALL reemplazar Approval pendiente. Al resolver o
cerrar Approval, Settings SHALL NOT restaurarse automáticamente.

Anteriormente: la cadena `Modal > Settings > PluginDialog > completion`
contradecía el escenario que reemplazaba Settings por un diálogo de plugin.
Esta sustitución conserva ese reemplazo autorizado sin rebajar Approval.

#### Scenario: SET-12-A — Approval preempta Settings

- GIVEN `Settings` abierto en `sound` con cambios sin guardar y sin Apply iniciado
- WHEN el host solicita Approval para una herramienta
- THEN se descarta Settings sin persistir y Approval ocupa el foco y render
- AND al resolver Approval no reaparece Settings

#### Scenario: SET-12-B — Diálogo autorizado reemplaza Settings

- GIVEN `Settings` abierto en `theme` sin Apply iniciado
- WHEN un comando vigente autorizado devuelve un resultado `Dialog`
- THEN se descarta Settings sin persistir y se renderiza el diálogo del plugin
- AND entrada capturada bajo el lease anterior no llega al nuevo diálogo

#### Scenario: SET-12-C — Solicitud rechazada no reemplaza

- GIVEN `Settings` abierto con una copia modificada, o Approval pendiente en otro caso
- WHEN llega una solicitud ordinaria revocada, o una ordinaria mientras Approval está pendiente, respectivamente
- THEN el overlay vigente conserva foco y contenido
- AND la solicitud rechazada no inicia un guardado ni concede permiso, aunque un Apply anterior pueda completar su escritura

#### Scenario: SET-12-D — Approval durante Apply y receipt tardío

- GIVEN un Apply con escritura ya iniciada y pendiente bajo una barrera controlada
- WHEN el host abre Approval y después llega el receipt con éxito o error, en casos separados
- THEN Approval preempta inmediatamente Settings y revoca su lease sin iniciar otra escritura
- AND el receipt no cierra, resuelve ni modifica Approval y no restaura Settings ni su foco
- AND solo una aceptación explícita válida de Approval puede conceder permiso
- AND el éxito actualiza el snapshot guardado para reapertura posterior; el error conserva el previo
- AND ninguna cancelación afirma deshacer la escritura iniciada ni publica la copia como conversación
