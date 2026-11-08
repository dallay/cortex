# Diseño: autonomía y permisos de workspace de Huginn

## Enfoque técnico

La autorización seguirá siendo una decisión del host. `huginn-core` conservará contratos independientes de terminal e I/O; `huginn-application::HostPolicy` aplicará el catálogo cerrado y el scope de grants; `huginn-runtime` asignará identidad/procedencia confiables y preparará/revalidará recursos; `huginn-presentation` recogerá decisiones y mostrará estados, sin autoridad propia; `apps/huginn` compondrá la política y resolverá configuración/modo. Este diseño sigue ADR-0006/0007/0009/0016 y la separación `core → application → runtime/presentation → apps` establecida en `.agents/contexts/huginn.md`.

**Decisiones ratificadas por el usuario:** lectura, listado, búsqueda, creación y edición ordinaria de archivos dentro del workspace se autorizan por defecto en TUI, `run` y `--json`; `write_file` solo crea sin prompt una ruta nueva, mientras reemplazar un archivo existente requiere aprobación; los grants de sesión son exactos y volátiles; shell/MCP, recursos sensibles, externos, destructivos no clasificados y fuera del workspace conservan controles; todos los menús interactivos aceptan clic; el diálogo mejora su jerarquía visual. Ninguna decisión convierte shell/MCP en sandboxed.

## Decisiones de arquitectura

### Decisión: clasificación cerrada de efectos asignada por el host

Añadir una clasificación interna tipada (por ejemplo, `EffectClass` más identidad de acción nativa) cuya fuente sea el registro del host/runtime. No derivar permisos del texto de preview, los argumentos del modelo, los nombres aportados por plugins ni la ruta sin validar. `HostPolicy` compara acción exacta, clase, procedencia y recurso; valor ausente/desconocido, discrepancia o error significa aprobación explícita si existe aprobador, de otro modo rechazo/fallo cerrado.

Catálogo ratificado:

- Autónomos: lecturas, listados y búsquedas nativas dentro del workspace activo canónico, no sensibles, solo si la identidad está registrada por el host. Mantener su comportamiento actual.
- Autónomos: `native.write_file` solo para crear una ruta nueva; `native.edit_file` para modificar un archivo ordinario existente mediante la edición puntual. En ambos casos el recurso es no sensible y está canónicamente dentro del workspace; la propuesta se prepara y revalida justo antes de escribir.
- Requiere aprobación: `native.write_file` al reemplazar un archivo existente, además de borrado, movimiento, rename y otras operaciones destructivas no clasificadas.
- Siempre fuera de la autonomía: shell (incluidas variantes), cada invocación MCP, arranque/consentimiento MCP como grant de llamadas, efectos externos, recursos sensibles, fuera del root, procedencia desconocida, operaciones destructivas no clasificadas y cualquier error de verificación. Shell y MCP no están sandboxed; el cwd, el grant, guardrails y autorización host-side no aíslan procesos.

**Alternativas/tradeoffs**: limitar autonomía a lectura/listado/búsqueda conserva prompts de escritura que motivan el cambio. Hacer autónomas todas las escrituras nativas dentro del workspace mezcla creación/edición con truncado y reemplazo potencialmente destructivo. La decisión ratificada permite creación y edición puntual, y conserva aprobación para `write_file` que reemplaza contenido existente. Una categoría genérica como «tool interna» se rechaza: no prueba alcance ni efecto.

### Decisión: revalidación en el límite inmediatamente anterior al efecto

Mantener el ciclo `prepare → host authorization → revalidate → execute` en el adapter nativo. La autorización debe ligar decisión/grant a la misma acción y recurso canónico que fueron preparados. Antes de mutar, runtime vuelve a verificar root activo, resolución de ruta/symlinks, sensibilidad, identidad de recurso y vigencia/huella de contenido; cualquier cambio invalida la autorización de esa propuesta. No transformar automáticamente un cambio en nueva aprobación: el host debe devolver propuesta obsoleta/no verificable y preparar una revisión nueva o fallar cerrado. La comprobación final debe ocurrir dentro del adapter que ejecuta, no solo en el loop/UI.

