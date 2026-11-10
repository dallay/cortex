# Diseño: Permisos autónomos del espacio de trabajo de Huginn

## Enfoque técnico

En macOS, `run_shell` siempre debe preparar un perfil Seatbelt SBPL restrictivo y lanzar el comando mediante `/usr/bin/sandbox-exec`. El aislamiento es obligatorio, no configurable y no tiene exclusión ni bypass: si no se puede preparar o aplicar, la operación falla sin ejecutar el comando. El perfil parte de `(deny default)`, permite lectura/escritura solo en el workspace autorizado y una zona temporal privada por invocación si las pruebas demuestran que es necesaria; deniega red y sockets. El shell y los procesos que cree deben heredar el mismo sandbox.

La autorización de la operación sigue en `HostPolicy` y no cambia por la presencia del sandbox. `Deny` sigue denegando; `Allow once` solo aprueba esa invocación y nunca retira el confinamiento. `HostPolicy` no autoriza quitarlo. Este diseño adopta el mecanismo técnico Seatbelt de Codex, no su modelo de elevación, excepciones ni consentimiento para ejecutar fuera del sandbox. Cualquier posibilidad futura de elevar permisos requerirá un cambio separado y consentimiento independiente; no forma parte de este cambio. No se crean, leen ni honran grants persistentes. No se extiende la garantía a herramientas nativas ni a MCP, y no se afirma aislamiento en Linux o Windows. El mecanismo depende de interfaces Apple deprecadas; no se promete disponibilidad futura ni soporte uniforme entre versiones.

## Decisiones de arquitectura

### Decisión: Mantener el generador puro y el lanzamiento en el adaptador runtime

**Elección:** Añadir un módulo privado en `crates/huginn/runtime/src/` que convierta un contexto de ejecución validado en texto SBPL. El generador no realiza E/S ni ejecuta procesos; recibe raíces ya resueltas y devuelve `Result<ProfileText, ProfileError>`. El adaptador runtime prepara las rutas/contexto, crea y limpia recursos temporales e invoca `sandbox-exec`. La orquestación permanece en `run_shell`.

**Alternativas consideradas:** Poner generación SBPL, resolución de rutas y lanzamiento en una función extensa; añadir un port de sandbox a `huginn-core` o un crate dedicado.

**Justificación:** La política concreta es infraestructura dependiente de macOS y no es un contrato de dominio. Un generador pequeño y puro facilita pruebas adversariales sin simular la aplicación del sandbox. Un módulo privado evita una abstracción por plataforma que no se necesita hoy y conserva las dependencias hacia dentro conforme a ADR-0012.

### Decisión: Confinamiento obligatorio; sin ajuste ni bypass

**Elección:** En macOS, toda invocación de `run_shell` usa el sandbox. No se añade configuración, exclusión, modo opt-out ni bypass. Ninguna aprobación existente puede elegir ejecución sin confinamiento. Cualquier posibilidad futura de elevar permisos requiere un cambio separado y consentimiento independiente; no se define ni se implementa aquí.

**Alternativas consideradas:** Añadir `sandbox = false`; tratar la aprobación ordinaria de shell como consentimiento para ejecutar sin sandbox; importar el modelo de elevación de Codex.

**Justificación:** `HostPolicy` autoriza operaciones, pero no autoriza retirar el límite del sistema operativo. Reutilizar la aprobación existente sería un bypass silencioso. La spec vigente exige confinamiento siempre y excluye expresamente la ejecución sin sandbox; el diseño se alinea con ese contrato.

### Decisión: Entregar el perfil mediante archivo temporal privado y `-f`

**Elección:** Usar la interfaz de archivo de perfil (`sandbox-exec -f <profile-file>`) en lugar de pasar el SBPL completo con `-p`. Crear el archivo con nombre aleatorio en un directorio temporal privado de Huginn, permisos restrictivos (directorio `0700`, archivo `0600`, creación exclusiva sin seguir enlaces), conservarlo hasta terminar el launcher y solicitar cleanup explícito en cada salida controlada. Limitar tamaño antes de escribir y fallar si se excede.

