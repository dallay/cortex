# Informe de verificación: huginn-autonomous-workspace-permissions

**Versión:** N/A (delta OpenSpec)  
**Fecha:** 2026-11-09  
**Resultado actual:** PASS WITH WARNINGS  
**Alcance:** verificación técnica de la implementación y del contrato revisado; no es aceptación QA.

## Resumen ejecutivo

La fuente actual satisface los criterios revisados de las tareas 2.1–2.2. Cada salida controlada de `run_shell_with_seatbelt_profile` obtiene un resultado de ejecución y llama a `SeatbeltProfileFile::cleanup`; la composición conserva el error primario y el de cleanup cuando ambos existen. Para fallos de apertura/escritura previos al spawn, se intenta eliminar lo creado y se conserva el error primario junto con los errores de cleanup observables. El flujo no afirma que el shell no se ejecutó cuando pudo ejecutarse. `Drop` permanece best-effort, y no se exige un error de cierre no observable mediante la API safe de `std::fs::File`.

La suite actual del paquete pasó en macOS 27.3/build 26C5035r, y también pasaron clippy, formato, diff-check y los tests de contratos CI. El resultado lleva advertencias porque el enforcement real solo se observó en esa configuración, CI remoto y `just ci-local` no se ejecutaron, y los tests de composición/cleanup aislados no equivalen a inyectar una falla arbitraria del sistema operativo a través del launcher completo. Ese límite no contradice el diseño aprobado, que contempla probar la composición mediante helper y el cleanup explícito directamente.

El FAIL anterior se conserva como histórico. La nota histórica temprana de 3.2 ya está seguida por el cierre fechado más reciente que documenta la evidencia posterior y marca la tarea; no se alteró el historial.

## Completitud

| Métrica            | Valor |
|--------------------|------:|
| Entradas checklist |    23 |
| Completas          |    23 |
| Incompletas        |     0 |

Las entradas agrupadas (por ejemplo, 1.1–1.13) cuentan como una entrada de checklist, conforme al texto de `tasks.md`. Las tareas 2.1 y 2.2 están marcadas `[x]` y coinciden con el contrato aprobado. No se detectaron tareas pendientes.

## Build, pruebas y validaciones

### Ejecutadas en esta verificación

- `cargo test -p huginn-runtime` — **exit 0** en macOS 27.3/build 26C5035r; `/usr/bin/sandbox-exec` existe y es ejecutable. Resultados: lib 60/60; `adapter_security` 3/3; `approval_resource` 44/44; `coding_workflow` 34/34; `connections` 4 pasaron y 1 se ignoró por requerir Keychain/D-Bus externo; `guardrails` 8/8; `host_secrets` 1/1; `inference_adapters` 31/31; `mcp_lifecycle` 2/2; `providers_integration` 9/9; `session_locks` 6/6; `sessions_persistence` 5/5; `skill_loading` 10/10; `skills_catalog` 9/9; `stream_persistence` 4/4; `tool_authoring` 4/4; `tool_lifecycle` 12/12; `transports` 6/6; targets WASM y doc-tests sin casos. Cero fallos.
- `cargo clippy -p huginn-runtime --all-targets -- -D warnings` — **exit 0**.
- `cargo fmt --all -- --check` — **exit 0**.
- `git diff --check` — **exit 0**.
- `node --test .github/scripts/ci-*.test.cjs` — **exit 0**, 309/309.
- Inspección de la fuente actual: `run_shell`, `run_shell_with_seatbelt_profile`, `combine_shell_cleanup`, `cleanup_before_spawn`, `SeatbeltProfileFile::{cleanup,drop}`, creación/escritura del perfil, builder Seatbelt, workflow CI, spec, diseño, propuesta y tareas.

### Evidencia de apply revisada

`apply-progress.md`, sección “Reapertura de 2.1–2.2 bajo el contrato aprobado”, registra RED conductual real al reconstruir temporalmente la conducta anterior y restaurar la implementación: la composición pre-spawn perdió el error secundario de cleanup, y la post-spawn perdió el timeout y la posibilidad de que el shell hubiera corrido. Los tests de composición y de resultado exitoso con cleanup fallido pasaron después. El registro también informa 29 tests focalizados Seatbelt en lib y el mismo módulo en `approval_resource`, `cargo test -p huginn-runtime` exit 0 (60 lib, 44 `approval_resource`, demás suites pasaron, una prueba ignorada por requisito externo), clippy `-D warnings`, fmt y diff-check exitosos. Esta evidencia de apply se distingue de los comandos que se reejecutaron aquí.

