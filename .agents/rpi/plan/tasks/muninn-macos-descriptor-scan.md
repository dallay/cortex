# Tarea RPI: Diagnosticar el scan de descriptores en macOS

## Ruta

Delegated direct; depuración sistemática y cambio acotado al diagnóstico/test del escáner de descriptores para el PR #721. El usuario autorizó implementar el plan, incluyendo publicar la instrumentación necesaria para obtener evidencia del runner; se respetan los hooks.

## Tareas

- [ ] RPI-001: Confirmar en runner macOS 15 el origen exacto de `None` en los tres fallos mediante evidencia por operación.
- [ ] RPI-002: Después de confirmar causa, agregar una regresión determinista y aplicar el cambio mínimo sin relajar el contrato de no dejar descriptores abiertos.
- [x] RPI-003: Ejecutar formato y pruebas enfocadas localmente; anotar validación y limitación de CI.

## Criterios de aceptación

- Los errores que producen `None` quedan identificados por operación, sin registrar rutas ni payloads sensibles.
- El test de descriptor cerrado y los probes de publicación conservan aserciones concluyentes; no se degradan a aceptación de `None`.
- La corrección conserva clasificación segura: ningún error de inspección se interpreta como descriptor cerrado sin evidencia.
- Las pruebas enfocadas, `cargo fmt --all -- --check` y `git diff --check` pasan.
- No se realizan commits ni pushes.

## Evidencia inicial

- Head local/remoto: `228ecad87c43aed0872a0f6c2988f9c38e281c3c`; worktree limpio.
- Último CI: 130 passed, 3 failed, 4 ignored. Los tres fallos obtienen `None`; la instrumentación anterior solo cubría `statat` y `fstat` y no mostró errno.
- Código con retornos inconclusos aún sin instrumentar: apertura del descriptor dir (`open(...).ok()?`), `read_dir(...).ok()?`, error de iterador y conversión de `st_dev`.
- Estado al inicio: `origin/main` contiene 5 commits que no están en la rama; no se integrará ni rebasará durante este alcance.

## Evidencia de esta pasada

- Instrumentados `descriptor.open_directory`, `descriptor.read_directory`, `descriptor.read_entry` y la conversión de `st_dev`; no se registran rutas ni payloads. Se conserva la instrumentación previa de `statat` y `fstat`.
- `cargo fmt --all -- --check`: pasó.
- Suite `cargo test --locked -p muninn-c1-exporter --features cpu-f32 --target aarch64-apple-darwin --lib`: 133 passed, 0 failed, 4 ignored. Los tres tests que fallaron en CI también pasan individualmente.
- `git diff --check`: pasó.
- Diagnóstico: el job remoto `44493075911` terminó antes de esta instrumentación y no se reproduce localmente; la raíz sigue sin confirmar.
- No se modificó la clasificación Linux para el error de la segunda comprobación `statat(..., SYMLINK_NOFOLLOW)`.
- No se ejecutó CI remoto porque requiere publicar el cambio y no se hizo commit ni push.

## Estado

Needs your decision — validación local verde; falta evidencia del runner macOS 15 para identificar la causa y confirmar cualquier arreglo funcional.

## Siguiente paso

Autorizar commit y push de la instrumentación diagnóstica para disparar un job macOS 15, o indicar un mecanismo alternativo para ejecutar el cambio en ese runner. Con evidencia, confirmar raíz y completar RPI-001/002 mediante regresión y TDD. No modificar dependencias por el fallo separado de `cargo audit`.