**Alternativas consideradas:** `-p <texto>`; stdin como canal del perfil.

**Justificación:** `-p` expone el perfil a límites de longitud de argumentos del sistema y hace innecesario transportar potencialmente grande SBPL por argv; stdin no es un canal de perfil documentado en el contrato del cambio y compite con la semántica stdin nula del shell. El archivo tiene una vida útil acotada y permisos privados. En la prueba de macOS se verificará que `sandbox-exec` acepta `-f` en los runners objetivo. No se afirmará compatibilidad de versiones por documentación o generación aislada.

El archivo se crea y abre con APIs safe de archivo (sin rutas controladas por usuario), se escribe con `write_all` y se deja caer antes del lanzamiento. No se exige ni prueba un error independiente de `close`: `std::fs::File` no lo expone por API safe. Si falla la creación o escritura antes del spawn, no se inicia el launcher ni el shell; el runtime intenta cleanup explícito de lo ya creado y compone el error primario con cualquier error de cleanup. Si el launcher ya pudo iniciar, cada retorno controlado ejecuta cleanup explícito y propaga su error. El `Drop` del guard sigue intentando borrar archivo y directorio de mejor esfuerzo para cancelación abrupta del future/owner, sin garantía de eliminación ante un fallo del sistema operativo. El perfil solo permite al launcher leer el archivo en la fase de carga si las pruebas demuestran que esa lectura ocurre bajo restricciones; no se incorpora su directorio temporal como raíz general permitida.

Los errores compuestos deben distinguir la fase: para un fallo previo al spawn, el texto identifica el fallo primario y el fallo de cleanup, y puede afirmar que el launcher no se inició; para un resultado del launcher, conserva el resultado primario (incluidos timeout/cancelación) y agrega el error de cleanup sin afirmar que el shell no se ejecutó. La composición no usa un error de cleanup para reemplazar ni ocultar el error primario. Los errores actuales de `open` y escritura deben dejar de descartar los resultados de `remove_file`/`remove_dir`: se conserva el error primario, se intenta cleanup explícito y se adjunta cualquier fallo secundario antes de retornar. Un fallo al crear el directorio no tiene recursos creados que limpiar.

### Decisión: El grupo de procesos lo lidera `sandbox-exec`

**Elección:** Construir `tokio::process::Command` para el ejecutable absoluto `/usr/bin/sandbox-exec`, añadir `-f`, ruta de perfil y luego `/bin/sh -c <command>`, y aplicar `CommandWrap::wrap(ProcessGroup::leader()).wrap(KillOnDrop)` al proceso del launcher. Así el proceso que nace como líder del grupo es `sandbox-exec`; el shell y sus descendientes heredan tanto el grupo como el sandbox. La limpieza usa el wrapper `ChildWrapper` ya empleado por `run_shell`.

**Alternativas consideradas:** Seguir liderando el grupo con `/bin/sh` directamente; iniciar el launcher fuera del grupo y confiar en señales al PID.

**Justificación:** El proceso monitorizado directamente pasa a ser el launcher, por lo que el comportamiento de señales y `start_kill()` cambia respecto del código actual. No se considera probado que `process-wrap` mate a todos los descendientes solo por envolver el launcher. Una prueba macOS debe confirmar el proceso líder/grupo real y que timeout, cancelación y salida del launcher no dejan descendientes vivos. Si no se logra, detener el cambio; no debilitar el requisito.

### Decisión: Resolver raíces y serializar rutas de forma conservadora

**Elección:** Resolver el workspace una sola vez antes de generar el perfil, canonicalizarlo y exigir que exista, sea directorio y no cambie de identidad durante la preparación. Generar reglas de lectura y escritura separadas. Las rutas se emiten como literales SBPL mediante una única rutina de serialización probada, escapando comillas, barras inversas, caracteres de control y cualquier sintaxis con significado. No se interpolan rutas en texto de shell, regex SBPL ni fragmentos de política.

