# Especificación: autonomía y permisos de workspace de Huginn

## Estado y decisiones pendientes

El usuario ratificó el catálogo de lectura, listado, búsqueda, creación y edición ordinaria dentro del workspace; su activación por defecto en TUI, `run` y `--json`; los límites para efectos sensibles, destructivos no clasificados, externos, fuera del workspace, shell y MCP; los grants de sesión exactos y volátiles; y la selección por ratón en todos los menús interactivos. `write_file` solo es autónomo al crear un archivo nuevo; reemplazar un archivo existente requiere aprobación. Los detalles de implementación no pueden ampliar este contrato.

## Requisitos

### Requisito: Autoridad y datos confiables de autorización

El host MUST tomar la decisión efectiva de autorización en `HostPolicy`. La elegibilidad para autonomía MUST basarse en la identidad de acción y los metadatos de efecto/recurso establecidos o verificados por el host. El sistema MUST NOT conceder autonomía por afirmaciones del modelo, parámetros no verificados, metadatos aportados por una herramienta/plugin, hooks, ubicación aparente o presentación de la interfaz. La interfaz MUST limitarse a las decisiones que la política del host permite.

#### Escenario: Acción nativa clasificada por el host

- GIVEN que el host identifica una acción nativa registrada y su efecto y recurso verificables
- WHEN `HostPolicy` evalúa la solicitud
- THEN la política aplica la regla correspondiente a esa identidad y clasificación verificadas
- AND la interfaz no puede ampliar las decisiones disponibles

#### Escenario: Clasificación o procedencia no confiable

- GIVEN que una acción, efecto, recurso o metadato no está clasificado o no tiene procedencia verificable por el host
- WHEN se solicita autorización
- THEN el sistema MUST requerir aprobación explícita si hay aprobador disponible, o denegar/fallar cerrado si no lo hay
- AND MUST NOT inferir que la acción es ordinaria por su nombre, ruta aparente o declaración de un plugin/modelo

#### Escenario: Hook intenta conceder autorización

- GIVEN que un hook de herramienta bloquea, anota o devuelve datos sobre una solicitud
- WHEN `HostPolicy` toma la decisión
- THEN el hook MUST NOT otorgar un grant ni alterar la autoridad de autorización

### Requisito: Autonomía limitada a efectos nativos ordinarios del workspace

El host MUST autorizar sin prompt únicamente las acciones nativas incluidas en una lista cerrada y clasificadas como ordinarias, cuando el recurso esté canónicamente dentro del workspace activo, no sea sensible y la preparación y revalidación hayan tenido éxito. Ninguna categoría fuera de esa lista MUST heredar autonomía.

El catálogo ratificado incluye lecturas, listados y búsquedas nativas dentro del workspace canónico, no sensibles; `write_file` solo al crear una ruta nueva; y `edit_file` al modificar un archivo ordinario existente mediante su edición puntual. El reemplazo de un archivo existente con `write_file`, el borrado, movimiento, rename y otros efectos destructivos requieren aprobación explícita. Los casos no verificables quedan fuera del catálogo.

#### Escenario: Lectura nativa ordinaria

- GIVEN una lectura, listado o búsqueda nativa autorizada por el catálogo ratificado
- AND el recurso se resuelve dentro del workspace canónico y no es sensible
- WHEN el host evalúa la acción
- THEN la acción puede continuar sin solicitud de aprobación

#### Escenario: Escritura nativa incluida en el catálogo

- GIVEN una creación o modificación nativa que el catálogo ratificado clasifica como ordinaria
- AND el recurso canónico está dentro del workspace y no es sensible
- AND la propuesta preparada sigue vigente
- WHEN el host autoriza el efecto tras revalidarlo
- THEN el efecto puede continuar sin prompt
- AND la autorización no cubre otros recursos ni acciones

#### Escenario: Efecto no incluido en catálogo cerrado

