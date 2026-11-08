# Proposal: autonomía y permisos de workspace de Huginn

## Intent

Permitir que Huginn avance sin prompts repetidos para efectos nativos ordinarios confinados al workspace, conservando autorización central en `HostPolicy` y límites explícitos para efectos destructivos, sensibles, externos, no clasificables y no confinados. El cambio también debe hacer que todos los menús interactivos de la TUI admitan selección con ratón y mejorar la legibilidad y el feedback del diálogo de aprobación sin quitar soporte de teclado.

El usuario ratificó el catálogo de lectura, creación y edición ordinaria dentro del workspace, activo por defecto tanto en la TUI interactiva como en `run` y `--json`; los grants de sesión exactos y volátiles; los límites para shell/MCP, recursos sensibles y destructivos; los clics en todos los menús interactivos; y la mejora visual del diálogo. `write_file` solo se autoriza automáticamente para crear un archivo nuevo; reemplazar un archivo existente sigue sujeto a aprobación. El diseño puede precisar los detalles de implementación sin ampliar estas decisiones.

## Alcance

### Dentro del alcance

- Actualizar el contrato de permisos de Huginn para que el host use una lista cerrada de acciones nativas ordinarias que pueden ejecutarse sin prompt cuando el recurso esté dentro del workspace canónico y no sea sensible. La clasificación debe derivarse de identidad y metadatos controlados por el host; datos de herramienta, plugin, modelo o ruta no verificada no pueden conceder permisos.
- **Catálogo autónomo ratificado:** lecturas, listados, búsquedas y `edit_file` para modificar un archivo ordinario existente; `write_file` solo cuando crea una ruta nueva. El recurso debe estar canónicamente dentro del workspace, no ser sensible y tener propuesta preparada y revalidada inmediatamente antes de ejecutar. `write_file` que reemplace un archivo existente requiere aprobación. La política debe fallar de forma segura ante symlinks o escapes, cambios de ruta/contenido, errores de canonicalización y clasificación desconocida.
- **Efectos recomendados que siguen requiriendo aprobación explícita o denegación segura:** borrado, sobrescritura destructiva, movimientos o cambios de nombre con impacto no revisable, acceso o modificación de recursos sensibles, recursos fuera del workspace, acciones externas o no clasificables, y todo efecto cuya procedencia o alcance no pueda verificarse. Shell y cada llamada MCP deben continuar preguntando o denegarse si no hay un aprobador disponible. El shell y MCP no están sandboxed; el directorio de trabajo, el grant, las guardrails o la UI no cambian ese hecho. El consentimiento para iniciar MCP sigue siendo distinto del consentimiento para cada llamada.
- Mantener los grants de sesión volátiles y acotados por identidad de sesión, acción y recurso canónico exactos; no restaurarlos desde historial, SQLite ni al reanudar. Un grant no se amplía por comodines ni sobrevive al cambio de workspace. Las rutas sensibles y las acciones excluidas no reciben grants de sesión.
- Aplicar el mismo catálogo autónomo por defecto en TUI, `run` y `--json`. Los efectos excluidos que requieren aprobación fallan cerrados cuando no hay aprobador. Ninguna opción convierte shell o MCP en operaciones aisladas.
- Añadir entrada de ratón a todos los menús interactivos de la TUI —incluidos settings, selección de proveedor/modelo, workflows, diálogos de plugins, ayuda, aprobación y cualquier selector/completion que se presente como menú— sin eliminar navegación, foco y activación por teclado. Definir hit testing de filas y botones, desplazamiento, resize y comportamiento de clic fuera. Un clic no puede saltarse una revisión requerida, aprobar una solicitud obsoleta ni confirmar accidentalmente una acción protegida.
- Mejorar el diálogo de aprobación con jerarquía visible para acción, recurso, riesgo y vista previa íntegra; conservar únicamente las decisiones que el host permite. Dar feedback visible cuando una aprobación no esté disponible (por ejemplo, canal ausente, terminal no interactiva o presentación revocada), explicar que el efecto no se ejecutará y ofrecer una salida segura. El diseño debe mantener el comportamiento fail-closed, el foco y la cancelación por teclado.
- Conciliar documentación y contratos afectados, en particular ADR-0007 y la documentación de Huginn sobre efectos, configuración, shell/MCP y aprobación.
- Añadir pruebas deterministas de política, sesión, modos CLI/TUI, disponibilidad de aprobación y entradas por ratón/teclado; validar el diálogo y los menús con pruebas de presentación y PTY cuando corresponda.

