# huginn-ui-events — Especificación

## Propósito

Separar conversación y superficies UI durante emisión, proyección y recuperación.
Esta capacidad es nueva; `proposal.md` prevalece sobre sugerencias de exploración.

## Requirements

### Requirement: EVT-01 — Clasificación explícita antes de deduplicar

El sistema MUST distinguir `Conversation`, `Overlay`, `Notification`, `Status`
y `Diagnostic` mediante métodos explícitos por categoría, sin `notice` genérico
ni clasificación por prefijos. Solamente `Conversation` MUST alimentar
`conversation`. Ningún evento UI MUST convertirse en conversación por repetición,
expiración de dedup o sincronización. Avisos y diagnósticos MUST ser observables
en superficies no conversacionales, sin una capa de toasts.

#### Scenario: EVT-01-A — QA de apertura y cierre sin contaminación

- GIVEN una conversación cuyo contenido y número de entradas se han registrado
- WHEN se abre y cierra cada uno de `/help`, `/providers`, `/login`, `/model` y `/settings`, y se repite después de más de 2 segundos
- THEN el contenido y número de entradas de conversación no cambian
- AND ayuda, formularios, resultados UI y avisos se muestran fuera de conversación
- AND estos comandos no se envían al modelo como prompts

#### Scenario: EVT-01-B — Clasificación independiente del texto

- GIVEN el mismo texto emitido explícitamente como `Conversation`, `Notification`, `Status` y `Diagnostic`
- WHEN se proyectan las cuatro emisiones
- THEN solo la emisión `Conversation` se añade a conversación
- AND los prefijos del texto no alteran la categoría

### Requirement: EVT-02 — Conversación y persistencia conservadas

El sistema MUST renombrar `committed` a `conversation` y conservar mensajes
de usuario/agente y resultados relevantes de herramientas, incluidos errores
de herramientas que constituyan resultados conversacionales. MUST NOT deduplicar
mensajes conversacionales legítimos. El streaming y la sincronización MUST
conservar orden, contenido y ausencia de duplicados por reproyección. El límite
del buffer UI MUST NOT truncar la sesión persistida ni su recuperación.

#### Scenario: EVT-02-A — Repetición legítima y streaming

- GIVEN dos mensajes legítimos idénticos en un intervalo inferior a 2 segundos y una respuesta en streaming
- WHEN termina la respuesta y se sincroniza de nuevo la misma sesión
- THEN permanecen ambos mensajes en orden y una sola respuesta completa
- AND el resultado relevante de una herramienta permanece en conversación, no convertido en aviso UI

#### Scenario: EVT-02-B — Límite exacto y recuperación

- GIVEN una sesión con más de 256 eventos persistidos y un buffer UI vacío
- WHEN se incorporan 256 eventos retenibles y después uno más
- THEN el buffer UI contiene exactamente 256 eventos, los más recientes en orden
- AND el evento más antiguo sale únicamente del buffer UI
- AND reabrir la sesión permite recuperar todos los mensajes y resultados persistidos, incluidos los anteriores al límite

### Requirement: EVT-03 — Errores deduplicados por operación

El sistema MUST deduplicar errores UI usando hash, igualdad del mensaje e identidad
de operación. El hash solo MUST NOT determinar igualdad. Con reloj controlable,
un error igual de la misma operación MUST suprimirse durante los 2 segundos
posteriores a su última emisión visible; a los 2 segundos exactos MUST poder
reaparecer. Suprimir MUST NOT prolongar esa ventana. Operaciones independientes
MUST NOT suprimirse mutuamente; una nueva invocación MUST tener identidad distinta.
Un error suprimido MUST NOT producir otro aviso equivalente en una superficie UI
distinta. Esta regla MUST NOT eliminar resultados conversacionales.

#### Scenario: EVT-03-A — Fronteras temporales sin esperas reales

- GIVEN un error visible de la operación A en t=0 y un reloj controlado
- WHEN el mismo error de A llega en t=1,999 segundos y en t=2 segundos
- THEN la primera repetición no genera una segunda presentación del error
- AND la segunda se presenta una vez fuera de conversación
- AND la repetición suprimida no ha desplazado el vencimiento

#### Scenario: EVT-03-B — Operaciones distintas e intercaladas

- GIVEN un error M de A visible en t=0
- WHEN en t=0,5 llega M de B y en t=1 llega M de A
- THEN M de B se presenta porque B es independiente
- AND M de A sigue suprimido aunque se haya intercalado B

#### Scenario: EVT-03-C — Colisión y mensaje distinto

- GIVEN dos mensajes distintos con el mismo hash dentro de una misma operación
- WHEN ambos se emiten en menos de 2 segundos
- THEN ambos se presentan, pues su contenido no es igual

### Requirement: EVT-04 — Retención UI y dedup acotadas

El buffer UI MUST retener como máximo 256 eventos y expulsar primero los más
antiguos. El estado de dedup MUST tener un límite finito documentado de entradas
y de tamaño retenido, liberar operaciones finalizadas y registros expirados,
y no conservar secretos. Agotar ese límite MUST NOT ocultar errores nuevos:
la expulsión MAY permitir mostrar de nuevo un error previamente visto.
El límite concreto de dedup queda al diseño, no la obligación de acotarlo.

#### Scenario: EVT-04-A — Saturación y expiración

- GIVEN el límite documentado L de dedup y un reloj controlado
- WHEN llegan errores de L+1 identidades y después expiran sus ventanas o finalizan sus operaciones
- THEN la retención nunca supera el límite documentado
- AND cada error nuevo sigue siendo observable
- AND los registros expirados o finalizados se liberan sin alterar la sesión persistida
