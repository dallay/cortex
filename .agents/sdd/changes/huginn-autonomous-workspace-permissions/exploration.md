# Exploración: autonomía y permisos de workspace de Huginn

## Estado actual

La dirección aprobada es «Autónomo dentro del workspace»: el trabajo ordinario interno debe avanzar sin solicitar aprobación por cada efecto. Los efectos destructivos, externos, fuera del workspace o sensibles conservan límites explícitos. Además, todos los menús interactivos de la TUI deben admitir selección por clic y el diálogo de aprobación debe mejorar visualmente sin perder soporte de teclado.

El contrato vigente de ADR-0007 aprueba lecturas/búsquedas dentro del workspace y exige aprobación para escrituras, shell y cada llamada MCP. `HostPolicy` en `crates/huginn/application/src/authorization.rs` es autoridad central; CLI `--allow` otorga nombres de acción por invocación, pero no autoriza shell/MCP/escrituras mediante la vía de grant simple. Ya existe `ApproveForSession`, limitado a `write_file`/`edit_file` nativos registrados por el host, con ruta absoluta canónica, no sensible y coincidencia exacta por sesión + acción + ruta. El grant vive en memoria dentro del `HostPolicy` clonado y no se restaura al reanudar el proceso. Rutas sensibles solo admiten rechazo o aprobación única.

Las herramientas nativas están en `crates/huginn/runtime/src/tools.rs`: lecturas, listados y búsquedas no piden aprobación; mutaciones muestran diff completo, vuelven a validar ruta/contenido antes de escribir y fallan si el contenido cambió; shell solicita aprobación y ejecuta un proceso POSIX con límites de tiempo/salida, pero **no está sandboxed**. Las guardrails son restrictivas, no constituyen autorización ni sandbox. MCP exige aprobación de inicio y de cada llamada; el proceso MCP conserva autoridad del usuario y no está sandboxed (ADR-0006). Hooks de herramientas solo pueden bloquear/anotar (ADR-0016), no conceder permisos.

La aprobación se representa en core como `ApprovalRequest`/`ApprovalDecision`; el host limita las decisiones permitidas y la presentación no puede añadirlas. La TUI mantiene input por teclado en `crates/huginn/presentation/src/overlay.rs` y pinta la aprobación en `presentation/src/terminal.rs`. La inspección no encontró manejo de eventos de ratón ni habilitación de captura de ratón en la presentación. El selector actual puede ser estrecho/compacto; los detalles de shell requieren revisión antes de aprobar. La línea interactiva existe también en `apps/huginn/src/terminal.rs`. `main.rs` selecciona y construye las políticas en el límite de host; la configuración de usuario es TOML con overrides CLI (ADR-0009).

La comparación local previa de ejemplos de agentes (`tmp/project_examples/agents`) se toma solo como contexto comparativo, no como autoridad de seguridad ni como contrato a copiar. La inspección local confirmó que la decisión del proyecto debe mantener autorización en el host y no inferir aislamiento de un prompt, herramienta, UI o proceso hijo.

Se revisaron `.agents/AGENTS.md`, `.agents/contexts/huginn.md`, arquitectura y ADR-0006/0007/0009/0010/0016. La configuración SDD indica modo OpenSpec. Hay un cambio activo distinto (`huginn-epic-01-microkernel-service-composition`); se leyó su estado, sin modificarlo. No hay especificación principal de permisos Huginn entre las áreas SDD listadas.

## Áreas afectadas

- `docs/huginn/adr/0007-read-freely-approve-effects.md` — autoridad, política y expectativas de autonomía actuales; requiere conciliación con la nueva dirección.
- `docs/huginn/architecture.md`, `docs/huginn/implementation-specification.md`, `docs/huginn/README.md` — describen límites actuales, acciones, configuración y superficies.
- `crates/huginn/application/src/authorization.rs` — elegibilidad, grants volátiles, duración/identidad de sesión y decisión efectiva.
- `crates/huginn/core/src/lib.rs` y `crates/huginn/application/src/loop_engine.rs` — contrato de decisiones, recurso y flujo prepare → aprobación → ejecución.
- `crates/huginn/runtime/src/tools.rs`, `runtime/src/mcp.rs`, `runtime/src/composition.rs` — clasificación de efectos, revalidación, sandbox real y ejecución de herramientas.
- `apps/huginn/src/main.rs`, `apps/huginn/src/config.rs`, `apps/huginn/src/terminal.rs` — defaults, CLI/configuración y política efectiva en modos TUI, lineal y no interactivo.
- `crates/huginn/presentation/src/overlay.rs`, `terminal.rs`, `state.rs`, `input.rs` — modelo de overlays, dibujo, hit testing/input de ratón y accesibilidad por teclado.
- Pruebas: `application/tests` (autorización y loop), `runtime/tests` (herramientas, guardrails, MCP), `apps/huginn/tests/cli.rs` y pruebas PTY, `presentation` (unitarias y drivers de prueba). Añadir cobertura determinista y PTY de permisos, clic, teclado, cancelación y denegación según alcance.

## Enfoques

1. **Conceder autonomía por cada efecto seguro ya clasificado** — host autoriza sin prompt solo acciones nativas ordinarias, confinadas al workspace y no sensibles; continúa pidiendo aprobación para acciones fuera de la lista y efectos de riesgo.
   - Ventajas: se alinea con la dirección del usuario, reduce interrupciones y conserva autoridad central y defaults explícitos.
   - Desventajas: requiere taxonomía precisa de efectos/recursos y revisión de las condiciones de carrera entre preparación, autorización y ejecución.
   - Esfuerzo: medio/alto.