No se debe presentar esta revalidación como sandbox ni como eliminación total de carreras del sistema de archivos. Para reducir TOCTOU, mantener handles/operaciones relativas al directorio cuando las APIs existentes lo permitan; si la garantía de la plataforma no está disponible, denegar el caso no verificable. No añadir nueva dependencia sin necesidad probada.

### Decisión: ciclo de vida de grants

Conservar los grants solo como estado en memoria, con coincidencia exacta `(identidad de sesión, acción, ruta canónica, workspace activo)` y solo para recursos no sensibles y acciones nativas elegibles. `apps/huginn` debe fijar el workspace canónico confiable en `SessionPolicy` desde el límite de composición; el root no lo puede aportar el modelo, la UI ni metadata de herramientas. La política comprueba que cada recurso canónico pertenece al root fijado y usa ese root en la clave del grant. `for_session` sin root confiable no concede ni reutiliza grants de sesión. Los clones comparten el almacenamiento en memoria de la sesión, pero no pueden cambiar el root fijado. Nunca serializar grants ni reconstruirlos desde historial/SQLite/resume. Invalidar al terminar sesión o crear un nuevo scope de workspace. No aceptar comodines.

La identidad de workspace vive en `SessionPolicy`, no en `ApprovalRequest`/`ApprovalResource`: así se evita ampliar contratos core y serializaciones, mientras el host conserva la autoridad. Una ruta absoluta no basta por sí sola para demostrar el scope del grant. La raíz se canonicaliza/valida al preparar o reanudar la sesión; si no se puede fijar, la política falla cerrada para autonomía/grants.

**Alternativas/tradeoffs**: un grant por turno minimiza exposición, pero no elimina prompts dentro de un mismo flujo y es más restrictivo que `ApproveForSession` existente. Mantener grants durante toda la sesión conversacional satisface continuidad y conserva semántica actual, con riesgo acotado por clave exacta y volatilidad. Persistirlos o restaurarlos simplifica reanudación, pero amplía el riesgo a otro proceso/contexto y contradice el alcance propuesto; no recomendado. `--allow` conserva sus nombres explícitos por invocación y no pasa a significar autorización genérica de shell/MCP/escrituras; resolver la precedencia de autonomía y `--allow` en composición sin relajar clases excluidas.

### Decisión: configuración y modos

El catálogo ratificado se activa por defecto en TUI, `run` y `--json`. Los efectos excluidos que requieren aprobación fallan cerrados cuando no hay canal/aprobador disponible. Cualquier configuración u override futuro conserva estas clases excluidas y no transforma la ausencia de interfaz en autorización.

**Alternativas/tradeoffs**: pedir activación explícita por sesión reduce sorpresas, pero conserva fricción. Un catálogo cerrado y verificable permite aplicar la misma autonomía sin prompt en los tres modos ratificados; la falta de aprobador sigue denegando los efectos excluidos. Si luego se añade override, la precedencia y el alcance de `--allow` deben documentarse y probarse.

### Decisión: entrada de ratón y geometría de menús

Activar/desactivar la captura de ratón junto a bracketed paste en el ciclo de vida existente del terminal, incluyendo restauración incluso ante error/salida. El loop de Crossterm ya centraliza `Event` y `Resize`; añadir el despacho de `Event::Mouse` a la capa de entrada/presentación y no introducir otro lector de eventos. Ratatui no proporciona el manejo de entrada: el código debe conservar el pipeline de eventos propio.

Usar un modelo de hit testing derivado de la geometría que se dibujó en el último frame, con Rects/filas/botones y generación/identidad del overlay. Ratón primario sobre una fila ejecuta la misma transición que teclado; botones invocan el mismo `InputEffect`. Clic en espacio vacío/borde/fuera es no-op, no cambia foco ni activa acción. Scroll modifica el viewport/lista y vuelve a dibujar antes de aceptar coordenadas posteriores. Resize invalida la geometría previa y la reemplaza al render siguiente; no reutilizar hitboxes viejas. Verificar identidad de overlay/ticket y que siga activo antes de comprometer efectos, siguiendo el patrón existente de `TransitionTicket`/snapshot.