- GIVEN que la acción nativa modifica, elimina, reemplaza, mueve o renombra un recurso y no está expresamente clasificada como autónoma
- WHEN se solicita ejecutar el efecto
- THEN MUST requerir aprobación explícita o denegarse de forma segura
- AND MUST NOT tratar un efecto destructivo o de impacto no revisable como ordinario por defecto

### Requisito: Efectos que conservan aprobación o denegación segura

El sistema MUST requerir aprobación explícita para efectos sensibles, destructivos no clasificados como ordinarios, externos, no clasificables o fuera del workspace. Si la aprobación no está disponible, el sistema MUST denegar/fallar cerrado y MUST NOT ejecutar el efecto. Shell y cada llamada MCP MUST seguir requiriendo aprobación por acción o ser denegados si no hay aprobador; aprobación para iniciar MCP MUST NOT autorizar sus llamadas posteriores. Shell, MCP, plugins y procesos hijos MUST NOT considerarse sandboxed por el workspace, guardrails, autorización o interfaz.

#### Escenario: Recurso sensible

- GIVEN que el host clasifica el recurso como sensible
- WHEN una acción solicita leerlo o modificarlo
- THEN la acción MUST quedar fuera de la autonomía ordinaria
- AND solo puede continuar con una decisión de aprobación permitida por el host

#### Escenario: Acción fuera del workspace o externa

- GIVEN que el recurso está fuera del workspace canónico, o que el efecto interactúa con un sistema externo
- WHEN se solicita el efecto
- THEN el sistema MUST solicitar aprobación explícita o denegar/fallar cerrado si no hay aprobador
- AND MUST NOT conceder autonomía por el directorio de trabajo o por una ruta de entrada dentro del workspace

#### Escenario: Shell sin sandbox

- GIVEN una solicitud para ejecutar shell, aunque su directorio de trabajo esté dentro del workspace
- WHEN el host evalúa la solicitud
- THEN MUST solicitar aprobación explícita para esa ejecución o denegarla si no hay aprobador
- AND MUST informar que el shell no está sandboxed y puede afectar recursos externos al workspace

#### Escenario: Llamadas MCP

- GIVEN que una conexión MCP fue aprobada para iniciarse
- WHEN se solicita una llamada MCP
- THEN la llamada MUST obtener aprobación propia o ser denegada si no hay aprobador
- AND el consentimiento de inicio MUST NOT servir como grant para la llamada
- AND el sistema MUST NOT presentar MCP como sandboxed

#### Escenario: Sin aprobador en modo no interactivo

- GIVEN que una acción requiere aprobación y el modo actual no dispone de un aprobador interactivo
- WHEN se intenta ejecutar la acción
- THEN el sistema MUST denegar/fallar cerrado sin ejecutar ningún efecto
- AND MUST producir un resultado o error observable que indique que falta aprobación

### Requisito: Preparación, autorización y revalidación contra TOCTOU

El sistema MUST separar la preparación del efecto de su autorización y ejecución. Antes de ejecutar una mutación, el host MUST revalidar inmediatamente la identidad del recurso, su pertenencia al workspace, su sensibilidad y la vigencia de la propuesta/contenido. Si una ruta, symlink, contenido, clasificación o workspace cambia, deja de ser verificable o difiere de lo revisado, el sistema MUST detener el efecto y solicitar una revisión/aprobación nueva cuando corresponda, o fallar cerrado. MUST NOT ejecutar una propuesta obsoleta.

#### Escenario: Propuesta sin cambios

- GIVEN que una mutación fue preparada y clasificada
- AND que su ruta canónica, contenido, workspace y clasificación coinciden con la propuesta autorizada
- WHEN se revalidan inmediatamente antes de la ejecución
- THEN la mutación puede ejecutarse conforme a la decisión de autorización vigente

#### Escenario: Cambio de ruta, symlink o workspace

- GIVEN que después de preparar una mutación cambia la resolución de la ruta, un symlink permite escapar del workspace o cambia el workspace activo
- WHEN el sistema revalida antes de ejecutar
- THEN MUST detener la mutación
- AND MUST solicitar nueva revisión/aprobación o denegar de forma segura