### Omisiones y adaptadores

- No se ejecutó un build separado: `.agents/sdd/config.yaml` no configura `build_command`; no se presenta como pase.
- Coverage no está configurado (`coverage_threshold: 0`); no aplicable.
- `.agents/sdd/quality-runner.json` no existe: `fallback`; no hay enforcement determinista ni envelope del runner.
- No se ejecutaron `just ci-local` ni un run remoto de GitHub Actions. Los tests locales del workflow pasaron y el job macOS `test-huginn-seatbelt` aparece conectado al evaluador `CI / Required`; no se afirma que el job remoto haya pasado.
- No se ejecutó QA ni archive.

## Matriz de conformidad

| Requisito / escenario                                                      | Evidencia y resultado                                                                                                                                                                                                                                                                                                                                                                                                                                                |
|----------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| 2.1: RED para composición pre-spawn y post-spawn                           | **CONFORME.** Evidencia RED conductual registrada en apply: al restaurar temporalmente la composición previa, cada test falló por la información que debía conservar; la implementación corregida se restauró y los tests pasaron. No es una inferencia basada solo en la existencia de tests.                                                                                                                                                                       |
| 2.2: cleanup explícito para cada retorno controlado                        | **CONFORME.** El fuente obtiene el `Result` de `run_shell_with_launcher`, invoca `profile_file.cleanup()` y compone ambos. Cubre éxito, status no-cero/rechazo representado como salida observable, error de spawn, timeout, cancelación y errores de lectura/ejecución que retorna el helper. El test focalizado corre dentro de `cargo test -p huginn-runtime` y los tests de composición de helper pasaron.                                                       |
| Post-spawn: propagar error cleanup sin afirmar que el shell no corrió      | **CONFORME.** `combine_shell_cleanup` mantiene el error primario más cleanup failure; en resultado exitoso con cleanup error y en el caso combinado emite “may have run”. Tests `post_spawn_error_composition_preserves_primary_and_possible_execution` y `post_spawn_success_with_cleanup_error_does_not_claim_shell_was_skipped` pasaron.                                                                                                                          |
| Pre-spawn: no iniciar shell; cleanup y conservar error primario/secundario | **CONFORME.** La creación del directorio, apertura y escritura preceden al launcher. `open` y `write_all` fallidos invocan cleanup explícito de recursos creados y `cleanup_before_spawn` conserva ambos errores; fallo de mkdir no deja recursos creados. Tests cubren fallo de creación por padre no-directorio, writer fallido, tamaño, composición de errores y spawn fallido sin marcador. No se afirma haber inyectado una falla arbitraria del OS end-to-end. |
| Drop abrupto                                                               | **CONFORME.** `Drop` intenta `remove_file`/`remove_dir` y descarta sus errores, coherente con best-effort y sin garantía ante fallo OS. No se convierte este camino en salida controlada ni se exige unlink exitoso.                                                                                                                                                                                                                                                 |
| Escritura y cierre de archivo                                              | **CONFORME.** La escritura productiva usa `write_all`; test de writer fallido pasa. No se exige error independiente de `close` de `std::fs::File`, que no expone ese error mediante API safe.                                                                                                                                                                                                                                                                        |
| Aislamiento macOS, política deny-default y launcher obligatorio            | **PARCIAL / alcance observado.** Tests reales ejecutados por `run_shell` usan `/usr/bin/sandbox-exec`; el perfil inspeccionado importa `dyld-support.sb`, luego aplica `(deny default)`, limita rutas y no permite red/socket. La suite de enforcement real pasó en este host. No se generaliza a otras versiones/macOS futuros.                                                                                                                                     |
| Fallos de launcher/perfil y no ejecución pre-spawn                         | **CONFORME para casos probados.** Casos de perfil inválido, launcher ausente/no ejecutable, creación/escritura y tamaño tienen pruebas que pasan; marcadores ausentes en los casos correspondientes. No se extrapola a todos los fallos OS posibles.                                                                                                                                                                                                                 |
| Política/aprobación, no grants/bypass                                      | **CONFORME según inspección y evidencia apply.** No hay opt-out, fallback ni bypass en macOS; no se crean/leen grants. Tests de aplicación `Deny` y `Allow once` constan en apply-progress. No se atribuye a esos tests enforcement del kernel.                                                                                                                                                                                                                      |
| Herramientas nativas/MCP fuera de garantía                                 | **CONFORME según scope y documentación inspeccionados.** La garantía está limitada a `run_shell` y descendientes.                                                                                                                                                                                                                                                                                                                                                    |
| No paridad Linux/Windows                                                   | **CONFORME según inspección.** Launcher Seatbelt se condiciona a macOS y no se declara aislamiento equivalente en otras plataformas.                                                                                                                                                                                                                                                                                                                                 |
| Ciclo de vida y CI                                                         | **CONFORME en casos probados, limitado por entorno.** Suite actual incluye cwd/PATH/stdin/streams, límites, timeout/cancelación, descendientes, rutas y protocolos; tests CI locales 309/309. Enforcement observado solo en macOS 27.3/build 26C5035r, sin CI remoto.                                                                                                                                                                                                |

