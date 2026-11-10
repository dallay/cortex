<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:70168a6d5ba42de730739ce1c9732fcd2fdc2fe39b4e63d8f2e1fcd4c870bda2 -->

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:ed99815a5a04bdfc399ca33e91dc7bbf3a7b98f24938392e93bdb6bbb96d9bca -->

# Propuesta: Permisos autónomos de espacio de trabajo para Huginn

## Intención

Implementar en macOS una frontera de sistema operativo para los comandos de shell de Huginn, siguiendo el patrón técnico de Codex: perfil Seatbelt SBPL generado dinámicamente, política de denegación predeterminada y ejecución del shell real dentro de `/usr/bin/sandbox-exec`, con confinamiento heredado por sus procesos descendientes. La referencia adopta el mecanismo de sandbox; no adopta toda la política de escalation de Codex.

Apple depreca `sandbox-exec` y las interfaces Seatbelt relacionadas. La implementación debe comunicar esta limitación, fallar de forma cerrada y no afirmar compatibilidad con otros sistemas operativos.

## Alcance

### Incluido

- Implementar un backend de sandbox solo para macOS, basado en SBPL dinámico y el ejecutable absoluto `/usr/bin/sandbox-exec`, aplicado a `/bin/sh -c` y sus descendientes.
- Mantener el sandbox activo por defecto en macOS, con `(deny default)`, permisos mínimos demostrados, escritura acotada al workspace, lectura del workspace y excepciones de runtime bajo `/bin` y `/usr/lib`, metadatos del padre inmediato del workspace y acceso de red denegado.
- Separar el confinamiento técnico de la política de aprobación. Una aprobación ordinaria autoriza una operación según la política; no elimina ni amplía la frontera del sistema operativo.
- Si falla la preparación, validación, instalación del perfil o el launcher, no ejecutar el comando. Informar un error al usuario; no reintentar fuera del sandbox.
- Mantener las concesiones persistentes desactivadas y no crear, cargar ni honrar un registro de grants persistentes. Los grants de sesión de `HostPolicy` permanecen solo en memoria, vinculados a la sesión, acción y ruta canónica exactas para `native.write_file` y `native.edit_file` con procedencia nativa y recurso no sensible; no amplían el sandbox de shell.
- Preservar las decisiones vigentes sobre resolución de política y alcance de operaciones: las restricciones administradas prevalecen y solo restringen; la confianza del proyecto no amplía permisos; `Deny` y `Allow once` conservan su alcance; no se añaden modos YOLO, Exit ni acceso total.
- Preservar el ciclo de vida actual de ejecución, incluyendo entorno seleccionado, directorio de trabajo, entrada/salida, cancelación y timeout. Para las salidas controladas, realizar cleanup explícito y propagar su error; ante terminación abrupta o cancelación por `drop`, usar `Drop` como intento de mejor esfuerzo, sin prometer que los recursos se eliminen.
- Documentar la deprecación de Apple, los límites del sandbox y la posibilidad de que algunos comandos existentes fallen.

### Excluido

- Cualquier ejecución de shell sin sandbox, modo opt-out, configuración `sandbox = false` o bypass del launcher. En macOS, `run_shell` siempre debe ejecutarse bajo sandbox; si no es posible, debe fallar sin ejecutar.
- Cualquier escalación de permisos o elevación fuera del sandbox. Una aprobación ordinaria de `HostPolicy` no constituye consentimiento para retirar el confinamiento. Cualquier elevación futura requiere un cambio separado con contrato explícito de consentimiento, identidad de operación, explicación del riesgo y razón obligatoria.
- Copiar la política completa de escalation de Codex. Este cambio copia solo el mecanismo técnico Seatbelt y no importa sus flujos de consentimiento, excepciones o permisos amplios.
- Linux y Windows, sus backends de aislamiento y cualquier afirmación de paridad entre plataformas.
- Acceso de red, sockets o excepciones de proxy dentro del sandbox.
- Grants persistentes, almacenamiento o interfaz de gestión de grants. El verificador de restricciones administradas sigue siendo requisito independiente para cualquier propuesta futura de grants.
- Permisos amplios de Codex sin necesidad demostrada; App Sandbox firmado, helpers, bookmarks, empaquetado o firma. El POC de App Sandbox anterior no constituye evidencia de aislamiento.
- Afirmar que el sandbox protege operaciones nativas, llamadas MCP u otros efectos fuera de la ejecución de shell cubierta por este cambio.