#### Escenario: Contenido obsoleto o error de canonicalización

- GIVEN que el contenido cambió desde su revisión, o la ruta no puede canonicalizarse/verificarse
- WHEN el sistema revalida
- THEN MUST NOT ejecutar la mutación
- AND MUST devolver un estado observable de propuesta obsoleta o recurso no verificable

### Requisito: alcance y duración de concesiones de sesión

Todo grant de sesión que permita un efecto MUST estar limitado a identidad de sesión, acción exacta y recurso canónico exacto, sin comodines ni ampliación implícita. MUST NOT autorizar recursos sensibles ni persistirse en disco, historial, almacenamiento de sesión o reanudación de proceso. Un cambio de sesión, acción, recurso o workspace MUST invalidar la coincidencia del grant. La semántica final respecto de `ApproveForSession`, políticas clonadas y `--allow` requiere una decisión de diseño explícita.

El usuario ratificó grants volátiles en memoria, limitados a identidad de sesión, acción exacta y recurso canónico exacto; no se restauran al reiniciar o reanudar. Los grants no aplican a recursos sensibles, shell/MCP, recursos externos ni acciones excluidas del catálogo.

#### Escenario: Grant coincide con su scope

- GIVEN un grant válido en memoria para una sesión, acción y recurso canónico exactos
- WHEN la misma sesión solicita esa acción para ese recurso no sensible
- THEN el grant puede satisfacer únicamente esa coincidencia, sujeto a revalidación y autoridad de `HostPolicy`

#### Escenario: Grant no coincide o cambia el workspace

- GIVEN un grant para otra sesión, acción, recurso o workspace
- WHEN se solicita el efecto
- THEN el grant MUST NOT autorizarlo
- AND el host MUST aplicar la regla ordinaria de aprobación o denegación segura

#### Escenario: Reinicio o reanudación

- GIVEN que existía un grant antes de cerrar o reanudar el proceso
- WHEN la nueva ejecución evalúa una acción
- THEN el grant anterior MUST NOT estar disponible
- AND el efecto MUST evaluarse con la política vigente y su configuración actual

### Requisito: Configuración y defaults según modo de ejecución

El sistema MUST definir defaults y precedencia de configuración para modos interactivos y no interactivos. Ninguna configuración, argumento CLI o `--allow` MUST convertir shell/MCP en operaciones sandboxed ni autorizar efectos externos, sensibles, destructivos no clasificados o no verificables por ubicación aparente. En cualquier modo sin aprobador disponible, una solicitud que requiera aprobación MUST fallar cerrada.

El usuario ratificó el mismo catálogo autónomo por defecto en TUI, `run` y `--json`. Los efectos excluidos que requieren aprobación deben fallar cerrados si no existe un aprobador disponible. No se requiere una opción de activación independiente para el catálogo ratificado. Toda configuración o override futuro debe conservar las exclusiones y la autoridad de `HostPolicy`.

#### Escenario: Modo interactivo configurado

- GIVEN que el usuario habilita explícitamente autonomía para un modo interactivo
- AND las decisiones de catálogo y precedencia se han ratificado
- WHEN una acción cumple exactamente los criterios autónomos ratificados
- THEN puede continuar sin prompt
- AND los efectos excluidos siguen requiriendo aprobación o denegación

#### Escenario: Modo no interactivo

- GIVEN `run`, `--json` u otro modo sin aprobación interactiva disponible
- WHEN ocurre una acción que requiere aprobación
- THEN el sistema MUST fallar cerrado sin ejecutar el efecto
- AND MUST exponer de forma verificable que la aprobación no estaba disponible

#### Escenario: Overrides no alteran clases prohibidas

- GIVEN que el usuario usa configuración, un override CLI o `--allow`
- WHEN la acción es shell, MCP, sensible, externa, destructiva no clasificada, fuera del workspace o desconocida
- THEN el override MUST NOT conceder autonomía por sí solo
- AND se aplica aprobación explícita o denegación segura