**Justificación y límites:** `canonicalize` no cierra carreras entre comprobar rutas y usarlas, ni protege por sí solo contra renombres/enlaces concurrentes. Las pruebas cubrirán enlaces simbólicos dentro y fuera del workspace, rutas con metacaracteres y cambios de destino. El diseño no afirmará resistencia a todas las carreras de pathname/FD que Seatbelt no permita demostrar. No se heredan descriptores arbitrarios; se configuran explícitamente stdio y no se usan `pre_exec` ni FDs de usuario como acceso indirecto. Ante raíz ambigua, no canonicalizable o fuera del alcance, se rechaza la ejecución.

No se permite escritura global en `/tmp`, home, preferencias, `/Applications` ni otros workspaces. El workspace completo es la única raíz persistente de escritura autorizada por este perfil; no se permite que `cwd` amplíe el ámbito. La zona temporal, si hace falta, se crea por ejecución y solo esa ruta exacta recibe permiso. La política no incluye `allow network*`, sockets locales, proxy ni servicios opcionales.

### Decisión: Lista de runtime mínima basada en evidencia de runner

**Elección:** Conservar `(deny default)` y añadir solo reglas necesarias para ejecutar `/bin/sh`, cargar dyld/bibliotecas del sistema y recursos runtime probados, hacer `fork/exec` bajo la misma política, usar `/dev/null`, conservar stdio ya conectado, acceder al `cwd` workspace y a la zona temporal privada designada. Las reglas de sistema serán de solo lectura y lo más acotadas posible. Cada permiso adicional requiere un caso de integración reproducible y revisión de que no amplía la frontera.

**Alternativas consideradas:** Copiar la política base amplia de Codex o conceder árboles globales de sistema/servicios por anticipado.

**Justificación:** Codex es referencia de mecanismo, no autorización para importar su allowlist. SBPL y los requisitos de dyld/runtime pueden variar por versión de macOS. El conjunto concreto se valida en los runners macOS bloqueantes; no se fija una afirmación de compatibilidad cruzada no respaldada por pruebas. Un fallo de una herramienta por una dependencia no autorizada se informa; no se resuelve abriendo permisos amplios sin evaluación.

**Evidencia y ajuste de implementación (macOS 27.3, 2026-11-09):** el perfil inicial, sin reglas de bootstrap dyld, terminaba en `SIGABRT` incluso para `/usr/bin/true` y `/bin/sh -c 'exit 0'` al invocarlo directamente con `sandbox-exec`. Importar `dyld-support.sb` antes de las reglas locales de denegación permitió iniciar esos procesos y, con las reglas existentes, permitió shell read/write en la raíz canónica mientras denegó un marcador externo. El archivo del sistema identifica sus reglas como necesarias para el bootstrap de dyld y se declara interfaz privada de Apple, sujeta a cambios. La implementación importa solo `dyld-support.sb`; no importa `system.sb`, no añade permisos globales de lectura ni permisos de red. La operación exacta que causa el aborto inicial no se aisló. Esta importación es un riesgo de compatibilidad explícito: disponibilidad, contenido y comportamiento se deben comprobar en cada runner macOS soportado; cualquier error mantiene `run_shell` cerrado. No se afirma
que la importación por sí sola pruebe confinamiento completo.

## Flujo de datos y comando

```text
ToolContext + command
  → validar/canonicalizar workspace y preparar contexto acotado
  → builder puro: contexto validado → SBPL deny-default
  → archivo privado temporal (-f; write_all antes del spawn)
  → CommandWrap(ProcessGroup::leader, KillOnDrop)
  → /usr/bin/sandbox-exec -f PROFILE /bin/sh -c COMMAND
  → proceso/grupo monitorizado, stdout/stderr acotados y drenados
  → cleanup explícito del perfil en todo retorno Result; Drop best-effort solo si el future/owner se descarta abruptamente
```

La invocación exacta usa argumentos separados, sin `sh -c` adicional para construir el launcher:

```text
argv[0] = "/usr/bin/sandbox-exec"
argv[1] = "-f"
argv[2] = <ruta-privada-del-perfil>
argv[3] = "/bin/sh"
argv[4] = "-c"
argv[5] = <comando original>
```

