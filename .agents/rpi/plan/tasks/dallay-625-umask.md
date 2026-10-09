# DALLAY-625: Respetar el umask al crear archivos

## Ruta
Delegated direct: atender el fallo Reliability de Sonar y reducir complejidad de `NativeTool::execute` dentro del mismo PR DALLAY-625, preservando el flujo atómico y permisos existentes.

## Tareas
- [x] RPI-001: Cambiar modo temporal para archivos nuevos en Unix a creación normal con `0666`, dejando que el kernel aplique umask; no cambiar comportamiento no-Unix.
- [x] RPI-002: Añadir prueba aislada para umask restrictivo `077` (y `027`) sin mutar el umask global del proceso de pruebas.
- [x] RPI-003: Ejecutar pruebas enfocadas y revisar diff.
- [x] RPI-004: Leer permisos existentes antes de crear el temporal y usarlos como modo de creación; añadir test de reemplazo.
- [x] RPI-005: Evaluar alternativa sin shell; se mantiene el wrapper porque cambiar umask sin shell requiere `unsafe`, prohibido por lint, y los argumentos del wrapper son constantes controladas por el test.
- [x] RPI-006: Hacer asíncrono el test de subprocesses umask usando `tokio::process::Command` sin modificar el umask global del proceso de pruebas.
- [x] RPI-007: Reducir complejidad de `NativeTool::execute` extrayendo helpers, preservando comportamiento.
- [ ] RPI-008: Actualizar evidencia y subir los cambios a PR #299; confirmar resultado del nuevo Quality Gate.

## Criterios de aceptación
- Archivos nuevos respetan el umask en Unix.
- Reemplazar conserva permisos del destino.
- Se mantiene temporal + rename atómico.
- La prueba de `077` no muta el umask global en el proceso de pruebas concurrente.
- Comportamiento no-Unix permanece sin cambios o queda documentado.

## Evidencia
- `cargo test -p agent-runtime --test coding_workflow` — pasó: 18 tests; incluye subprocesses aislados que verifican umask `077` y `027`, y prueba de preservación de permisos.
- `cargo clippy -p agent-runtime --tests -- -D warnings` — pasó.
- `cargo fmt --check`, `git diff --check` y Markdownlint enfocado — pasaron.
- Refactor de dispatch documentado en `docs/agent/implementation-specification.md` y diagrama Mermaid actualizado.
- Pendiente: enviar cambios al PR #299 y confirmar resultado del Quality Gate remoto.

## Estado
Working — corregidos localmente ambos hallazgos de Sonar. Esperando commit/push y nueva evaluación de PR.

## Siguiente paso
Revisar diff, commit y push a la rama del PR #299.