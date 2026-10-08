# DALLAY-625: Respetar el umask al crear archivos

## Ruta
Delegated direct: corregir creación de archivos nuevos en `write_file`, preservando el flujo atómico y los permisos de archivos existentes.

## Tareas
- [x] RPI-001: Cambiar modo temporal para archivos nuevos en Unix a creación normal con `0666`, dejando que el kernel aplique umask; no cambiar comportamiento no-Unix.
- [x] RPI-002: Añadir prueba aislada para umask restrictivo `077` (y `027`) sin mutar el umask global del proceso de pruebas.
- [x] RPI-003: Ejecutar pruebas enfocadas y revisar diff.
- [x] RPI-004: Leer permisos existentes antes de crear el temporal y usarlos como modo de creación; añadir test de reemplazo.

## Criterios de aceptación
- Archivos nuevos respetan el umask en Unix.
- Reemplazar conserva permisos del destino.
- Se mantiene temporal + rename atómico.
- La prueba de `077` no muta el umask global en el proceso de pruebas concurrente.
- Comportamiento no-Unix permanece sin cambios o queda documentado.

## Evidencia
- `cargo test -p agent-runtime --test coding_workflow` — pasó: 18 tests; incluye subprocesses aislados que verifican umask `077` y `027`, y prueba de preservación de permisos.
- `git diff --check` — pasó.
- La documentación del flujo atómico en `docs/agent/implementation-specification.md` ahora especifica la aplicación del umask y contiene un diagrama Mermaid con las ramas Unix/no-Unix y preservación de permisos.

## Estado
Ready — nuevas escrituras Unix usan modo `0666` sujeto al umask del kernel; reemplazos preservan permisos existentes; temporal + rename atómico se mantiene; no se modifica el umask global.

## Siguiente paso
Revisar el diff completo; cambios listos para integración.