### Requisito: Selección de ratón y teclado en todos los menús interactivos de TUI

Todos los menús interactivos seleccionables de la TUI MUST permitir seleccionar con ratón y teclado, incluidos settings, selección de proveedor/modelo, workflows, diálogos de plugins, ayuda, aprobación y cualquier selector/completion que se presente como menú. La entrada MUST usar hit-testing coherente con el contenido visible, preservar foco y navegación de teclado, y definir el resultado de clic en filas, botones, áreas desplazables, espacios vacíos, bordes y clic fuera. Resize MUST recalcular la geometría de interacción sin dejar objetivos desplazados respecto de lo dibujado. Clic MUST NOT saltarse una revisión obligatoria, confirmar accidentalmente una acción protegida ni aprobar una solicitud obsoleta.

#### Escenario: Selección equivalente por ratón y teclado

- GIVEN un menú interactivo visible con una opción seleccionable
- WHEN el usuario la activa con clic primario o con la ruta de teclado disponible
- THEN ambas entradas seleccionan la misma opción, salvo controles que requieren confirmación protegida
- AND la ruta de teclado, foco y cancelación existentes siguen disponibles

#### Escenario: Clic fuera de un objetivo

- GIVEN que el usuario hace clic fuera de una fila, botón u objetivo interactivo válido
- WHEN el menú procesa el evento
- THEN el evento MUST NOT activar una opción ni confirmar/aprobar una acción
- AND la selección y el foco MUST permanecer en un estado válido

#### Escenario: Resize y hit-testing

- GIVEN un menú abierto cuya terminal cambia de tamaño
- WHEN se renderiza y procesa una entrada posterior
- THEN los objetivos de clic MUST corresponder a la geometría visible actual
- AND un clic sobre una posición que ya no contiene una opción MUST NOT seleccionar la posición anterior

#### Escenario: Aprobación protegida no se omite por clic

- GIVEN una solicitud que requiere revisar un diff, comando u otro detalle antes de aprobar
- WHEN se hace clic en una acción de aprobación antes de completar la revisión requerida o sobre una solicitud obsoleta
- THEN el sistema MUST NOT aprobar ni ejecutar el efecto
- AND MUST conservar la oportunidad de revisar o cancelar

### Requisito: Diálogo de aprobación, revisión y feedback de no-op

El diálogo MUST presentar con jerarquía legible la acción, el recurso, el riesgo y la vista previa íntegra requerida por la clase de efecto. MUST exponer solo las decisiones permitidas por el host. El estado de revisión MUST distinguir una vista previa incompleta/recortada de una revisada íntegramente, especialmente para detalles de shell; terminal estrecha o resize MUST NOT ocultar contenido mientras comunica falsamente que fue revisado. Si la presentación de aprobación está ausente, revocada o no puede producir una decisión válida, el diálogo/flujo MUST mostrar feedback claro de que no se ejecutará el efecto y ofrecer una salida segura. Una decisión sin efecto (no-op), rechazo o cancelación MUST generar feedback visible y un estado de vista/revisión coherente; no debe parecer que la acción se ejecutó.

#### Escenario: Solicitud revisable completa

- GIVEN que existe una solicitud de aprobación válida
- WHEN se presenta el diálogo
- THEN se muestran acción, recurso, riesgo y la vista previa completa requerida
- AND se indican únicamente las decisiones permitidas por el host
- AND la aprobación solo queda disponible tras satisfacer la revisión requerida

#### Escenario: Vista previa en terminal estrecha o redimensionada

- GIVEN que la vista previa no cabe íntegra en el espacio disponible
- WHEN el diálogo se renderiza o la terminal cambia de tamaño
- THEN el usuario puede navegar/revisar la vista previa completa
- AND el diálogo MUST identificar claramente contenido pendiente y MUST NOT indicar revisión completa antes de tiempo

#### Escenario: Aprobador ausente o presentación revocada