El alcance incluye overlays de settings, provider/login/model picker, help, confirm/approval, workflow y plugin dialogs; también selectores/completions que se presenten como menú (incluida la línea interactiva de `apps/huginn`). Mantener navegación, foco visible, Enter/Escape y activación protegida por teclado. Las vistas que no son menús seleccionables no necesitan targets inventados.

La API instalada es Ratatui `=0.30.2` y Crossterm `=0.29.0`. Según la documentación actual consultada, Ratatui delega el input a Crossterm; Crossterm aporta `EnableMouseCapture`/`DisableMouseCapture` y `Event::Mouse`. En resize, volver a dibujar usando el área actual del frame: Ratatui documenta que `draw` actualiza el tamaño del backend, incluso si eventos resize fueron agrupados o no procesados. Usar coordenadas de evento y Rects en coordenadas de terminal. No asumir captura de ratón por Ratatui ni APIs no presentes en las versiones fijadas.

### Decisión: aprobación y feedback

La solicitud que entra a `ApprovalPrompt` sigue siendo la fuente para acción/recurso/riesgo/preview; la UI no crea ni amplía decisiones permitidas. Presentar primero identidad/acción y recurso, etiqueta clara del riesgo y luego preview completo. El indicador de progreso debe diferenciar «pendiente por revisar» de «revisado completo»; recorte visual no equivale a revisión. Conservar navegación vertical/paginación para preview completa y layout de terminal estrecha. Solo habilitar aprobación después del requisito de revisión ya existente y de volver a validar identidad/estado de la solicitud. El clic no elude ese gate.

Ausencia/revocación del prompt, cancelación, rechazo, no-op, error de revalidación y ejecución deben producir estados visibles y distintos; nunca mostrar éxito si no se ejecutó. Para shell, seguir mostrando preview íntegra y advertencia explícita de que no está sandboxed. Mantener `ApprovalDecision` permitido por host y las restricciones actuales para `ApproveForSession`.

## Flujo de datos

```text
ToolCall no confiable
  → runtime identifica registro nativo y prepara propuesta/recurso
  → HostPolicy valida acción + procedencia + clase + scope + recurso
      ├─ catálogo autónomo exacto + recurso verificado → decisión host-side
      └─ resto → ApprovalPrompt disponible → decisión validada por HostPolicy
                       └─ ausente/cancelado/error → denegar/fallar cerrado
  → runtime revalida ruta/root/sensibilidad/contenido inmediatamente antes de mutar
  → execute solo si la misma propuesta sigue vigente

Crossterm Event (Key/Mouse/Resize)
  → event loop único y Ui/overlay actual
  → transición pura de presentación con geometry snapshot + identity ticket
  → host confirma que overlay sigue vigente y ejecuta el InputEffect
  → render; resize invalida y recompone hitboxes
```

## Contratos propuestos

Los nombres siguientes son conceptuales; conservar los tipos y convenciones reales donde ya existan.

```rust
enum EffectClass {
    ReadWorkspace,
    WriteOrdinaryFile,
    Destructive,
    External,
    Unknown,
}

struct VerifiedEffect {
    action: NativeActionId,       // identidad registrada por host, no string del modelo
    class: EffectClass,
    resource: Option<VerifiedResource>,
    provenance: ApprovalProvenance,
}

enum AuthorizationOutcome {
    Authorized(ApprovalDecision),
    NeedsApproval(ApprovalRequest),
    Denied(AuthorizationFailure),
}
```

No es necesario añadir una interfaz por tipo; el objetivo del contrato es que `Unknown` no pueda colarse como clase permitida y que presentación nunca produzca `Authorized`. La revalidación puede expresar `Current`, `Stale` o `Unverifiable`; solo `Current` continúa con la misma decisión. El tipo exacto puede seguir siendo `AgentError`/`ApprovalRequest` si satisface el estado observable sin añadir variantes innecesarias.

