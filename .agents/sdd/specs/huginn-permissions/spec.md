# Delta Specification: Permisos autónomos del espacio de trabajo

## MODIFIED Requirements

### Requirement: Aislamiento de comandos de shell en macOS

En macOS, Huginn MUST ejecutar `run_shell` con aislamiento del sistema operativo activo por defecto. La política MUST denegar por defecto, permitir únicamente el acceso necesario al workspace y a los recursos requeridos para ejecutar el shell, y denegar el acceso a la red. El proceso shell y sus descendientes MUST permanecer sujetos a la misma política. Si el aislamiento no está disponible o no puede aplicarse, Huginn MUST impedir la ejecución y MUST NOT reintentarla sin aislamiento.

La política MUST generarse dinámicamente en formato SBPL e imponerse mediante `/usr/bin/sandbox-exec`. Los datos de rutas no confiables MUST NOT permitir ampliar la política.

El mecanismo depende de interfaces Seatbelt de Apple que están deprecadas. Huginn MUST comunicar esta limitación y MUST NOT presentar su disponibilidad o comportamiento futuro como garantizado ni como avalado por Apple. El comportamiento debe verificarse por separado en las versiones de macOS soportadas. Este requisito no declara soporte de aislamiento para Linux o Windows.

#### Scenario: Shell permitido dentro del workspace

- GIVEN `run_shell` solicita una operación dentro de los permisos efectivos del workspace
- WHEN Huginn ejecuta el comando en macOS
- THEN el comando se ejecuta bajo la política restrictiva activa
- AND los procesos que descienden del shell permanecen confinados por esa política

#### Scenario: Acceso fuera del workspace denegado

- GIVEN un comando intenta leer o modificar un archivo fuera de las rutas permitidas del workspace
- WHEN el proceso shell o uno de sus descendientes solicita el acceso
- THEN el sistema operativo deniega el acceso
- AND el comando no obtiene permisos adicionales por cambiar su directorio de trabajo

#### Scenario: Rutas manipuladas no amplían permisos

- GIVEN el workspace o una ruta solicitada contiene metacaracteres, secuencias con significado en SBPL o enlaces simbólicos hacia el exterior
- WHEN Huginn genera y aplica la política
- THEN la política no otorga acceso fuera de las rutas autorizadas
- AND Huginn deniega la ejecución si no puede determinar el alcance de forma segura

#### Scenario: Red y sockets denegados

- GIVEN un comando o descendiente intenta abrir una conexión de red o un socket
- WHEN la operación se evalúa bajo la política de shell
- THEN el acceso se deniega

#### Scenario: Descendiente intenta eludir la política

- GIVEN un comando inicia un descendiente o una ejecución alternativa
- WHEN ese proceso intenta acceder a archivos fuera del workspace o a la red
- THEN el sistema operativo deniega el acceso bajo la misma política

#### Scenario: Fallo del launcher o de la política

- GIVEN `/usr/bin/sandbox-exec` no está disponible, o la política no puede generarse, validarse o aplicarse
- WHEN Huginn recibe una solicitud `run_shell`
- THEN no inicia el comando
- AND devuelve un resultado observable de fallo
- AND no intenta ejecutar el comando sin aislamiento

#### Scenario: Interfaz Seatbelt deprecada

- GIVEN una versión de macOS cambia, restringe o elimina `sandbox-exec` o una interfaz Seatbelt requerida
- WHEN Huginn no puede confirmar y aplicar la política
- THEN Huginn informa que el aislamiento no está disponible
- AND no ejecuta el comando sin aislamiento como alternativa automática

#### Scenario: No se declara paridad en otras plataformas

- GIVEN `run_shell` se solicita en Linux o Windows
- WHEN Huginn aplica este cambio
- THEN este requisito no atribuye aislamiento a esa ejecución
- AND Huginn no declara que el comportamiento de esa plataforma sea equivalente al de macOS

### Requirement: Aprobación, política y aislamiento son controles independientes

Huginn MUST conservar las semánticas de política y aprobación de fase 1. Las reglas de usuario y globales, las restricciones administradas y las decisiones de aprobación determinan si una operación está autorizada; no conceden ni amplían el aislamiento del sistema operativo. Las restricciones administradas prevalecen y solo pueden restringir. `Deny` MUST impedir la operación, y `Allow once` MUST autorizar únicamente la operación presentada. Huginn MUST NOT introducir grants persistentes, ni cargarlos, crearlos u honrarlos. Huginn MUST NOT introducir modos YOLO, Exit o de acceso total.

#### Scenario: Aprobación ordinaria no retira el aislamiento

- GIVEN una operación de shell requiere aprobación y el usuario la concede
- WHEN Huginn ejecuta la operación
- THEN la aprobación se limita a la operación autorizada
- AND el comando permanece sujeto al aislamiento activo y a la política efectiva
- AND la aprobación ordinaria de `HostPolicy` no se interpreta como consentimiento separado para ejecutar sin sandbox
- AND Huginn no ofrece una configuración, exclusión ni bypass para ejecutar `run_shell` sin sandbox

#### Scenario: Restricción administrada prevalece

- GIVEN una operación permitida por una regla de usuario está denegada por una restricción administrada
- WHEN Huginn resuelve la operación
- THEN Huginn no la ejecuta
- AND la aprobación del usuario no anula la restricción administrada

#### Scenario: Denegación y aprobación de una sola vez

- GIVEN el usuario elige `Deny` o `Allow once` para una operación
- WHEN Huginn procesa la decisión
- THEN `Deny` impide esa operación y no se convierte en autorización
- AND `Allow once` no autoriza operaciones posteriores ni crea estado persistente