### Fuera del alcance

- Implementar o afirmar un sandbox del sistema operativo para shell, MCP, plugins o procesos hijos. La clasificación de efectos propuesta es política de autorización del host, no aislamiento.
- Aprobar automáticamente shell, llamadas MCP, acciones externas o efectos genéricos solo porque su ruta aparente o directorio actual estén dentro del workspace.
- Persistir grants en disco, historial de sesión o reanudación de proceso; ampliar grants a comodines, recursos sensibles o fuera del workspace.
- Cambiar la autoridad de `HostPolicy`, permitir a plugins/modelos conceder permisos o convertir hooks que bloquean/anotan en fuentes de grants.
- Alterar otros comportamientos de TUI o rediseñar temas completos fuera de los menús interactivos y el diálogo de aprobación.
- Escribir implementación en esta fase. La autorización actual cubre la planificación SDD; el código requiere una fase posterior aprobada.

## Approach

Seguir el enfoque de lista cerrada recomendado por la exploración. Mantener la autorización en el host y separar preparación, clasificación, decisión y ejecución. La especificación debe precisar qué operaciones nativas califican como ordinarias y sus condiciones de recurso; todo caso desconocido debe requerir aprobación o denegarse. Mantener la revalidación de ruta y contenido antes de mutar para evitar que una propuesta obsoleta o un cambio de symlink escape de la revisión. No inferir confinamiento de shell/MCP a partir de sus argumentos, ruta o UI.

Para grants, se recomienda conservar el alcance actual de sesión en memoria y hacerlo explícito como identidad de sesión + acción + recurso canónico, sin restauración. Para la configuración se recomienda autonomía interactiva limitada al catálogo y defaults conservadores en modos sin interacción; ambas son recomendaciones sujetas a decisión, no acuerdos ya tomados. En presentación, añadir captura y despacho de eventos de ratón a los menús existentes, manteniendo el mismo estado de foco y las rutas de teclado. El diálogo debe comunicar con claridad tanto la solicitud revisable como la indisponibilidad de aprobación, sin ampliar las decisiones autorizadas.

## Áreas afectadas

| Área | Impacto | Descripción |
|------|--------|-------------|
| `docs/huginn/adr/0007-read-freely-approve-effects.md` | Modified | Conciliar la política de autorización y el catálogo de efectos con la dirección de autonomía aprobada. |
| `docs/huginn/architecture.md`, `docs/huginn/implementation-specification.md`, `docs/huginn/README.md` | Modified | Documentar catálogo, modos, grants y límites de shell/MCP. |
| `crates/huginn/application/src/authorization.rs` | Modified | Evaluación de elegibilidad, alcance del grant y decisión efectiva del host. |
| `crates/huginn/core/src/lib.rs`, `crates/huginn/application/src/loop_engine.rs` | Modified | Contrato y flujo de decisión, aprobación disponible/no disponible y autorización de efectos. |
| `crates/huginn/runtime/src/tools.rs`, `runtime/src/mcp.rs`, `runtime/src/composition.rs` | Modified | Clasificación de efectos nativos, revalidación y límites de herramientas no aisladas. |
| `apps/huginn/src/main.rs`, `apps/huginn/src/config.rs`, `apps/huginn/src/terminal.rs` | Modified | Defaults, configuración/CLI y comportamiento interactivo y no interactivo. |
| `crates/huginn/presentation/src/overlay.rs`, `terminal.rs`, `state.rs`, `input.rs` | Modified | Ratón en todos los menús, teclado preservado, jerarquía y estados del diálogo de aprobación. |
| Pruebas de `application`, `runtime`, `apps/huginn` y `presentation` | Modified | Cobertura de autorización, grants, límites de modo, ratón/teclado y feedback de aprobación. |

