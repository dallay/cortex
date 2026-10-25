# huginn-status-bar — Especificación

## Propósito

Mostrar selección y estado reales de Huginn sin confundir configuración local
con disponibilidad remota.

## Requirements

### Requirement: BAR-01 — Barra persistente

La TUI MUST mostrar una barra compacta en cada frame, con y sin overlay,
incluida `Approval`. MUST mostrar proveedor, modelo y modo actuales o su ausencia
explícita. Un overlay MUST NOT ocultarla ni sustituirla por contenido conversacional.
En terminales estrechos MAY abreviar etiquetas sin inventar valores ni estado.

#### Scenario: BAR-01-A — QA con todos los overlays

- GIVEN una terminal con espacio para una línea de barra y una selección conocida
- WHEN se renderiza sin overlay y con cada variante de `Overlay`, incluida `Approval`
- THEN la barra sigue visible y muestra proveedor, modelo y modo correspondientes
- AND cerrar el overlay no añade esos valores a conversación

### Requirement: BAR-02 — Snapshot inicial y actualizaciones coherentes

El core MUST definir `Event::ConnectionChanged`; el servicio de conexiones MUST
publicar la selección confirmada de proveedor/modelo/modo y aportar un snapshot
inicial al conectar la UI. La barra MUST converger a la selección vigente sin
depender de abrir `/providers`. Cada actualización MUST aplicarse como una unidad
coherente. Resultados asíncronos obsoletos, duplicados o de una conexión/generación
anterior MUST NOT sobrescribir la selección más reciente. Fallos o cancelaciones
previos a confirmar un cambio MUST NOT publicar una selección ficticia.

#### Scenario: BAR-02-A — Arranque y ausencia de selección

- GIVEN una selección persistida utilizable, o ninguna selección en un caso separado
- WHEN se conecta la UI y recibe el snapshot inicial
- THEN la barra muestra la selección efectiva o ausencia explícita sin interacción adicional
- AND un cambio confirmado posterior actualiza conjuntamente proveedor, modelo y modo

#### Scenario: BAR-02-B — Actualización fuera de orden

- GIVEN cambios confirmados A y después B y la barra ya mostrando B
- WHEN llega tarde una actualización de A o un snapshot inicial anterior a B
- THEN la barra conserva B sin mezclar proveedor de A y modelo/modo de B
- AND una duplicación de B no produce avisos ni conversación adicional

#### Scenario: BAR-02-C — Fallo, revocación y reconexión

- GIVEN una selección A y un intento de cambiar a B
- WHEN falla o se cancela B, o se revoca A, en casos separados
- THEN el fallo/cancelación no anuncia B como selección confirmada
- AND la revocación refleja la selección efectiva restante o su ausencia
- AND tras reconectar la UI, eventos de la generación anterior no revierten el snapshot vigente

### Requirement: BAR-03 — Evidencia de conexión, no inferencia local

Seleccionar proveedor/modelo o recuperar credenciales MUST NOT bastar para mostrar
`Connected` ni equivalente de disponibilidad remota verificada. La UI MUST
distinguir selección/preparación local de evidencia remota. Un éxito remoto MAY
describirse como tal únicamente para la conexión y operación verificadas; cambiar
selección, revocar credenciales o recibir un fallo MUST NOT conservar una afirmación
engañosa de conexión vigente. La barra MUST reflejar actividad, aprobación pendiente
y errores sin convertirlos en mensajes de conversación.

#### Scenario: BAR-03-A — QA sin evidencia remota

- GIVEN un proveedor/modelo seleccionado localmente sin llamadas remotas satisfactorias
- WHEN se abre la TUI o se completa `/model`
- THEN se muestra la selección local sin `Connected` ni otra promesa de disponibilidad remota
- AND un modelo candidato no se describe como descubierto o validado remotamente

#### Scenario: BAR-03-B — Evidencia obsoleta y error

- GIVEN evidencia remota satisfactoria de A
- WHEN se selecciona B sin verificar, se revoca A o falla una operación remota vigente, en casos separados
- THEN la barra no atribuye a la selección vigente la disponibilidad previa de A
- AND el error o estado pendiente sigue observable incluso con un overlay abierto