#### Scenario: No hay grants persistentes

- GIVEN un archivo de proyecto, configuración o estado previo contiene una solicitud o dato de grant persistente
- WHEN Huginn resuelve permisos para una operación
- THEN no crea, carga ni honra un grant persistente a partir de ese dato

### Requirement: El aislamiento de shell no cubre herramientas nativas ni MCP

Huginn MUST limitar la garantía de este aislamiento a los procesos iniciados por `run_shell` y sus descendientes. Las herramientas nativas de archivos MUST seguir sujetas a sus propios controles de política y autorización. Las llamadas MCP MUST conservar su política de aprobación específica. Huginn MUST NOT afirmar que esas operaciones están confinadas por el sandbox de shell.

#### Scenario: Herramienta nativa fuera de alcance

- GIVEN una herramienta nativa solicita una operación de archivo
- WHEN Huginn autoriza o deniega esa operación
- THEN aplica los controles propios de la herramienta nativa
- AND no usa el sandbox de `run_shell` como prueba de autorización o confinamiento

#### Scenario: Llamada MCP fuera de alcance

- GIVEN un servidor MCP recibe una llamada
- WHEN Huginn aplica la política de aprobación MCP
- THEN conserva las reglas específicas del servidor y la herramienta
- AND no atribuye a la llamada el aislamiento de `run_shell`

### Requirement: Ciclo de vida de `run_shell`

La aplicación del aislamiento MUST preservar las garantías existentes del ciclo de vida de `run_shell`: entorno seleccionado, directorio de trabajo, entrada y salida, timeout, cancelación y limpieza del grupo de procesos. El launcher y la política MUST NOT permitir que la cancelación o el timeout dejen procesos descendientes activos fuera del control de Huginn.

En cada salida controlada que retorna como `Result` —éxito, salida no-cero, rechazo o error de spawn, timeout y cancelación— el runtime MUST intentar el cleanup explícito de los recursos del launcher y MUST propagar cualquier error de cleanup. Si el shell pudo ejecutarse, un error de cleanup MUST NOT afirmar que el shell no se ejecutó. Si la preparación, creación o escritura de recursos falla antes de iniciar el launcher, Huginn MUST NOT iniciar el shell; MUST intentar cleanup explícito de los recursos ya creados y MUST hacer observables tanto el error primario como cualquier error de cleanup, sin ocultar este último.

Si se descarta abruptamente el future o su owner y solo se ejecuta `Drop`, el cleanup MUST ser de mejor esfuerzo. El contrato MUST NOT garantizar la eliminación del recurso si el sistema operativo falla. La escritura de recursos MUST completar mediante `write_all`. El cierre de `std::fs::File` no es un escenario exigido independiente: su error no es observable mediante la API safe de ese tipo. Esta limitación no reduce los requisitos de seguridad del launcher.

**Rationale:** `std::fs::File` no expone un error de cierre a través de su API safe, por lo que exigir o probar de forma independiente ese error no sería verificable. El runtime sí puede exigir `write_all`, cleanup explícito en salidas controladas y propagación de errores observables. La limitación de observabilidad del cierre no autoriza a omitir cleanup, relajar el launcher ni ejecutar sin sandbox.

#### Scenario: Ejecución conserva los parámetros seleccionados

- GIVEN una solicitud válida especifica entorno, directorio de trabajo e entrada
- WHEN Huginn ejecuta `run_shell` bajo aislamiento
- THEN el proceso recibe esos parámetros según la solicitud
- AND Huginn entrega la salida conforme al contrato existente de `run_shell`

#### Scenario: Timeout o cancelación limpia descendientes

- GIVEN un comando crea uno o más procesos descendientes
- WHEN la ejecución alcanza el timeout o se cancela
- THEN Huginn detiene el proceso shell y limpia su grupo de procesos según las garantías existentes
- AND ningún descendiente continúa ejecutándose como resultado de una limpieza incompleta

#### Scenario: Cleanup explícito en salidas controladas

- GIVEN el launcher ha sido preparado y `run_shell` termina con éxito, salida no-cero, rechazo, error de spawn, timeout o cancelación retornada como `Result`
- WHEN el runtime devuelve el resultado de `run_shell`
- THEN el runtime intenta cleanup explícito de los recursos del launcher
- AND propaga cualquier error de cleanup
- AND si el shell pudo ejecutarse, el error devuelto no afirma que el shell no se ejecutó

#### Scenario: Fallo de preparación antes del launcher

- GIVEN la preparación, creación o escritura de un recurso del launcher falla antes de iniciarlo
- WHEN Huginn procesa el fallo
- THEN no inicia el launcher ni el shell
- AND intenta cleanup explícito de los recursos ya creados
- AND el resultado hace observables el error primario y cualquier error de cleanup, sin ocultar el error de cleanup

#### Scenario: Drop abrupto usa cleanup de mejor esfuerzo

- GIVEN se descarta abruptamente el future o su owner antes de que `run_shell` retorne un `Result`
- WHEN solo puede ejecutarse el cleanup de `Drop`
- THEN Huginn intenta eliminar el recurso como mejor esfuerzo
- AND el contrato no garantiza la eliminación si falla el sistema operativo

#### Scenario: Escritura completa sin requisito independiente de error de cierre

- GIVEN Huginn escribe el recurso privado del launcher
- WHEN la escritura se completa
- THEN Huginn usa una operación que escribe todos los bytes o informa un error de escritura
- AND la conformidad no depende de inducir un error de cierre de `std::fs::File` mediante su API safe