- GIVEN que no hay canal/aprobador interactivo disponible o que la presentación se revoca antes de decidir
- WHEN un efecto requiere aprobación
- THEN el diálogo o flujo MUST comunicar que no se ejecutará por falta de aprobación
- AND MUST ofrecer rechazo/cancelación o retorno seguro, sin ejecutar el efecto

#### Escenario: Rechazo, cancelación o decisión sin efecto

- GIVEN que el usuario rechaza, cancela o elige una decisión que no ejecuta el efecto
- WHEN el diálogo se cierra
- THEN el estado del resultado MUST indicar rechazo/cancelación/no-op según corresponda
- AND la vista/revisión MUST reflejar que el efecto no se ejecutó
- AND el flujo no debe informar éxito de ejecución

### Requisito: Conservación de rechazo, cancelación y fail-closed

El sistema MUST conservar las decisiones de rechazo/cancelación como resultados distintos de aprobación y MUST NOT convertirlas en aprobación por defecto, timeout, ausencia de UI, error de terminal, error de clasificación o fallo de revalidación. Todo fallo previo a la ejecución que impida confirmar autorización, scope o vigencia MUST impedir el efecto. La UI MUST mantener una acción de cancelación/rechazo accesible por teclado, incluida la ruta Escape cuando sea compatible con el diálogo vigente.

#### Escenario: Usuario rechaza o cancela

- GIVEN una solicitud que espera aprobación
- WHEN el usuario la rechaza o cancela por teclado o ratón
- THEN el efecto MUST NOT ejecutarse
- AND el resultado MUST conservar la decisión exacta sin reinterpretarla como aprobación

#### Escenario: Error durante autorización o revalidación

- GIVEN un error al clasificar, verificar permisos, canonicalizar o revalidar el recurso
- WHEN el flujo llega al límite de ejecución
- THEN el sistema MUST fallar cerrado sin realizar el efecto
- AND MUST devolver un estado de error observable

#### Escenario: Timeout o pérdida de la interfaz

- GIVEN que la interfaz o el canal de aprobación deja de responder antes de una decisión válida
- WHEN vence el flujo o se detecta la pérdida
- THEN MUST tratarse como ausencia de aprobación
- AND MUST NOT ejecutar el efecto

### Requisito: Verificación determinista de permisos e interacción

La cobertura de pruebas MUST verificar comportamiento observable mediante pruebas unitarias deterministas de clasificación, autorización y grants; pruebas de integración del flujo de preparación/autorización/ejecución y modos; y pruebas de presentación/PTY para interacción con ratón y teclado. Las pruebas MUST incluir casos adversariales y estados de error, y MUST evitar `sleep` o expectativas temporales no deterministas.

#### Escenario: Cobertura unitaria de política y grants

- GIVEN el conjunto de clases, recursos y estados de sesión aplicables
- WHEN se ejecutan las pruebas unitarias de autorización
- THEN se verifican acciones exactas, recursos exactos, sesión distinta, cambio de workspace, sensibilidad, clasificación desconocida, reinicio/reanudación y ausencia de grant
- AND los resultados no dependen de pausas temporales

#### Escenario: Cobertura de integración del límite de ejecución

- GIVEN acciones autónomas elegibles y acciones que requieren aprobación
- WHEN se ejercita el loop de aplicación con aprobación presente, cancelada, revocada o ausente
- THEN las pruebas verifican que solo los efectos autorizados se ejecutan
- AND que preparación obsoleta, escapes/symlinks, contenido cambiado y aprobador ausente no producen efectos

#### Escenario: Cobertura TUI/PTY

- GIVEN menús interactivos y diálogos de aprobación en terminal
- WHEN las pruebas inyectan entradas de ratón y teclado, clic fuera, cancelación, resize y terminal estrecha
- THEN verifican hit-testing, foco, revisión, feedback de no-op y preservación de rechazo/cancelación
- AND no usan `sleep` ni dependen de timing no determinista
