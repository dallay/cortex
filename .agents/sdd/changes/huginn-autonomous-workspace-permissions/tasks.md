# Tareas: autonomía y permisos de workspace de Huginn

## Previsión de carga de revisión

| Campo | Valor |
|---|---|
| Presupuesto de revisión | 400 líneas modificadas |
| Carga estimada | Alta: política, runtime, CLI/TUI, menús, pruebas y documentación |
| PR encadenados recomendados | Sí |
| Estrategia | `feature-branch-chain` elegida por el usuario |
| Balance de entregas | Capas por entregable autónomo: permisos/revalidación; entrada de ratón; aprobación y documentación. Cada capa incluye pruebas y reversión coherentes. |
| Base | `main` |
| Rama de integración | `integration/huginn-autonomous-workspace-permissions` |
| Seguimiento | `.agents/rpi/plan/tasks/huginn-approval-interactions.md` |
| Slice 1 | `feature/huginn-workspace-autonomy-policy`, desde integración |
| Slice 2 | `feature/huginn-tui-mouse-menus`, desde slice 1 |
| Slice 3 | `feature/huginn-approval-ui`, desde slice 2 |

Decisiones de estrategia/base/seguimiento/commits locales antes de apply: resueltas por el usuario
PR encadenados recomendados: Sí
Riesgo del presupuesto de 400 líneas: Alto

### Metadatos de ramas por slice

| Posición | `trunk` | `parent_branch` | `base` | `branch` | Issue/tracker |
|---|---|---|---|---|---|
| 1 de 3 | `main` | `integration/huginn-autonomous-workspace-permissions` | `integration/huginn-autonomous-workspace-permissions` | `feature/huginn-workspace-autonomy-policy` | Tracker RPI local; sin issue externo |
| 2 de 3 | `main` | `feature/huginn-workspace-autonomy-policy` | `feature/huginn-workspace-autonomy-policy` | `feature/huginn-tui-mouse-menus` | Tracker RPI local; sin issue externo |
| 3 de 3 | `main` | `feature/huginn-tui-mouse-menus` | `feature/huginn-tui-mouse-menus` | `feature/huginn-approval-ui` | Tracker RPI local; sin issue externo |

## Fase 1: política y efectos nativos — capa 1

- [ ] 1.1 RED: Añadir pruebas deterministas de `HostPolicy` para catálogo cerrado, procedencia host, sensibilidad, recurso canónico y fallback desconocido; cubrir lectura/listado/búsqueda y permisos de creación/edición.
- [ ] 1.2 GREEN: Implementar autorización host-side del catálogo ratificado; `write_file` solo crea rutas nuevas, `edit_file` edita archivos ordinarios existentes y reemplazo/destructivos requieren aprobación.
- [ ] 1.3 REFACTOR: Consolidar clasificación y decisión sin ampliar autoridad; repetir pruebas de aplicación y revisar que plugins/hooks/modelo no conceden permisos.
- [ ] 1.4 RED: Probar grants exactos por sesión+acción+recurso canónico+workspace; `SessionPolicy` sin workspace confiable no concede grants; cubrir clones del mismo scope, separación de roots, recurso fuera del root, sensibilidad y ausencia tras nueva sesión/reanudación.
- [ ] 1.5 GREEN: Fijar el root canónico confiable en `SessionPolicy` desde el composition root; incluir root en la clave, comprobar pertenencia del recurso e impedir grants sin scope. Mantener grants en memoria, sin comodines ni persistencia.
- [ ] 1.6 REFACTOR: Simplificar el estado compartido de `HostPolicy`; verificar decisiones permitidas y precedencia de `--allow` sin permitir que clones cambien el root fijado.

## Fase 2: revalidación y modos de ejecución — capa 1