El `Command` del launcher establece `current_dir` en el workspace validado, `env_clear()`, restaura únicamente el `PATH` que el contrato actual selecciona, conecta stdin a `Stdio::null()` y stdout/stderr a pipes. El shell hereda esos valores del launcher. No se copian variables adicionales del entorno. Las tareas deben confirmar si cualquier otra variable forma parte del contrato actual antes de cambiarla.

El proceso launcher se construye con el ejecutable fijo absoluto; nunca se busca `sandbox-exec` por `PATH`. La cancelación del token y el timeout mantienen la selección actual alrededor de `wait()`. En ambas salidas controladas se solicita terminación del grupo y se espera/cierra la salida de lectores con límites ya existentes; cada retorno controlado —éxito, salida no-cero, rechazo, error de spawn, timeout o cancelación retornada— pasa luego por cleanup explícito del perfil. Un fallo de cleanup se propaga. Si también existe un error primario, ambos se reportan; en resultados posteriores al spawn el mensaje no afirma que el shell no se ejecutó. Solo el descarte abrupto antes de retornar `Result` depende de `Drop` best-effort.

## Interfaces / contratos mínimos

Tipos internos privados, nombres sujetos a la organización final del módulo:

```rust
struct SandboxRoots {
    workspace: PathBuf,       // canonical, existente, directorio
    invocation_tmp: PathBuf,  // exacto, privado, opcional solo si probado necesario
}

struct SandboxProfile(String); // SBPL ya validado; no expuesto a capas interiores

fn build_profile(roots: &SandboxRoots) -> Result<SandboxProfile, ProfileError>;
```

La capa de perfil es pura: no canonicaliza, no lee archivos, no ejecuta procesos y no decide aprobaciones. Runtime posee canonicalización, archivo temporal, launcher y ciclo de vida. No se añade trait/port al core, campo de configuración para opt-out ni permiso/grant nuevo.

## Archivos previstos

| Archivo                                                                | Acción                      | Descripción                                                                                                                                                                                                                                                                              |
|------------------------------------------------------------------------|-----------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `crates/huginn/runtime/src/tools.rs`                                   | Modificar                   | Sustituir el ejecutable directo `/bin/sh` por launcher SBPL macOS y preservar configuración stdio/env/cwd, salida acotada, timeout, cancelación y grupo. El texto de aprobación/definición ya no debe afirmar «not sandboxed» en macOS; distinguir explícitamente plataformas y límites. |
| `crates/huginn/runtime/src/seatbelt.rs` (o módulo privado equivalente) | Crear                       | Constructor puro de SBPL, escapes, límites de perfil y validación de raíces ya canonicalizadas. Solo compilar/usar en macOS cuando corresponda.                                                                                                                                          |
| `crates/huginn/runtime/src/lib.rs`                                     | Modificar si es necesario   | Declarar módulo privado según convenciones existentes.                                                                                                                                                                                                                                   |
| `crates/huginn/runtime/tests/`                                         | Crear/modificar             | Tests de unidad e integración macOS aislados del resto de pruebas.                                                                                                                                                                                                                       |
| `apps/huginn/src/config.rs`, `apps/huginn/config.example.toml`         | Sin cambio de configuración | No añadir setting, exclusión ni bypass de sandbox.                                                                                                                                                                                                                                       |
| `docs/huginn/README.md`, `docs/huginn/architecture.md`, ADR relevante  | Modificar                   | Documentar default en macOS, red denegada, fallos de compatibilidad, deprecación Apple, límites de plataforma y exclusiones nativas/MCP.                                                                                                                                                 |
| `.github/workflows/ci.yml`                                             | Modificar                   | Ejecutar las pruebas de integración de sandbox en runner macOS en eventos de PR y hacerlas bloqueantes. Aprovechar `test-multi` solo si se amplía su disparador PR sin alterar cobertura accidentalmente; en caso contrario, añadir job específico.                                      |
| `crates/huginn/runtime/Cargo.toml`                                     | Sin cambio esperado         | `process-wrap` ya es dependencia. No se prevé dependencia nueva; cualquier requisito nuevo necesita análisis de arquitectura.                                                                                                                                                            |

## Estrategia de pruebas y criterios de evidencia