Para TUI, mantener la traducción de input a efectos puros en `overlay.rs`, extendiéndola con hit testing en un snapshot de geometría; usar `InputEffect` existente y el ticket de transición como check de commit. El estado del overlay es owner del foco; un evento Mouse no debe mutar un callback/handle de plugin directamente.

## Archivos afectados

| Archivo | Acción | Descripción |
|---|---|---|
| `crates/huginn/application/src/authorization.rs` | Modificar | Política de catálogo cerrado, modo de autorización y grants exactos/no persistentes; mantener `HostPolicy` como autoridad y validar decisiones recibidas. |
| `crates/huginn/core/src/lib.rs` | Modificar si hace falta | Contrato de resultado/estado de aprobación o autorización; sin dependencias de terminal/adapter. Evitar cambio si contratos existentes alcanzan. |
| `crates/huginn/application/src/loop_engine.rs` | Modificar si hace falta | Orquestar aprobador ausente/denegación y preservar la secuencia sin reintento/replay de efectos. |
| `crates/huginn/runtime/src/tools.rs` | Modificar | Clasificación host-originada para builtins; preparación/revalidación de escritura y rechazo de propuestas obsoletas. |
| `crates/huginn/runtime/src/mcp.rs`, `composition.rs` | Modificar si hace falta | Preservar permiso por llamada MCP y procedencia; no inferir aislamiento. |
| `apps/huginn/src/main.rs`, `config.rs`, `terminal.rs` | Modificar | Resolver configuración/defaults/modo en composition root; mantener fallos observables de no-interactivo. |
| `crates/huginn/presentation/src/terminal.rs` | Modificar | Habilitar/restaurar captura, despacho de Mouse y actualización tras resize sin segundo lector. |
| `crates/huginn/presentation/src/overlay.rs`, `state.rs`, `input.rs` | Modificar | Añadir geometría/hit-test y transición común a teclado; validar overlay/ticket y mantener cancelación/foco. |
| `crates/huginn/presentation/src/approval.rs` y dibujo asociado en `terminal.rs` | Modificar | Jerarquía, estado de revisión completa, riesgo y feedback de rechazo/no-op/no aprobador. |
| `apps/huginn` selector/line UI y `crates/huginn/presentation` menús | Modificar tras inventario | Añadir targets de ratón a todos los menús seleccionables, incluidas completion/line UI si se presenta como menú. |
| Pruebas `application/tests`, `runtime/tests`, `apps/huginn/tests`, `presentation` | Modificar | TDD de política, ejecución/revalidación, modos, geometría, teclado/ratón y PTY. |
| `docs/huginn/adr/0007-read-freely-approve-effects.md`, `docs/huginn/{architecture.md,implementation-specification.md,README.md}` | Modificar | Documentar catálogo ratificado, grants/modos y distinción entre autorización host-side y sandbox. |

## Estrategia de pruebas (TDD)

| Capa | Casos | Método |
|---|---|---|
| Unit: application authorization | Catálogo exacto; acción/procedencia desconocida; sensibilidad; fuera del root; acción/recurso/sesión/workspace distintos; cancelación y aprobador no disponible; decisiones no permitidas | Escribir primero pruebas fallidas de `HostPolicy` con prompt determinista; verificar clones y que grants no reaparezcan tras crear nueva política/sesión. |
| Unit: runtime tools | Root cambiado, symlink/escape, error de canonicalización, contenido obsoleto, sensibilidad reevaluada, herramienta registrada/revocada | Fakes de filesystem/acciones donde posible y fixtures tempfile deterministas; comprobar que no ocurre escritura antes ni después de fallo. |
| Integration: application loop + runtime | Solo efecto del catálogo ejecuta; prompt requerido en shell/MCP/destructivo; start MCP no da grant de tool-call; aprobación revocada/cancelada/ausente no ejecuta ni reintenta | Tests de flujo con `ApprovalPrompt`, `Tool`, registry y `EventSink` falsos; afirmar explícitamente cero invocaciones de execute. |
| CLI/config | Precedencia CLI/TOML/default y diferencias interactivo, `run`, `--json`; `--allow` no convierte shell/MCP ni efectos excluidos | Tests existentes de `apps/huginn/tests/cli.rs`, parseo TOML con config aislada y aserciones de error/resultados. |
| Presentation unit | Coordenadas en cada fila/botón; fuera/no objetivo; geometría invalidada en resize; overlay viejo/ticket viejo; clic no supera confirmación; preview incompleta; keyboard focus/Escape | Driver/pruebas existentes de presentation, `TestBackend`, eventos `Event::Mouse` fabricados; mismas transiciones observables que teclado. |
| PTY | Captura/restauración de terminal; todos los overlays/menu clickables; estrecho/resize; scroll de preview; cancelación; salida por error | Fixture PTY ya existente y entradas mouse/resize controladas; sin sleeps ni asserts dependientes de timing. |
| Seguridad/calidad | Límites arquitectónicos, lints, regresiones y contratos | `cargo test -p <paquete Huginn afectado>`, `just huginn-architecture`, fmt/clippy pertinentes; revisión de que no se añadió dependencia innecesaria ni se afirma sandbox. |