**Resumen:** los escenarios 2.1–2.2 cumplen el contrato revisado con tests runtime/helper y revisión del cableado productivo. El uso de helper para probar composición, y el test directo de cleanup, están explícitamente previstos por el diseño; no se exige una falla arbitraria de unlink inyectada end-to-end. La evidencia no afirma que tal fallo OS se haya probado por el launcher completo.

## Corrección estructural y coherencia con diseño

| Decisión                                                         | Estado                           | Evidencia                                                                                                                                 |
|------------------------------------------------------------------|----------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------|
| Builder puro y privado; preparación runtime de la raíz           | Conforme                         | `seatbelt.rs` separa generación/política del flujo runtime; `run_shell` usa raíz canónica para el perfil y `current_dir`.                 |
| `/usr/bin/sandbox-exec` fijo, `-f`, argv separados; fail-closed  | Conforme                         | Callsite macOS fija launcher y argumentos. No existe fallback sin sandbox para macOS.                                                     |
| Perfil temporal privado, permisos restrictivos, límite y cleanup | Conforme                         | Directorio `0700`, archivo `0600`, `create_new`, límite 64 KiB, `write_all`, cleanup explícito en salida controlada y `Drop` best-effort. |
| Seguridad del perfil                                             | Conforme en la evidencia probada | `(deny default)`, sin reglas de red/socket ni raíces amplias añadidas. Enforcement real solo en los probes y host indicados.              |
| Grupo/procesos descendientes                                     | Conforme en los casos observados | `ProcessGroup::leader`, `KillOnDrop`; tests timeout/cancelación pasan. No se amplía el claim más allá de esos casos.                      |
| Archivos y CI previstos                                          | Conforme                         | Runtime, docs, config de ejemplo, ADR y workflow corresponden al diseño. Los tests locales de workflow pasaron; no hubo ejecución remota. |

## Auditoría TDD

| Métrica                         | Estado                                                                                                                                                                                                |
|---------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Aplicabilidad                   | Comportamiento de runtime: aplica TDD.                                                                                                                                                                |
| RED→GREEN                       | Confirmado en apply para composición de errores pre/post-spawn mediante reconstrucción temporal de conducta previa; los asserts fallaron por la conducta incorrecta y pasaron restaurada la solución. |
| Refactor / validación posterior | Suite Seatbelt focalizada y suite completa del paquete pasaron; el código centraliza composición en helpers privados compartidos.                                                                     |
| Evidencia persistida            | Sí, en `apply-progress.md`; el informe distingue la ejecución registrada de las ejecuciones nuevas de verify.                                                                                         |

## Hallazgos

### CRÍTICOS

Ninguno para el contrato revisado de 2.1–2.2.

### ADVERTENCIAS

1. Enforcement real verificado solo en macOS 27.3/build 26C5035r. La interfaz Seatbelt está deprecada y la importación `dyld-support.sb` depende de Apple; no se afirma compatibilidad futura ni con otras versiones.
2. Los tests de composición sintetizan errores para verificar la preservación de errores; el test de cleanup comprueba propagación directamente. No se ha provocado un fallo arbitrario del OS de unlink dentro de una ejecución completa del launcher. El diseño revisado permite esta separación de pruebas, y la implementación inspeccionada conecta cleanup a cada retorno controlado.
3. No hubo CI remoto, `just ci-local`, build separado ni runner determinista. El resultado no equivale a CI completo.
4. Una prueba se ignora por requisito externo de Keychain o D-Bus. No corresponde al área modificada.

### SUGERENCIAS

Ninguna.

## Veredicto

**PASS WITH WARNINGS** — 2.1–2.2 satisfacen el contrato revisado y las 23 entradas de checklist están completas. Las advertencias limitan el alcance de plataforma y la evidencia operacional; no invalidan la conformidad de esas tareas. Este informe no sustituye aceptación QA independiente ni autoriza archive por sí mismo.