La prueba de generación SBPL demuestra únicamente qué texto produce el builder; **no demuestra que macOS aplique el aislamiento**. La aceptación requiere pruebas reales en macOS con `/usr/bin/sandbox-exec`.

| Capa                                | Casos y método                                                                                                                                                                                                                                                                                                                                                                                               | Resultado requerido                                                                                                                                     |
|-------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------|
| Unitarias puras                     | Raíz normal, Unicode, comillas, barra inversa, saltos/control, paréntesis/comas y sintaxis SBPL; rutas excesivas; separación read/write; no aparición de reglas de red o raíces amplias. Casos adversariales comprueban que los datos nunca cierran/inyectan una regla.                                                                                                                                      | Salida válida o error; nunca ampliar permisos por serialización.                                                                                        |
| Runtime de rutas                    | Workspace inexistente, archivo en vez de directorio, canonicalización fallida, symlink a exterior, enlaces internos, rename/symlink adversarial durante preparación y FDs heredados.                                                                                                                                                                                                                         | Rechazo cuando no se puede determinar alcance seguro; sin asumir que canonicalize neutraliza TOCTOU.                                                    |
| Integración macOS obligatoria       | Bajo launcher real: lectura/escritura dentro workspace permitida; lectura/escritura de home/credenciales, otro workspace, rutas externas y temp no designado denegadas; cambio de `cwd` no amplía permiso; TCP/UDP, Unix sockets y otros sockets denegados. Probes con código de salida/control verifican que recurso/operación existen y distinguen denegación de error del harness.                        | Evidencia de enforcement del SO correlacionada con proceso y operación. Fallo o resultado ambiguo bloquea.                                              |
| Descendientes y grupo               | Shell crea nieto, lanza otro intérprete/ejecutable permitido y trata de acceder fuera/red; verificar sandbox heredado. En timeout/cancelación el hijo deja marcador si sigue vivo; confirmar que launcher líder y todo el grupo terminan, incluso si retienen pipes.                                                                                                                                         | Ningún descendiente sobrevive ni obtiene más permisos.                                                                                                  |
| Fallos cerrados y cleanup           | Launcher ausente/no ejecutable, perfil inválido, fallo de creación o escritura pre-spawn y rechazo del launcher. En fallo pre-spawn, verificar que no se ejecuta el shell y que el resultado conserva el primario y cualquier error de cleanup. Probar cleanup fallido directamente sobre `SeatbeltProfileFile` en tests privados; probar composición con helper privado puro, sin seam público ni `unsafe`. | Error observable; marcador ausente para fallos pre-spawn; cualquier error de cleanup se conserva. No se induce ni exige un error de cierre de `File`.   |
| Ciclo de vida y salidas controladas | cwd, PATH seleccionado, stdin nulo, stdout/stderr separados y límite de salida; éxito, salida no-cero, error de spawn, timeout y cancelación retornada. Verificar cleanup explícito en cada retorno y propagación cuando cleanup falla, con helper interno de composición testeado por separado; no añadir una interfaz pública o inyección de dependencias solo para forzar fallos.                         | Igual contrato de `run_shell`; los errores combinados preservan resultado primario y cleanup, sin afirmar que el shell no se ejecutó si pudo iniciarse. |
| Drop abrupto                        | Descartar el future/owner durante espera y confirmar solo el intento best-effort del `Drop` de recursos.                                                                                                                                                                                                                                                                                                     | No se exige eliminación exitosa ni propagación de error desde `Drop`; no se presenta como equivalente a una salida controlada.                          |
| CI/versiones                        | Job macOS bloqueante en PR; registrar versión del SO observada. Ejecutar en cada versión principal que el proyecto declare soportada, según runners disponibles.                                                                                                                                                                                                                                             | No marcar integración como aprobada si no corrió. No convertir fallo/ausencia en skip verde.                                                            |

Si el sistema operativo o `sandbox-exec` no está disponible en un entorno local, el test de integración puede producir un estado explícito `unsupported`/skip con razón visible; ese resultado no cuenta como aprobación y debe mantenerse distinguible de un pase. En CI macOS requerido, ausencia, error o cualquier skip de la integración por dependencia deben fallar el job. Tests unitarios pueden ejecutarse en otras plataformas, pero no sustituyen integración macOS. Las pruebas de runtime no deben resetear datos persistentes.