- [ ] 2.1 RED: Añadir pruebas con filesystem/fakes deterministas para symlink/escape, error de canonicalización, sensibilidad, workspace root cambiado o fuera del scope fijado, y contenido obsoleto; afirmar cero escrituras ante fallo.
- [ ] 2.2 GREEN: Aplicar preparación→autorización→revalidación inmediata→ejecución en el adapter nativo; revalidar root fijado, pertenencia/ruta/symlinks, sensibilidad e identidad/contenido; invalidar propuestas modificadas y fallar cerradas.
- [ ] 2.3 REFACTOR: Eliminar comprobaciones duplicadas sin debilitar el límite final del adapter; repetir pruebas de runtime.
- [ ] 2.4 RED: Probar catálogo por defecto en TUI, `run` y `--json`; shell, cada llamada MCP y otros efectos excluidos requieren aprobador y no ejecutan si falta.
- [ ] 2.5 GREEN: Componer defaults y errores observables en `apps/huginn`; conservar aprobación separada para inicio MCP y cada llamada.
- [ ] 2.6 REFACTOR: Verificar precedencia TOML/CLI/`--allow` y fallos cerrados con mocks deterministas; no inferir sandbox para shell/MCP.

## Fase 3: ratón y menús interactivos — capa 2

- [ ] 3.1 RED: Añadir pruebas de presentación para cada menú/selector, clic de fila/botón, clic fuera, scroll, foco/teclado, overlay obsoleto y geometría invalidada por resize; exigir que clic y teclado produzcan la misma transición válida.
- [ ] 3.2 GREEN: Implementar hit-testing desde geometría del último frame y despachar `Event::Mouse` por el event loop existente a través de `InputEffect`/ticket vigente; cubrir overlays y selector/completion de línea.
- [ ] 3.3 REFACTOR: Unificar rutas de entrada y verificar que clic vacío no cambia selección ni activa acciones protegidas.
- [ ] 3.4 RED: Añadir prueba PTY de habilitación/restauración de captura de ratón en salida normal, error y cancelación.
- [ ] 3.5 GREEN: Habilitar captura junto a bracketed paste y restaurar terminal en todos los caminos de salida.
- [ ] 3.6 REFACTOR: Repetir prueba PTY y confirmar que no se introduce un segundo lector de eventos.

## Fase 4: aprobación, integración y documentación — capa 3

- [ ] 4.1 RED: Probar que Allow once/Allow session por clic y teclado no omiten revisión íntegra, no aceptan preview incompleto/solicitud obsoleta, y que rechazo, cancelación, no-op o aprobador ausente nunca ejecutan el efecto.
- [ ] 4.2 GREEN: Mejorar jerarquía visual de acción, recurso, riesgo y preview; mantener decisiones autorizadas por host, gate de revisión y feedback claro de no ejecución.
- [ ] 4.3 REFACTOR: Verificar terminal estrecha, navegación del preview completo, resize, foco y Escape; repetir pruebas de presentación y PTY sin sleeps.
- [ ] 4.4 RED: Añadir pruebas de integración de loop con `ApprovalPrompt`, herramientas y `EventSink` falsos para aprobación, cancelación, revocación y ausencia; afirmar cero llamadas de ejecución no autorizadas y no replay.
- [ ] 4.5 GREEN: Conectar estados de aprobación y resultados observables en core/application/runtime/presentation y CLI sin ampliar la autoridad de UI.
- [ ] 4.6 REFACTOR: Ejecutar pruebas de paquetes Huginn afectados, `just huginn-architecture`, fmt y clippy pertinentes; revisar rollback por capa.
- [ ] 4.7 Documentación — validación documental: actualizar ADR-0007, `docs/huginn/architecture.md`, `implementation-specification.md` y `README.md`; comprobar catálogo, grants, defaults, fallos cerrados y que shell/MCP no están sandboxed.
- [ ] 4.8 Validación documental: revisar consistencia contra spec/diseño y que ninguna documentación afirme aislamiento del sistema operativo.

## Orden y control de apply

Completar cada capa con sus pruebas y documentación asociada antes de iniciar la siguiente. Registrar primero los artifacts SDD como commit convencional en la rama de integración; después actualizar la rama del slice 1 para que herede esa base. Encadenar los tres slices con las ramas indicadas y mantener el seguimiento en el documento RPI. El usuario autorizó commits locales convencionales por entrega. No usar metadatos de GitHub Stack ni hacer push/crear PRs sin autorización. Ninguna tarea autoriza persistir grants ni ampliar el catálogo ratificado.