La documentación no necesita test conductual independiente. No ejecutar pruebas destructivas sobre el workspace del usuario; fixtures temporales y fakes deben contener las mutaciones.

## Migración / despliegue

No requiere migración de datos. Los grants continúan volátiles y los historiales/sesiones existentes no se deben reescribir. Si se adopta una opción de configuración nueva, definir default y compatibilidad de TOML antes del release; por la deserialización `deny_unknown_fields` de `Config`, añadir campos TOML de forma deliberada y probar configs existentes. Rollback debe restaurar prompts/política conservadora y no puede revertir efectos ya ejecutados.

## Decisiones ratificadas y criterios para apply

1. **Catálogo**: lectura/listado/búsqueda; `write_file` solo para crear; `edit_file` para modificar archivos ordinarios existentes mediante edición puntual. Reemplazar con `write_file`, borrar, mover, renombrar y otros efectos destructivos no clasificados requieren aprobación.
2. **Defaults**: el catálogo opera por defecto en TUI, `run` y `--json`. Shell/MCP y cualquier efecto excluido que requiere aprobación falla cerrado si no hay aprobador interactivo.
3. **Grant/workspace**: decisión técnica cerrada para apply: ligar el root canónico en `SessionPolicy` desde `apps/huginn`; incluirlo en la clave exacta del grant y comprobar que el recurso pertenece al root. No añadir el workspace a contratos core ni habilitar grants de `for_session` sin root confiable. Mantener estado volátil y probar scopes distintos, clones, fuera del root y nueva sesión/reanudación.
4. **Mouse**: todos los menús interactivos, incluidos selectors/completion de `apps/huginn/src/terminal.rs`. La activación por clic debe seguir la misma transición válida que el teclado; en aprobación debe respetar el gate de revisión y dar feedback si el preview está incompleto.

Las decisiones de política están ratificadas. `sdd-tasks` puede desglosar la implementación. Si aparece un caso que amplía el catálogo, persiste grants o relaja un límite aprobado, detenerse y pedir autorización antes de apply.

## Riesgos

- Una clasificación genérica o metadato de herramienta no confiable puede conceder efectos host-side no pretendidos; el fallback debe ser aprobación/rechazo, nunca autonomía.
- Una ruta «dentro del workspace» no demuestra que shell/MCP/plugins estén confinados. No presentar autorización host-side como sandbox del sistema operativo.
- TOCTOU entre revalidación y operación puede persistir; fallar si no se puede asegurar identidad del recurso y no reusar decisiones para propuestas nuevas.
- Persistir o ampliar grants puede cruzar sesión/workspace; mantener estado en memoria y scope exacto salvo decisión explícita y análisis separado.
- Capturar ratón cambia el estado global del terminal; restaurar modo en todos los caminos y validar terminales/PTY. Hitboxes obsoletas o un clic de aprobación accidental pueden activar efectos.
- El modo sin prompt no debe convertir una política conservadora en autorización implícita; error de canal/presentación debe impedir ejecución y ser visible.