## Riesgos

| Riesgo | Probabilidad | Mitigación |
|------|------------|------------|
| Un catálogo demasiado amplio trata un efecto destructivo o sensible como ordinario. | Alta | Usar lista cerrada asignada por el host, revisar caso por caso y exigir aprobación o denegar ante incertidumbre. |
| Los usuarios interpretan «workspace» como sandbox, especialmente para shell o MCP. | Alta | Documentar y mostrar que shell/MCP no están sandboxed; mantener aprobación por llamada y no otorgar autonomía por ubicación aparente. |
| Escape por symlink, TOCTOU o propuesta obsoleta entre revisión y ejecución. | Media | Canonicalizar y revalidar recurso/contenido inmediatamente antes de ejecutar; fallar cerrado y pedir una revisión nueva. |
| Grants amplios o restaurados se aplican a otra sesión o workspace. | Media | Limitar grants a sesión + acción + recurso canónico; mantenerlos volátiles y probar separación, reinicio y reanudación. |
| Defaults permisivos en ejecución no interactiva convierten falta de UI en aprobación implícita. | Alta | Mantener default conservador recomendado; falta de aprobador debe denegar/fallar cerrado y producir feedback verificable. |
| Ratón cambia foco o confirma una acción protegida de forma accidental, o rompe terminales distintos. | Media | Preservar rutas de teclado y confirmación protegida; probar hit testing, foco, resize, clic fuera y estados de revisión con drivers/PTY. |

## Plan de reversión

Revertir el cambio de forma atómica para restaurar el contrato previo de aprobación por efecto y la UI anterior. Si el despliegue revela una clasificación incorrecta antes de revertir, deshabilitar la opción de autonomía mediante su configuración/override y volver a exigir aprobación para los efectos afectados. Ningún rollback debe restaurar grants persistidos, porque esta propuesta recomienda no persistirlos. La reversión no debe alterar datos del workspace ni afirmar que deshace efectos ya ejecutados.

## Dependencias

- Decisiones de especificación/diseño sobre catálogo exacto, modo predeterminado, semántica del grant y fallo de modos no interactivos.
- Compatibilidad con los contratos vigentes de `HostPolicy`, `ApprovalRequest`/`ApprovalDecision`, ADR-0006/0007/0009/0016 y el modelo de overlays de la TUI.
- Soporte de eventos de ratón de la biblioteca de terminal ya utilizada; esta propuesta no presupone una dependencia nueva.

## Criterios de éxito

- [ ] La especificación define una lista cerrada y verificable de efectos nativos ordinarios elegibles sin prompt y las condiciones canónicas/no sensibles del recurso.
- [ ] Shell, MCP, efectos externos, sensibles, destructivos no clasificados y fuera del workspace nunca reciben autorización automática por ubicación o por ausencia de UI; shell/MCP se describen explícitamente como no sandboxed.
- [ ] Los grants tienen ámbito y vida útil explícitos; los casos de sesión, acción, recurso, reinicio y reanudación no permiten expansión accidental.
- [ ] Los defaults y overrides interactivos/no interactivos son explícitos, y un efecto que necesita aprobación falla cerrado cuando no hay aprobador disponible.
- [ ] Cada menú interactivo de la TUI se puede seleccionar con ratón y teclado, con foco y cancelación predecibles; ningún clic omite revisión o confirmación protegida.
- [ ] El diálogo presenta la acción, recurso y vista previa requerida, conserva solo decisiones válidas y comunica claramente cuando la aprobación no está disponible.
- [ ] Las pruebas cubren autorización, clasificación desconocida, sensibilidad, escapes/symlinks, contenido obsoleto, cancelación, aprobador no disponible, ratón/teclado y terminal estrecha/resize sin expectativas temporales no deterministas.
- [ ] La documentación distingue autorización del host de aislamiento del sistema operativo y no afirma sandboxing.