## Enfoque

Envolver el shell existente en el launcher fijo `/usr/bin/sandbox-exec` y generar una política SBPL segura a partir de rutas normalizadas y validadas. La generación debe tratar cada ruta como entrada no confiable, escapar correctamente la sintaxis SBPL y evitar que rutas con metacaracteres, enlaces simbólicos o condiciones de carrera amplíen permisos. No se interpolarán rutas del usuario en texto de shell. El cleanup explícito de las salidas controladas debe propagar errores observables; un `Drop` durante terminación abrupta o cancelación es solo un intento de mejor esfuerzo y no garantiza la eliminación. El error de cierre de `std::fs::File` no es observable mediante APIs safe y no constituye un criterio de verificación independiente.

El perfil partirá de `(deny default)`. Solo incluirá los permisos mínimos demostrados para iniciar `/bin/sh`, ejecutar y mantener sus descendientes confinados, leer los componentes requeridos del sistema y acceder al workspace. La red permanecerá denegada. No se añadirán permisos generales para directorios temporales, preferencias, aplicaciones, proxies o servicios opcionales sin evidencia.

El sandbox estará activo por defecto en macOS y no habrá opt-out ni fallback sin sandbox en este cambio. Si falla la configuración, generación, validación o aplicación del perfil, o no está disponible el launcher, Huginn no ejecutará el comando y mostrará un error. La aprobación ordinaria de shell no autoriza una ejecución sin confinamiento. Cualquier elevación fuera del sandbox requiere un cambio separado que defina un contrato de consentimiento específico. En sistemas distintos de macOS, este cambio no declara ni implementa aislamiento de shell.

La validación incluirá pruebas adversariales en macOS: acceso permitido al workspace; denegación de credenciales, otros workspaces, rutas externas y red; confinamiento de descendientes; tratamiento seguro de rutas especiales; error sin ejecución cuando falle el perfil o launcher; y cancelación/timeout con cleanup explícito y propagación de errores cuando la salida sea controlada. Para terminación abrupta o cancelación por `drop`, se verificará únicamente el intento de mejor esfuerzo; no se afirmará garantía de eliminación ni de éxito ante fallos del sistema operativo. La matriz de versiones principales de macOS informará resultados observados, sin implicar respaldo de Apple.

## Áreas afectadas

| Área                                                                    | Impacto              | Descripción                                                                                                 |
|-------------------------------------------------------------------------|----------------------|-------------------------------------------------------------------------------------------------------------|
| `crates/huginn/runtime/src/tools.rs`                                    | Modificada           | Envolver la ejecución real de shell, preservar ciclo de vida y fallar cerradamente.                         |
| `crates/huginn/runtime/`                                                | Modificada           | Generación segura de política y pruebas de unidad, integración y adversariales en macOS.                    |
| `docs/huginn/README.md`, `docs/huginn/architecture.md` y ADR pertinente | Modificada           | Explicar límites, deprecación, default, impacto de compatibilidad y alcance de plataforma.                  |
| `.github/workflows/ci.yml`                                              | Posible modificación | Incorporar pruebas macOS a una ruta bloqueante adecuada; definirlo en diseño y tareas.                      |
| `.agents/sdd/changes/huginn-autonomous-workspace-permissions/`          | Actualizada          | Alinear la propuesta con fail-closed y dejar cualquier elevación fuera del sandbox para un cambio separado. |

## Impacto de compatibilidad

El default activo en macOS es un cambio intencional de comportamiento. Los comandos que lean o escriban fuera del workspace, usen red o sockets, dependan de servicios no permitidos o requieran rutas no incluidas pueden fallar y mostrar un error. No se ofrece una exclusión para recuperar compatibilidad mediante ejecución sin sandbox. La disponibilidad y el comportamiento de `sandbox-exec` pueden variar o desaparecer en futuras versiones de macOS. Linux y Windows mantienen su comportamiento actual y quedan fuera del alcance; esto no afirma que esos sistemas estén aislados.