Las pruebas de la matriz se ejecutan por separado por versión principal de macOS elegida por el proyecto. Los resultados solo acreditan la versión y configuración observadas. No se afirma soporte futuro ni aval de Apple.

## Migración y despliegue

No hay migración de datos. El cambio de comportamiento en macOS es intencional: shell queda siempre confinado, por lo que comandos con dependencias externas, red o rutas ajenas al workspace pueden fallar. No se recuperará compatibilidad mediante una opción de exclusión o ejecución sin sandbox. Linux/Windows conservan su comportamiento existente sin que eso implique aislamiento. No hay grants persistentes que migrar. No se debe lanzar la implementación sin pruebas de integración macOS bloqueantes aprobadas; una ausencia o skip de esas pruebas bloquea el lanzamiento.

La política no construible, el ejecutable ausente o el fallo de aplicación de política producen error sin ejecutar shell. Si la matriz de macOS o las pruebas adversariales fallan, se detiene el lanzamiento y la operación permanece sin ejecutar (fail-closed), sin fallback automático. Retirar posteriormente el backend exige un cambio explícito y revisado; no se puede revertir silenciosamente el default a ejecución sin sandbox. Una reversión de código tampoco debe presentarse como equivalente a aislamiento: hasta que exista otra implementación validada, no se habilitará `run_shell` sin confinamiento.

## Riesgos, límites y decisiones pendientes

- `sandbox-exec` y las interfaces Seatbelt son deprecadas y pueden cambiar o desaparecer. Confirmar mediante pruebas reales en cada versión de macOS soportada; no hacer claims de compatibilidad cruzada.
- La allowlist de runtime es sensible a plataforma. Cualquier permiso añadido requiere evidencia y revisión; no copiar la lista amplia de Codex.
- Las políticas SBPL basadas en rutas no eliminan por sí solas carreras, alias o acceso por descriptores. Las pruebas cubren ataques representativos; no se proclama garantía más amplia que la evidenciada.
- La semántica de `ProcessGroup::leader()` sobre el launcher y `ChildWrapper::start_kill()` debe confirmarse experimentalmente. Integración fallida bloquea implementación.
- El cleanup de `Drop` no puede propagar fallos y es best-effort; solo los caminos que retornan `Result` pueden hacer cleanup explícito y propagar el error observado. El error de close de `std::fs::File` no es observable mediante API safe y no es una verificación separada.
- `HostPolicy` actual no autoriza quitar confinamiento. El cambio copia el mecanismo Seatbelt, no el modelo de elevación Codex; cualquier posible elevación futura requiere un cambio separado y consentimiento independiente. No existe bypass ni setting en este diseño.
- Este sandbox cubre solo `run_shell` y descendientes. No cubre herramientas nativas, MCP ni los demás procesos de Huginn.
- No hay claim de aislamiento para Linux/Windows, ni paridad entre plataformas.
- Las pruebas solo en la generación de SBPL no constituyen prueba de enforcement.

## Fuentes de implementación

- `crates/huginn/runtime/src/tools.rs::run_shell`: lanzamiento actual directo `/bin/sh -c`, workspace cwd, entorno con `PATH`, stdin nulo, pipes, `ProcessGroup::leader`, `KillOnDrop`, cancelación/timeout y drenaje acotado.
- `crates/huginn/runtime/Cargo.toml`: `process-wrap` ya disponible.
- `docs/huginn/architecture.md`; ADR-0006, ADR-0007 y ADR-0012.
- `exploration.md` del cambio: comparación de la implementación Codex y advertencias de Apple. Codex es referencia de patrón, no plantilla de permisos.

## Preguntas abiertas

- [ ] La matriz de macOS soportada debe definirse desde el soporte de producto y runners disponibles; este diseño no fija compatibilidad que aún no se haya probado.
- [ ] El perfil mínimo final y cualquier permiso de runtime/temporal deben cerrarse con evidencia en runners macOS; el diseño no presume que la lista inicial sea suficiente.