2. **Grant amplio «workspace autónomo» por sesión** — tras habilitación/confirmación inicial, permitir la mayoría de efectos con alcance en workspace.
   - Ventajas: interacción simple y autonomía amplia.
   - Desventajas: la ubicación de un proceso shell o una herramienta MCP no limita sus efectos; un grant amplio puede convertir recursos aparentes del workspace en exfiltración o destrucción, y confunde autorización con sandboxing.
   - Esfuerzo: alto y riesgo de seguridad alto; no recomendado sin definición fuerte de clases excluidas y límites verificables.
3. **Mantener prompts por acción y mejorar solo el diálogo** — refrescar la UI y agregar clic sin alterar política.
   - Ventajas: riesgo bajo y alcance de UI acotado.
   - Desventajas: no resuelve el requisito central de autonomía y deja fricción en operaciones ordinarias.
   - Esfuerzo: medio; insuficiente.

## Recomendación

Usar el enfoque 1: política del host con autorización implícita únicamente para un conjunto cerrado de acciones nativas de bajo riesgo, con workspace canónico y recurso no sensible; conservar denegación segura y aprobación explícita para mutación destructiva, datos sensibles, acciones externas, shell/MCP y cualquier recurso no clasificable. No tratar `shell` como interno seguro solo porque su directorio de trabajo sea el workspace: no está aislado y puede acceder a red, procesos y archivos externos. Tampoco presentar MCP o plugins nativos como sandbox. Concretar en propuesta el catálogo de acciones/recursos, el límite exacto de «ordinario» y cómo manejar el valor ausente/desconocido.

Antes de implementar deben cerrarse estas decisiones:

- **Vida útil del grant:** ¿cada ejecución/turno, sesión conversacional en memoria, proceso completo o persistido al reanudar? La opción segura y compatible con el estado actual es volátil y ligada a sesión + acción + recurso canónico; no restaurar desde historial/SQLite. Si se amplía el grant, definir revocación, expiración y convivencia entre políticas clonadas.
- **Alcance por efecto/recurso:** definir si `write_file`/`edit_file` ordinarios se vuelven autónomos; separar sobrescritura/creación/borrado, archivos sensibles, fuera del root, symlinks, rutas cambiantes, operaciones de shell y recursos externos. Lista desconocida, error de canonicalización o procedencia no verificable deben requerir aprobación o denegarse, nunca heredar autonomía.
- **Persistencia/configuración y modos:** decidir si autonomía es default interactivo, opción TOML, override CLI o combinación; preservar defaults conservadores en `run`, `--json` y ausencia de prompt. Una opción explícita no puede hacer que shell/MCP estén sandboxed. Alinear la semántica con `--allow` de invocación sin comodines.
- **TUI:** inventariar todos los menús/diálogos seleccionables (settings, provider/model, workflows, plugin dialogs, help/approval y completion si es menú) y definir clic primario, filas/botones, scroll, resize y clic fuera. Mantener focus, navegación de teclado, Escape/rechazo y confirmación protegida; clic no debe saltarse revisión completa del diff/command ni armar/aprobar accidentalmente.
- **Diálogo de aprobación:** acordar jerarquía para acción, recurso, riesgo, preview íntegro, botones/decisiones disponibles, estado de revisión completa, sensibilidad y errores de terminal estrecho; preservar aprobación fresca y fail-closed.
- **Seguridad de ejecución:** documentar explícitamente que las comprobaciones actuales son límites host-side, no aislamiento del SO. Si el requisito pretendiera permitir shell/MCP automáticamente bajo una falsa suposición de confinamiento, resolver primero necesidad y diseño de sandbox real por plataforma, que no es una consecuencia automática de este cambio.
- **Verificación:** casos unitarios deterministas para clasificación y grant (acción/recurso equivocado, sensibilidad, sesión distinta, reinicio/reanudación, cancelación, prompt caído, path/symlink escape, contenido obsoleto); integración de loop para ningún efecto no autorizado; pruebas de ratón y teclado de todos los menús; PTY para terminal estrecho, resize, cancelación y conservación del flujo visual. Evitar sleeps/expectativas de timing no deterministas.

## Riesgos

- «Dentro del workspace» no implica que el efecto sea seguro: shell/MCP y herramientas nativas confiables pueden actuar fuera de ese árbol.
- Clasificación insuficiente o metadatos controlados por herramienta/plugin pueden escalar permisos; la autorización debe usar datos/procedencia asignados por host.
- Un grant persistido o demasiado amplio puede sobrevivir a cambios de workspace, sesión o contexto; definir identidad y revocación antes de ampliar su alcance.
- TOCTOU y symlinks pueden invalidar la ruta/recurso entre revisión y ejecución; mantener revalidación y fallar con nueva aprobación ante propuestas obsoletas.
- El clic puede aprobar sin revisión, romper navegación por teclado/foco o generar diferencias entre terminales; validar con PTY y entradas explícitas.
- Una UI más clara no corrige la falta de sandbox ni sustituye los límites de autorización.

## Preparación para propuesta

Sí, con el alcance recomendado. La propuesta debe explicitar el conjunto cerrado de acciones autónomas, los efectos que siempre conservan aprobación, el carácter no sandboxed de shell/MCP, la vida útil no persistente del grant salvo decisión expresa, y la cobertura de ratón/teclado y visual del diálogo.