## Riesgos

| Riesgo                                                                                                                                                          | Probabilidad | Mitigación                                                                                                                                                                           |
|-----------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Apple cambia o elimina una interfaz deprecada.                                                                                                                  | Alta         | Probar versiones macOS objetivo, informar errores y no habilitar fallback no confinado.                                                                                              |
| El sandbox bloquea comandos válidos por el perfil mínimo.                                                                                                       | Alta         | Documentar el impacto, probar herramientas representativas y devolver errores accionables; no permitir opt-out en este cambio.                                                       |
| Un error en generación o escape de SBPL amplía permisos.                                                                                                        | Media        | Generador pequeño y testeable, entradas validadas, pruebas adversariales y denegación predeterminada.                                                                                |
| Carreras de rutas, enlaces simbólicos o descriptores heredados eluden el confinamiento esperado.                                                                | Media        | Normalización segura, revisión y pruebas de rutas, enlaces, descriptores y descendientes; no afirmar garantías no verificadas.                                                       |
| El wrapper altera timeout, cancelación o señales al grupo de procesos; el cleanup puede no completarse ante terminación abrupta o fallos del sistema operativo. | Media        | Probar salidas controladas y propagación de errores observables; tratar `Drop` como mejor esfuerzo, sin prometer eliminación ante cancelación abrupta o fallo del sistema operativo. |
| El default activo afecta a usuarios actuales de macOS.                                                                                                          | Alta         | Tratarlo como cambio de compatibilidad explícito y proporcionar errores claros; no ejecutar fuera del sandbox.                                                                       |

## Plan de reversión

Si la matriz de macOS, las pruebas adversariales o la confiabilidad de la interfaz deprecada no cumplen los criterios, detener el lanzamiento y mantener los comandos sin ejecutar bajo este cambio (fail-closed), sin fallback automático. Una reversión posterior puede retirar el backend en un cambio explícito; no debe convertir silenciosamente el default en ejecución sin sandbox. No hay grants persistentes que migrar o revertir.

## Dependencias

- macOS con `/usr/bin/sandbox-exec` disponible para desarrollo y pruebas; su ausencia produce error y no ejecución.
- Acceso a runners o entornos macOS para probar las versiones objetivo.
- Cualquier consentimiento para elevar permisos fuera del sandbox queda fuera de este cambio y requiere un cambio separado.

## Criterios de éxito

- [ ] En macOS, `run_shell` usa por defecto el perfil SBPL mínimo generado dinámicamente y `/usr/bin/sandbox-exec`.
- [ ] El shell y sus descendientes quedan dentro de la política probada; si falla el perfil o launcher, el usuario recibe un error y el comando no se ejecuta.
- [ ] No hay opt-out, configuración `sandbox = false` ni fallback a ejecución sin sandbox.
- [ ] La aprobación ordinaria no elimina ni amplía el sandbox; cualquier elevación futura requiere otro cambio y contrato de consentimiento explícito.
- [ ] El perfil deniega red y limita escrituras al workspace; no importa permisos amplios de Codex sin evidencia.
- [ ] Las pruebas adversariales cubren rutas, límites de archivos, red, descendientes, fallos del launcher y cancelación/timeout; en salidas controladas, el cleanup explícito propaga los errores observables. En terminación abrupta o cancelación por `drop`, cleanup es best-effort y no garantiza eliminación. El error de cierre de `std::fs::File` no se verifica por separado porque no es observable mediante APIs safe.
- [ ] La matriz de versiones principales de macOS informa resultados reproducibles y límites observados.
- [ ] Grants persistentes siguen desactivados; Linux y Windows quedan fuera del alcance, sin claims de paridad.
- [ ] La documentación explica la deprecación de Apple, el cambio de default y la posibilidad de que algunos comandos fallen.

<!-- /coderabbit-ai-generated:v1 --><!-- /coderabbit-ai-generated:v1 -->