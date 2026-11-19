# Tarea RPI: Diagnosticar el scan de descriptores en macOS

## Ruta

Delegated direct; depuración sistemática y cambio acotado al diagnóstico/test del escáner de descriptores para el PR #721. El usuario autorizó implementar el plan, incluyendo publicar la instrumentación y el arreglo para obtener evidencia del runner.

## Tareas

- [x] RPI-001: Identificar el origen exacto del `None` en los tres fallos de CI macOS 15.
- [x] RPI-002: Agregar una regresión determinista y aplicar el cambio mínimo sin relajar el contrato de no dejar descriptores abiertos.
- [x] RPI-003: Ejecutar formato, clippy y pruebas enfocadas localmente; publicar y verificar en CI.

## Criterios de aceptación

- Los errores que producen `None` quedan identificados por operación, sin registrar rutas ni payloads sensibles.
- El test de descriptor cerrado y los probes de publicación conservan aserciones concluyentes; no se degradan a aceptación de `None`.
- La corrección conserva clasificación segura: ningún error de inspección se interpreta como descriptor cerrado sin evidencia.
- Las pruebas enfocadas, `cargo fmt --all -- --check` y `git diff --check` pasan.
- No se realizan commits ni pushes sin autorización del usuario.

## Causa raíz

En macOS el identificador de dispositivo (`dev_t`) es **firmado**. Un socket abierto reporta `st_dev = -1`. El escáner reabrió cada descriptor por `/dev/fd/<slot>` y convertía el identificador con `u64::try_from(reopened.st_dev).ok()?`:

- Para un socket, el `try_from` falla y el escáner devolvía `None` **sin ningún diagnóstico**, porque la ruta era un `?` silencioso.
- El proceso de pruebas siempre tiene sockets abiertos (`socketpair` de `mpsc`, `UnixStream`), así que cualquier scan que clasificara **todos** los slots quedaba inconcluso.
- El test del FIFO pasaba porque corta en el primer slot que coincide (`Some(true)`) y nunca clasifica el resto; los tres tests que fallan exigen clasificar todos los slots.

Evidencia:

- `git show 228ecad8:...` muestra la conversión silenciosa `u64::try_from(reopened.st_dev).ok()?`.
- El log del run fallido `44493075911` no contiene ninguna línea `[c1-fs-test]`, y se verificó que `libtest` **sí** captura e imprime `eprintln!` en la sección de fallo (prueba en un proyecto aislado). La ausencia de diagnóstico prueba que el `None` no venía de `statat` ni de `fstat`.
- Reproducción local determinista: un `UnixStream::pair()` abierto hace que `descriptor_open_for` devuelva `None` en macOS.

## Arreglo

- `widen_device` ensancha el `dev_t` firmado con extensión de signo, igual que hace la biblioteca estándar (`MetadataExt::dev`), de modo que el identificador negativo de un socket sigue siendo comparable y el scan puede clasificarlo.
- Se conserva la detección de `BADF` como evidencia de descriptor cerrado; ningún error de inspección se interpreta como cerrado.
- Se añade instrumentación por operación (`descriptor.open_directory`, `descriptor.read_directory`, `descriptor.read_entry`, `descriptor.statat`, `descriptor.fstat`) y el resumen `descriptor.scan_inconclusive`, sin registrar rutas ni payloads.

## Evidencia de esta pasada

- RED: `descriptor_scan_stays_conclusive_with_a_socket_descriptor_open` falló con `left: None, right: Some(false)` antes del arreglo.
- GREEN: la misma prueba pasa tras el arreglo.
- `cargo fmt --all`: aplicado. `cargo clippy --locked -p muninn-c1-exporter --features cpu-f32 --all-targets --target aarch64-apple-darwin -- -D warnings`: sin hallazgos en el crate.
- `cargo test --locked -p muninn-c1-exporter --features cpu-f32 --target aarch64-apple-darwin`: 136 + 10 pruebas, 0 fallidas, 4 ignoradas.
- Estrés: 15 iteraciones consecutivas de la suite del crate, todas en verde.
- `git diff --check`: limpio.

## Limitación conocida

La rama de Linux no puede clasificar un socket: `/proc/self/fd/<slot>` apunta a `socket:[<n>]`, el destino no existe como ruta y el escáner conserva `None` de forma honesta. El crate no se prueba en Linux en CI (el job `Test (Muninn C1 opt-in)` corre solo en `macos-15`). Queda como seguimiento fuera de este alcance.

## Estado

Ready — causa raíz confirmada, regresión determinista en verde y arreglo mínimo aplicado. Pendiente la verificación en el runner macOS 15 de CI.

## Siguiente paso

Publicar el arreglo y el diagnóstico en la rama del PR, confirmar que el job `Test (Muninn C1 opt-in)` pasa de forma repetida y dejar constancia en la descripción del PR. El fallo separado de `cargo audit` (`wasmtime`, `unsafe-libyaml`, `zbus_names`) no se toca en este alcance.
