# Apply progress — launcher/ciclo de vida (tareas 2.1–2.4)

Fecha: 2026-11-09. Worktree preexistente y dirty por la misma implementación; no se revirtieron ni reemplazaron sus cambios.

## Alcance ejecutado

Se añadieron pruebas macOS a `crates/huginn/runtime/src/tools.rs` para cwd, PATH seleccionado, stdin EOF, separación stdout/stderr, salida de tamaño alto limitada y drenada, y fallo de preparación de workspace que no inicia el comando. Se preservaron los argumentos separados del launcher existente y el ciclo timeout/cancelación/ProcessGroup.

## Evidencia de TDD

No se observó un RED válido del comportamiento: el primer intento falló por literales de newline incorrectos en las aserciones; el segundo detectó un formato esperado demasiado estricto y un comando de fixture que dependía de `yes`/`head`, no permitido por el perfil; el siguiente intento expiró por emitir 2 MB usando builtins en 10 s. Se corrigieron las aserciones y el fixture, pero estas fallas no se cuentan como RED de producto. No se añadió un override de launcher, perfil ni fallo de filesystem a producción solo para forzar RED.

El límite se verifica con una salida de 100 KB por cada stream: `run_shell_bounds_captured_output_while_draining_both_streams` pasó y la salida final no superó `MAX_OUTPUT_BYTES`, con contenido de ambos streams. El código existente limita cada vector durante el drenaje y sigue leyendo hasta EOF.

## Comandos y resultados

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: primera corrida con fixture/assertions defectuosos, falló; corrida corregida pasó (7 unitarios en lib y 7 en `approval_resource.rs`; integraciones restantes filtradas sin casos ejecutados).
- `cargo test -p huginn-runtime`: pasó sin filtro. 38 unit tests, 3 adapter_security, 22 approval_resource, 34 coding_workflow, 4 connections (1 ignorada), 8 guardrails, 1 host_secrets, 31 inference_adapters, 2 mcp_lifecycle, 9 providers_integration, 6 session_locks, 5 sessions_persistence, 10 skill_loading, 9 skills_catalog, 4 stream_persistence, 4 tool_authoring, 12 tool_lifecycle, 6 transports y doc-tests (0); sin fallos.
- `cargo fmt --all -- --check`: pasó.
- `git diff --check`: pasó.

## Límites/bloqueos de esta entrega

- No se validó de forma adversarial el launcher ausente/no ejecutable, rechazo de perfil o fallos de creación/escritura/cierre/limpieza con marcador. El ejecutable de producción es fijo y no hay un seam seguro existente para inyectar esos fallos. No se agregó un override de producción solo para pruebas; la cobertura de 2.1 y parte de 2.2 quedan bloqueadas.
- `SeatbeltProfileFile::drop` ignora errores de `remove_file`/`remove_dir`; por tanto, la limpieza exitosa no es una condición observable de la operación y el requisito de fallo cerrado ante error de limpieza no está demostrado.
- El test de workspace inválido demuestra fallo antes de spawn a partir de `prepare_workspace`, pero no simula un error de spawn del OS ni fallo de aplicación del perfil.
- La salida respeta un límite acotado de memoria durante captura. No se probaron simultáneamente las fallas de captura de stdout/stderr ni todas las variantes del límite.
- Las pruebas adicionales caracterizan comportamiento ya presente; al no existir RED válido y no hacerse cambio productivo, no se marcan 2.1–2.4 como completas en `tasks.md`.

## Estado

Parcial. Tareas 2.1–2.4 siguen sin marcar. Siguiente paso: definir un seam de prueba que no cambie la superficie de producción o acordar cómo validar los errores reales de launcher/perfil; no avanzar a tareas 3.x ni 4.x desde este slice.

## Revisión acotada 2.1–2.2 (2026-11-09)

- Se añadió `sandbox_exec_rejects_invalid_profile_before_shell_can_write_marker` en el módulo de tests macOS. Invoca directamente el `/usr/bin/sandbox-exec` real con un archivo SBPL inválido y un comando shell que escribiría un marcador. El proceso devolvió código 65, informó `unbound variable` junto a la ruta `invalid.sb`, y el marcador no apareció. El rechazo del parser fue inequívoco. El test no usa launcher configurable ni altera el perfil/entorno de producción.
- Primera corrida focalizada: falló solo porque la aserción esperaba la palabra `profile`/`syntax`, mientras macOS 27.3 informa `unbound variable`; no se cuenta como RED de producto. Tras corregir la aserción al diagnóstico observado, la prueba focalizada pasó. No se hizo cambio de lógica de producción; por tanto, no se reclama ciclo TDD RED/GREEN para comportamiento productivo.
- Esta prueba acredita el rechazo del archivo inválido por el launcher real y ausencia de ejecución del shell en ese caso. No prueba que `run_shell` entregue el archivo inválido, que traduzca tal rechazo a `Err`, ni fallos de spawn o de filesystem. No se factoriza helper ni se añade override de launcher: el rechazo se prueba contra el binario fijo real, y las tareas 2.1–2.2 permanecen incompletas.
- Limpieza: `SeatbeltProfileFile::drop` continúa siendo best-effort, ya que `Drop` no puede propagar errores. El test no fuerza fallos de unlink. No se afirma garantía absoluta de eliminación si el sistema operativo rechaza el unlink. La limpieza explícita con propagación para salidas controladas sigue sin implementarse/verificarse; drop/cancelación conserva solo best-effort.
- No se modificaron SBPL, grants, permisos ni tareas 2.3–2.4 o fases posteriores. `tasks.md` queda sin checks para 2.1–2.2: la evidencia cubre únicamente una parte de 2.1.

### Validación de esta revisión

- `cargo test -p huginn-runtime sandbox_exec_rejects_invalid_profile_before_shell_can_write_marker -- --nocapture`: pasó (1 test de lib; filtro también encontró el test replicado por integración).
- `cargo test -p huginn-runtime`: pasó sin filtro (39 unit tests; integración y doc-tests pasaron; 1 test ignorado por requisito externo).
- `cargo fmt --all -- --check`: pasó después de aplicar `cargo fmt --all`.
- `git diff --check`: pasó.

## Diagnóstico acotado de sockets UDP/TCP/Unix (2026-11-09)

### Cambio del harness

- Solo se cambió el test UDP en `crates/huginn/runtime/src/tools.rs`: el listener host y el listener sandbox ahora hacen `recv_from` en loop con timeout determinista de 3 s, acumulan cada datagrama y terminan al recibir exactamente `udp-probe` o vencer el plazo. Ambos resultados se registran en `--nocapture`.
- El control host recibió `[[88], [88], [88], [88], [117, 100, 112, 45, 112, 114, 111, 98, 101]]`, es decir, cuatro datagramas `X` y después `udp-probe`. El control cuenta: payload real recibido. No se ensayó alternativa de `nc`/stdin, porque el control sí transmitió el payload esperado.

### Resultados

- **TCP:** `cargo test -p huginn-runtime run_shell_seatbelt_denies_tcp_connect_after_host_control_succeeds -- --nocapture` pasó en lib y `approval_resource`. Host: `Connection to 127.0.0.1 port <puerto> [tcp/*] succeeded!`; sandbox usa `nc -v -w 1`, devuelve status normal 1 y `nc: connectx to 127.0.0.1 port <puerto> (tcp) failed: Operation not permitted`; listener sandbox no aceptó. También aparece el error independiente de lectura `/private/var/select/sh`.
- **UDP:** `cargo test -p huginn-runtime run_shell_seatbelt_denies_udp_datagram_after_host_control_succeeds -- --nocapture` pasó en lib y `approval_resource`. Host control recibió los cuatro `X` y el payload exacto. Sandbox usa `nc -v -u -w 1`, devuelve status normal 1 y `nc: connectx to 127.0.0.1 port <puerto> (udp) failed: Operation not permitted`; listener no recibió datagramas. También aparece el error independiente de lectura `/private/var/select/sh`.
- **Unix/local:** `cargo test -p huginn-runtime run_shell_seatbelt_denies_unix_socket_connect_after_host_control_succeeds -- --nocapture` falló deliberadamente en lib. Host control se conectó y listener aceptó. Sandbox usa `nc -v -U ... -w 1`; status normal 1, listener no aceptó, pero stderr solo contiene `Error opening /private/var/select/sh: Operation not permitted` y no un diagnóstico de `nc`/socket. La prueba no cuenta esta ausencia como denial OS y permanece fallida/ambigua. La observación del listener ahora se imprime antes de la aserción diagnóstica.

### Validación

- Los tres tests enfocados se ejecutaron. TCP y UDP pasaron; Unix falló por falta de diagnóstico atribuible a la operación, después de confirmar control host positivo y listener sandbox sin aceptación.
- `cargo fmt --all -- --check`: pasó.
- `git diff --check`: pasó.
- No se ejecutó la suite completa del paquete. No se tocaron producción, SBPL, perfil, policy, permisos, fases 2 ni tarea 3.1. Task 3.2 permanece unchecked; faltan resultado Unix concluyente y escenarios de descendientes.

## Diagnóstico de ambigüedad TCP 3.2 (2026-11-09)

- Se añadió una prueba solo diagnóstica que ejecuta `/usr/bin/nc -h` vía `run_shell` real y control host, con marcadores antes y después, y conserva los outputs completos en el log de `--nocapture`. Bajo sandbox el resultado fue status shell 0; stdout `before-nc` y `after-nc-status=1`; stderr incluyó `Error opening /private/var/select/sh: Operation not permitted` y después el texto completo de ayuda de `nc`. El control host dio status shell 0 y texto completo de ayuda, sin el error de lectura. El uso aparece en stderr; `nc -h` retorna status 1 dentro del shell.
- Conclusión limitada: `/usr/bin/nc` sí inicia y procesa `-h` bajo el profile. Esto refuta la forma general de la hipótesis (1), pero no que la operación de conexión tenga un fallo de inicialización específico.
- El probe TCP controlado y el sandboxed usan la misma forma `/usr/bin/nc -v -w 1 127.0.0.1 <puerto>`. El host produjo `Connection to 127.0.0.1 port <puerto> [tcp/*] succeeded!`, status 0, y el listener aceptó. En sandbox el shell alcanzó `shell-started`, terminó normalmente con status 1 y stderr solo tuvo `Error opening /private/var/select/sh: Operation not permitted`; el listener no aceptó durante los 3 s de observación. La aserción sigue fallando porque no hay diagnóstico propio de `nc` que atribuya el fallo a una denegación de socket.
- Resultado de hipótesis: (1) no explica la incapacidad general de arrancar `nc`; una falla de init específica de conexión sigue posible. (2) sigue posible pero no hay mensaje alternativo de `nc` que la pruebe. (3) la forma de invocación host/sandbox ahora es idéntica y el control host valida listener; no se observó aceptación sandboxed, pero esto no determina si hubo otro fallo previo al socket.
- Sin cambio de producción/SBPL/policy/allow rules. Los tests TCP, UDP y Unix siguen fallando en outcome ambiguo, y las capacidades de red/socket permanecen sin marcar como comprobadas.

### Validación del diagnóstico

- `cargo test -p huginn-runtime run_shell_diagnoses_nc_startup_under_seatbelt -- --nocapture`: pasó, tanto en lib como en `approval_resource`; salida completa de ayuda observada en ambos contextos.
- `cargo test -p huginn-runtime run_shell_seatbelt_denies_tcp_connect_after_host_control_succeeds -- --nocapture`: falló intencionalmente en la aserción de diagnóstico `nc`; control host aceptado, listener sandbox no aceptó, output sandbox sin diagnóstico de `nc`.
- `cargo test -p huginn-runtime run_shell_seatbelt_denies -- --nocapture`: 0/3 pasó; TCP, UDP y Unix fallaron por falta de diagnóstico de denegación. Ninguno se cuenta como OS denial.
- `cargo fmt --all -- --check`: inicialmente encontró únicamente formato del test diagnóstico; se aplicó `cargo fmt --all`. Falta reejecutar el check final.
- `git diff --check`: pasó.

## Continuación limitada: pruebas de sockets 3.2 (2026-11-09)

### RED y controles

- Se añadieron tres pruebas macOS directamente al módulo de integración de `run_shell`. Cada prueba crea un listener host con timeout acotado, verifica primero un control positivo sin sandbox con `/usr/bin/nc` y exige que el listener observe la conexión TCP, el datagrama UDP `udp-probe` o la conexión Unix local. El fixture usa puertos efímeros y `tempfile::TempDir`; no se copió el binario.
- El mismo probe `/usr/bin/nc` se envía después por `run_shell`, que prepara el profile del producto y usa el launcher real `/usr/bin/sandbox-exec`. Se exige el sentinel `shell-started`, status shell normal no-cero y un diagnóstico propio de `nc` que contenga `Operation not permitted`. Sin ello, el test falla como resultado ambiguo; ausencia de listener por sí sola no aprobaría denial.
- RED observado: los controles positivos pasaron para TCP, UDP y Unix/local. En cada ejecución sandboxed apareció `shell-started`, `Exit: exit status: 1` y stderr `Error opening /private/var/select/sh: Operation not permitted`. No apareció `nc:` ni diagnóstico de protocolo. Por tanto, la prueba no distingue si `nc` llegó a iniciar; no se afirma enforcement de TCP/UDP/Unix y no se acepta como GREEN. La ruta denegada sugiere una dependencia de lectura del arranque, pero la evidencia no identifica qué proceso la intentó ni demuestra por sí sola que sea `nc`.
- El perfil no se modificó; no se agregó permiso de red/socket, `dyld-support.sb` ni allowance para `/tmp`. No se modificó `tasks.md` para marcar 3.2 como completa. La prueba no cubrió descendientes, que sigue pendiente en 3.2.

### Resultados por protocolo

| Protocolo  | Control host                                                    | Resultado por `run_shell`                                                                                                               | Estado                                   |
|------------|-----------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------|
| TCP        | `/usr/bin/nc` conectó y listener aceptó                         | Sentinel; salida normal 1; error de lectura `/private/var/select/sh`; sin mensaje propio de `nc`; no se pudo medir aceptación sandboxed | Bloqueado/ambiguo; no cuenta como denial |
| UDP        | `/usr/bin/nc` envió `udp-probe`; listener recibió bytes exactos | Sentinel; salida normal 1; mismo error de lectura; sin mensaje propio de `nc`; no se pudo medir recepción sandboxed                     | Bloqueado/ambiguo; no cuenta como denial |
| Unix/local | `/usr/bin/nc -U` conectó y listener aceptó                      | Sentinel; salida normal 1; mismo error de lectura; sin mensaje propio de `nc`; no se pudo medir aceptación sandboxed                    | Bloqueado/ambiguo; no cuenta como denial |

### Comandos y validación

- `cargo test -p huginn-runtime run_shell_seatbelt_denies -- --nocapture`: RED intencional, 3 pruebas fallidas por no observar diagnóstico propio de `nc`; los tres controles host llegaron al listener. Las fallas son incompatibilidad/ambigüedad del probe bajo profile, no denegaciones aceptadas.
- `cargo test -p huginn-runtime`: 48 unit tests pasaron, los 3 nuevos escenarios fallaron por la misma condición ambigua; sin otros fallos registrados antes de finalizar el target.
- `cargo fmt --all -- --check`: pendiente al actualizar este registro.
- `git diff --check`: pendiente al actualizar este registro.

### Estado y siguiente paso

Parcial/bloqueado. Task 3.2 permanece sin marcar; TCP, UDP y Unix/local tienen control positivo válido, pero ninguno tiene evidencia concluyente de denegación del protocolo bajo el profile. No avanzar a CI/docs/3.1 ni ampliar permissions. Resolver en un slice autorizado cómo ejecutar un probe conocido bajo este profile sin ampliar permisos; si file-read del probe lo impide, registrar bloqueo sin añadir allowance.

## Seguimiento de helper privado 2.1–2.2 (2026-11-09)

- Antes del refactor, characterization baseline: `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` pasó. En macOS ejecutó 8 tests del módulo en lib y los mismos 8 en `approval_resource`; el test directo confirmó status 65, diagnóstico `unbound variable` con `invalid.sb` y ausencia del marcador. El flujo real de `run_shell` todavía no recibió el perfil inválido.
- Se extrajo `run_shell_with_launcher` para compartir el ensamblado y ciclo de vida del proceso. `run_shell_with_seatbelt_profile` es privado y macOS-only: prepara el archivo temporal y llama al mismo runner con `/usr/bin/sandbox-exec` fijo. El callsite productivo conserva `prepare_workspace` + `build_profile`; no hay configuración, env, argumentos públicos ni fallback.
- Se añadió un test que entrega SBPL inválido por el helper privado y exige `Err` y marcador ausente. Primera ejecución falló de forma natural: sandbox-exec devolvió el status de rechazo 65, pero el runner lo formateaba como salida normal. Se añadió conversión de status 65 a `AgentError::Tool`; al repetir, el test pasó. Esto es evidencia RED/GREEN del caso observable, aunque el resto del cambio es una extracción behavior-preserving. No se añadió un RED artificial.
- No se tocaron limpieza, seam de error de spawn ni tareas 2.3–2.4. La matriz 2.1–2.2 sigue incompleta: no se probaron launcher ausente/no ejecutable, errores de creación/escritura/cierre/limpieza ni todas las formas de rechazo; por ello ambas tareas permanecen unchecked. `SeatbeltProfileFile::drop` sigue siendo best-effort.

### Validación posterior al refactor

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: pasó; 9 tests en lib y 9 en `approval_resource`, incluyendo el rechazo vía helper.
- `cargo test -p huginn-runtime` (sin filtro): pasó. Conteos: lib 40; adapter_security 3; approval_resource 24; coding_workflow 34; connections 4 passed/1 ignored; guardrails 8; host_secrets 1; inference_adapters 31; mcp_lifecycle 2; providers_integration 9; session_locks 6; sessions_persistence 5; skill_loading 10; skills_catalog 9; stream_persistence 4; tool_authoring 4; tool_lifecycle 12; transports 6; wasm_outline 0; wasm_output_validation 0; doc-tests 0. Sin fallos.
- `cargo fmt --all -- --check`: pasó.
- `git diff --check`: pasó.

Estado de esta unidad: parcial. Tareas 2.1 y 2.2 siguen sin marcar; no avanzar a 2.3–2.4 ni fase 3 en este slice.

## Revisión del status 65 (2026-11-09)

- Causa raíz verificada en `run_shell_with_launcher`: cualquier status 65 en macOS retorna `AgentError::Tool` antes de formatear stdout/stderr. El status no identifica por sí solo al parser de SBPL: el comando shell puede terminar con el mismo código.
- Se añadió `run_shell_preserves_shell_exit_code_65_with_valid_seatbelt_profile`, que genera un perfil con `crate::seatbelt` y ejecuta `exit 65` por el helper privado. RED reproducido y guardado: la prueba falla en `.expect(...)` con `Tool("sandbox-exec rejected the shell profile or command")`.
- Evidencia macOS 27.3 con `/usr/bin/sandbox-exec`: un SBPL inválido devuelve 65 y stderr de parser con `sandbox-exec: unbound variable ... at <invalid.sb>, line 1, column 1` y ubicación del token; el SBPL válido con shell `exit 65` también devuelve 65, pero stderr contiene mensajes de ejecución (`Error opening .../var/select/sh` y error `getcwd` en esa ejecución), no el diagnóstico del parser. El test del helper también confirma el caso válido como status 65 ambiguo para el runner.
- No se implementa discriminación usando patrones de stderr: no hay evidencia de contrato estable/documentado de Seatbelt para ese formato, y stderr es compartido con el hijo, que puede emitir texto arbitrario. El código de salida tampoco discrimina. No se relaja el comportamiento fail-closed ni se adivina.
- Decisión: detener el fix y reportar el dilema. La regresión queda añadida y actualmente RED; el test existente de rechazo inválido mediante helper sigue pasando. La matriz de tareas 2.1/2.2 está incompleta, sin checks. Se requiere acordar/probar un discriminador estable antes de continuar; no se modificaron allow rules, limpieza ni otras tareas.

### Validación

- `cargo test -p huginn-runtime run_shell_preserves_shell_exit_code_65_with_valid_seatbelt_profile -- --nocapture`: falla como RED esperado por la conversión blanket de status 65.
- `cargo test -p huginn-runtime run_shell_returns_error_when_seatbelt_rejects_profile_without_running_shell -- --nocapture`: pasa (test de lib y duplicado en `approval_resource`).
- `cargo fmt --all -- --check`: pasa después de formato; `git diff --check`: pasa.
- No se ejecutó la suite completa del paquete porque no se implementó cambio productivo y el test de regresión está deliberadamente RED.

## Resolución de status 65 (2026-11-09)

- Cambio mínimo: `run_shell_with_launcher` ya no interpreta un status 65 como `AgentError`. No existe un discriminador fiable entre el status del parser de sandbox-exec y el status de salida del shell. El status y stdout/stderr del proceso quedan en el resultado observable estándar (`Exit: exit status: 65`, `stdout`, `stderr`); los fallos de spawn siguen propagándose como `Err`.
- El test de rechazo vía helper ahora exige salida status 65, el diagnóstico observado `unbound variable`/`profile.sb` y ausencia del marcador. Este output es el resultado observable del rechazo, no una ejecución aceptada del shell: el launcher rechazó el perfil antes del shell. El test con perfil válido y `exit 65` también exige el mismo status como salida normal.
- RED/GREEN: el RED de `exit 65` bajo el código anterior está documentado arriba; el usuario también informó la baseline del test de rechazo. Después del cambio ambos tests pasan en macOS 27.3, tanto en lib como en `approval_resource`.
- No se inspecciona stderr en producción ni se modifican SBPL, permisos, cleanup o fallback. El rechazo continúa fail-closed porque `sandbox-exec` no inicia el shell con perfil inválido; la API representa el status como salida observable, no como `Err`.
- `tasks.md` conserva 2.1 y 2.2 sin completar: persiste la matriz incompleta de launcher ausente/no ejecutable y fallos de creación, escritura, cierre y limpieza, entre otros escenarios. No se avanzó a otras tareas.

### Validación de la resolución

- `cargo test -p huginn-runtime run_shell_reports_seatbelt_profile_rejection_without_running_shell -- --nocapture`: pasó; 1 test en lib y su duplicado en `approval_resource`.
- `cargo test -p huginn-runtime run_shell_preserves_shell_exit_code_65_with_valid_seatbelt_profile -- --nocapture`: pasó; 1 test en lib y su duplicado en `approval_resource`.
- `cargo test -p huginn-runtime`: pasó sin filtro; 41 lib, 25 approval_resource, demás integraciones pasaron, 1 test ignorado por requisito externo y doc-tests pasaron.
- `cargo fmt --all -- --check` y `git diff --check`: pasaron.

## Continuación acotada: spawn, límite y cleanup (2026-11-09)

- Se añadió un test macOS que invoca el helper privado con un launcher absoluto inexistente y un comando marcador. Resultado: `Err` de spawn y marcador ausente. No se añadió configuración del launcher ni override de entorno. El test caracteriza una propagación de error ya existente.
- Se añadió un test del límite superior a `run_shell_with_seatbelt_profile`: un perfil de 64 KiB + 1 byte se rechaza durante preparación y el marcador del shell permanece ausente. El límite ya existía; el test caracteriza el rechazo antes del spawn.
- `SeatbeltProfileFile` ahora tiene `cleanup(self) -> io::Result<()>`: elimina el perfil y después el directorio, propagando errores. `run_shell_with_seatbelt_profile` llama cleanup después de cada resultado controlado (éxito, status de rechazo/error, timeout o cancelación). Si cleanup falla tras posible ejecución, el mensaje indica explícitamente que el comando puede haber corrido; no afirma que no hubo ejecución ni inicia un fallback. `Drop` sigue haciendo best-effort como respaldo para drop/cancelación abruptos.
- Tests del método `cleanup`: eliminación normal comprueba que perfil/directorio ya no existen; una entrada extra en el directorio hace fallar la eliminación del directorio y el error se propaga. La primera corrida con esos tests falló al compilar porque `cleanup` no existía. Esto demuestra la ausencia de la API antes de GREEN, pero no es un RED de aserción conductual; no se reclama ciclo TDD RED válido para este cambio productivo.
- No se pudo forzar de forma determinista un error de cleanup dentro del flujo que ejecuta el launcher. El test verifica el propagador y el wiring se revisó en `run_shell_with_seatbelt_profile`; no se simularon unlink fallido durante ejecución, fallo de creación/escritura/cierre, ni launcher no ejecutable. El error de directorio no vacío valida fallo de `remove_dir`, no denegación del SO.
- No se cambiaron SBPL, permisos, `status 65`, tests de fase 3 ni tareas 2.3–2.4. 2.1 y 2.2 permanecen unchecked porque no se cubre su matriz completa.

### Validación de esta continuación

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: pasó, 14 tests en lib y 14 duplicados en `approval_resource` (incluye spawn, límite y cleanup).
- `cargo test -p huginn-runtime` sin filtro: pasó; 45 unit tests, 4 connections passed/1 ignored, y todas las suites de integración pasaron; doc-tests pasaron.
- `cargo fmt --all -- --check` y `git diff --check`: pasaron después de `cargo fmt --all`.
- Reejecución final de `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: pasó 14/14 en lib y 14/14 en `approval_resource`.

## Continuación acotada: fallos comprobables de perfil y launcher (2026-11-09)

- Se añadió una prueba macOS con un launcher existente creado en un directorio temporal y modo `0600` (sin bit ejecutable). El helper privado de launcher devuelve `Err` de spawn y el marcador permanece ausente. El fixture queda bajo `tempfile::TempDir`, que limpia en éxito y al desenrollar el test.
- Se extrajo `create_seatbelt_profile_under(profile, temp_root)`, privado y usado por producción mediante `create_seatbelt_profile(profile)`, que siempre le pasa `std::env::temp_dir()`. La prueba suministra como raíz un archivo regular y obtiene `io::ErrorKind::NotADirectory` al crear el directorio del perfil. No hay variable de entorno ni seam público/configurable.
- Se extrajo `write_seatbelt_profile(&mut impl Write, bytes)`, usado por el camino productivo. `FailingWriter` devuelve un error determinista y el test verifica que `write_all` lo propaga. El flujo productivo solo llama al launcher después de que la creación/escritura del perfil devuelve `Ok`; el test de spawn independiente comprueba también que `Err` de lanzamiento no ejecuta el comando. No se inyecta un writer ni se agrega una opción de prueba a producción.
- Se preservan `0700` para directorio, `0600` para archivo, `create_new(true)` y el límite de 64 KiB.
- No se añadió `sync_all`: la garantía requerida es que el launcher lea el archivo después de `write_all` y cierre normal, en el mismo flujo; sincronizar a almacenamiento durable no es necesario para esa lectura y añade I/O/coste. Tampoco produce una comprobación de error de `File::drop`.
- Un fallo de `close` no se fuerza de forma determinista con la API safe de `std::fs::File`: `File` no ofrece `close()` que devuelva `io::Result`; `drop` no reporta errores. `sync_all` reportaría fallos de sincronización, no convierte el cierre en una operación comprobable ni demuestra un fallo de close. No se usó `unsafe` ni se afirma esa garantía.
- Los tests de cleanup se mantuvieron fuera del flujo de ejecución real: éxito y directorio no vacío prueban el método explícito, no la denegación arbitraria de `remove_file`/`remove_dir` en una salida del launcher. `Drop` sigue siendo best-effort para salidas abruptas; las tareas 2.1 y 2.2 quedan sin marcar porque close y limpieza arbitraria/flujo completo no están cubiertos.

### Validación

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: pasó; 17 tests en lib y 17 duplicados en `approval_resource`.
- `cargo test -p huginn-runtime`: pasó sin filtro; 48 unitarios, 4 connections passed/1 ignored, todas las suites de integración restantes y doc-tests, sin fallos.
- `cargo fmt --all -- --check`: pasó.
- `git diff --check`: pasó.
- No se avanzó a 2.3–2.4 ni fase 3; no se cambió SBPL, grants, política o fallback.

## Cierre apply revisado: tareas 2.1–2.2 (2026-11-09)

### TDD RED → GREEN → REFACTOR

- Se añadieron tests conductuales privados para la composición de error pre-spawn (fallo primario más cleanup) y de error post-spawn (timeout primario más cleanup), además del caso de resultado de ejecución exitoso con cleanup fallido. Comprueban que el mensaje pre-spawn dice que el launcher no se inició y que los errores post-spawn indican que el shell pudo ejecutarse.
- RED observado: `cargo test -p huginn-runtime pre_spawn_error_composition_preserves_primary_and_cleanup_failures -- --nocapture` falló con solo el fallo primario en el mensaje, sin el error secundario de cleanup. La falla fue de aserción conductual, no de compilación.
- GREEN: se centralizó la composición en helpers privados compartidos por producción y tests. Open y write fallidos limpian explícitamente recursos ya creados e incluyen cualquier error de cleanup junto al primario. Un error de creación del directorio indica fallo antes del spawn; no hay recursos creados que limpiar. `run_shell_with_seatbelt_profile` compone toda salida controlada del launcher con el resultado de `SeatbeltProfileFile::cleanup`, preservando errores de spawn, timeout/cancelación y resultado/error de ejecución.
- Los tests de mkdir no-directory, writer fallido, tamaño excedido, rechazo y spawn ausente/no ejecutable conservan los marcadores/errores esperados. El test de cleanup con directorio no vacío prueba `SeatbeltProfileFile::cleanup` directamente; demuestra propagación de error `remove_dir` ante contenido sobrante, no una denegación OS dentro del flujo completo del launcher. No se simularon errores de close de `std::fs::File`.
- `Drop` sigue haciendo unlink best-effort y no promete eliminación ante fallo del OS. No se introdujeron `unsafe`, overrides públicos/de entorno, dependencias nuevas, shell/profile override, permisos adicionales, fallback, retry ni cambios de status 65. Se conserva el perfil fijo, el límite, `create_new`, modos `0700`/`0600` y `/usr/bin/sandbox-exec -f PROFILE /bin/sh -c COMMAND`.

### Validaciones

- RED: `cargo test -p huginn-runtime pre_spawn_error_composition_preserves_primary_and_cleanup_failures -- --nocapture` — falló como esperado (0 passed, 1 failed; omite 59 tests filtrados).
- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` — pasó en macOS: 29/29 en `lib` y 29/29 en `approval_resource`; el primer intento anterior a las correcciones menores falló 2 asserts de texto (case/afirmación `may have run`), no se cuenta como resultado final.
- `cargo test -p huginn-runtime` sin filtro — pasó: 60 tests `lib`, 3 adapter security, 44 approval resource, 34 coding workflow, 4 connections pasadas/1 ignored, 8 guardrails, 1 host secrets, 31 inference adapters, 2 MCP lifecycle, 9 providers integration, 6 session locks, 5 session persistence, 10 skill loading, 9 skills catalog, 4 stream persistence, 4 tool authoring, 12 tool lifecycle, 6 transports y doc-tests; cero fallos.
- `cargo fmt --all -- --check` — pasó.
- `cargo clippy -p huginn-runtime --all-targets -- -D warnings` — pasó.
- `git diff --check` — pasó.

### Estado y límites

- Tareas 2.1–2.2 permanecen sin marcar: aunque los asserts pasan, el cambio de comportamiento se implementó antes de añadir las pruebas nuevas. El RED se observó al volver a introducir temporalmente la pérdida del error secundario, pero esto no corrige el incumplimiento del requisito de escribir el test primero.
- Las fases SDD completadas registradas siguen siendo `init, explore, propose, spec, design, tasks`; fase actual `apply`, `next: apply`, `apply_status: partial`. No se declara aceptación ni se recomienda `sdd-verify` hasta resolver el orden TDD.
- El task RPI local e ignorado refleja el estado parcial. No se ejecutó verify, QA ni archive.

## Caracterización de launcher: tareas 2.3–2.4 (2026-11-09)

- Revisé la spec de ciclo de vida y el flujo descrito en el diseño, así como `run_shell`, `run_shell_with_seatbelt_profile`, `run_shell_with_launcher` y los tests macOS en `crates/huginn/runtime/src/tools.rs`.
- Baseline previo a editar tests: `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` pasó; 17 tests en lib y 17 duplicados en `approval_resource`. Ya se caracterizaban cwd, PATH, stdin EOF, streams separados, dos streams de 100 KB con límite y drenaje, y errores de spawn para launcher ausente/no ejecutable sin marcador.
- Se amplió el test de parámetros de ejecución para comprobar `$0=/bin/sh` y que `HOME` no se hereda cuando existe en el entorno host. Tras el cambio, la suite focalizada pasó con los mismos conteos. Este test es caracterización del comportamiento previo; no se reclama RED/GREEN de producto.
- Mapeo de evidencia: cwd canónico, PATH elegido, stdin nulo/EOF, stdout y stderr separados, límite/drenaje y spawn errors están cubiertos por pruebas macOS. La implementación forma argv del launcher en elementos separados y establece `/bin/sh -c` con el comando original como un solo argumento; `env_clear()` restaura solo `PATH`. La prueba `$0` confirma el shell observado, no prueba por sí sola ausencia de toda posible capa intermediaria; la afirmación de argv/no shell adicional se limita a inspección directa del callsite y helper.
- Se marcaron 2.3–2.4 completas solo dentro de esa evidencia. 2.1–2.2 permanecen unchecked y parciales: cierre safe de `File` no es observable y la limpieza arbitraria no se fuerza desde el flujo del launcher. No se tocó comportamiento productivo ni fases posteriores.

### Validación de tareas 2.3–2.4

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: baseline y post-cambio pasaron; 17 tests en lib y 17 en `approval_resource`.
- `cargo test -p huginn-runtime`: pasó sin filtro; 48 tests unitarios, 4 connections passed/1 ignored, todas las suites de integración restantes y doc-tests, sin fallos.
- `cargo fmt --all -- --check`: pasó.
- `git diff --check`: pasó.

## Cierre acotado de tarea 3.2 (2026-11-09)

- Revisé los tres tests de socket en `crates/huginn/runtime/src/tools.rs`. Eliminé el test diagnóstico auxiliar `nc -h` que volcada toda la ayuda, junto con los `eprintln!` de alto volumen de los casos TCP/UDP/Unix. Las verificaciones ahora reportan output y resultado del listener en el mensaje de aserción ante fallo. UDP exige que el listener sandboxed no reciba ningún datagrama.
- Ajusté la espera compartida de resultado de listener a 5 s, por encima de los timeouts acotados de los listeners; los hilos tienen deadlines finitos y se unen en el flujo normal. El socket Unix está bajo `TempDir` dentro del workspace canónico; la ruta se elimina entre controles host y sandbox y el directorio temporal cubre cleanup ante fallo.
- Evidencia informada fresca por el orquestador en macOS 27.3: TCP host conecta y es aceptado; el probe `nc` descendiente sandboxed termina status 1 con `connectx ... (tcp) failed: Operation not permitted` y listener sin aceptación. UDP host recibe cuatro datagramas `[X]` y el payload exacto `[udp-probe]`; sandbox `nc` termina status 1 con `connectx ... (udp) failed: Operation not permitted` y no llega datagrama. Unix: host acepta; shell sandbox ve el socket (`socket-visible`), `nc -v -U ... -w1` sale con status normal 1 y listener no acepta. El `nc` de Apple no muestra diagnóstico Unix de connect; solo el error no fatal `/private/var/select/sh`, que también aparece en `nc -h` con ayuda completa y marcador posterior. No se inventa diagnóstico: la evidencia Unix es la combinación del control host exacto, visibilidad del socket, retorno normal no-cero y no aceptación.
- Los tres probes se ejecutan como procesos hijos del shell confinado. Esto limita la afirmación a esos probes y sus operaciones de socket; no valida otros descendientes, acceso externo de archivos ni más protocolos/versiones. Se marcó solo 3.2 `[x]`; 3.1, 3.3–3.6 y Phase 2 2.1–2.2 continúan pendientes/incompletas. No se modificó producción, SBPL, CI, docs ni grants.

### Commands Run

- `cargo test -p huginn-runtime run_shell_seatbelt_denies -- --nocapture`: exit 0; 3/3 en lib y 3/3 en `approval_resource`.
- `cargo test -p huginn-runtime`: exit 0, sin filtro. Resultados: lib 51; `adapter_security` 3; `approval_resource` 35; `coding_workflow` 34; `connections` 4 passed/1 ignored; `guardrails` 8; `host_secrets` 1; `inference_adapters` 31; `mcp_lifecycle` 2; `providers_integration` 9; `session_locks` 6; `sessions_persistence` 5; `skill_loading` 10; `skills_catalog` 9; `stream_persistence` 4; `tool_authoring` 4; `tool_lifecycle` 12; `transports` 6; los dos targets wasm y doc-tests sin tests. Sin fallos.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.

## Cierre acotado de tarea 3.1 (2026-11-09)

- Scope revisado antes de editar: `crates/huginn/AGENTS.md`, spec delta, diseño, tareas y estado. El árbol tenía cambios previos amplios; se preservaron y no se alteraron SBPL, allow rules, código productivo, CI, docs ni grants.
- TDD: la tarea verificó enforcement ya implementado, por lo que los escenarios añadidos son characterization, no se inventó un cambio de producto. Baseline previo: `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` pasó 20 tests en lib y 20 en `approval_resource`; el test externo y el positive workspace read/write ya existían.
- Se añadieron seis pruebas reales de `run_shell`/`/usr/bin/sandbox-exec`: otro workspace; fixture aislado tipo home/credenciales sin acceder a secretos reales; ruta externa; directorio temporal designado solo como fixture y fuera del workspace (sin tocar `/tmp` global); intento de `cwd` al exterior; symlink desde workspace al archivo exterior. Para los cinco casos de lectura externa, el host verificó que el fixture era legible antes del sandbox; el shell emitió sentinel, `/bin/cat` produjo `Operation not permitted` para el archivo existente, el shell devolvió status normal no-cero y el secreto no apareció. En el caso symlink se verificó que `canonicalize(link)` era el destino exterior. El test de `cwd` exige una denegación del SO al cambiar de directorio o, si se permite el cambio, denegación de lectura relativa y ausencia del secreto. El test existente positivo verifica lectura/escritura dentro del workspace canónico.
- No se observó fallo de profile en estos escenarios; no hay RED/GREEN de producto. En la primera ejecución combinada de toda la suite focalizada, falló una vez el control host UDP preexistente por no recibir el datagrama. Repetido aislado pasó; el focused adversarial final pasó 7/7 en lib y 7/7 en `approval_resource`; workspace positivo pasó 1/1 en ambos; la suite completa posterior pasó.
- Plataforma observada: macOS 27.3, build 26C5035r, con `/usr/bin/sandbox-exec`. Esta evidencia no establece soporte para otras versiones ni prueba otros paths/operaciones, robustez general ante TOCTOU o compatibilidad futura de Seatbelt.

### Commands Run

- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` (baseline): exit 0; 20 lib + 20 `approval_resource` pasaron.
- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` (post-tests, primera ejecución): exit 101; los seis denies y los casos de ruta pasaron, pero el control host UDP existente no entregó su datagrama. Este resultado se registró, no se ocultó.
- `cargo test -p huginn-runtime run_shell_seatbelt_denies_udp_datagram_after_host_control_succeeds -- --nocapture`: exit 0; 1 lib + 1 `approval_resource` pasaron en repetición aislada.
- `cargo test -p huginn-runtime run_shell_cannot_read -- --nocapture`: exit 0; 7 denies en lib + 7 en `approval_resource` pasaron.
- `cargo test -p huginn-runtime run_shell_can_read_and_write_inside_workspace -- --nocapture`: exit 0; 1 positivo en lib + 1 en `approval_resource` pasaron.
- `cargo test -p huginn-runtime` (sin filtro): exit 0; lib 57; adapter_security 3; approval_resource 41; coding_workflow 34; connections 4 passed/1 ignored; guardrails 8; host_secrets 1; inference_adapters 31; mcp_lifecycle 2; providers_integration 9; session_locks 6; sessions_persistence 5; skill_loading 10; skills_catalog 9; stream_persistence 4; tool_authoring 4; tool_lifecycle 12; transports 6; wasm targets/doc-tests 0. Sin fallos.
- `cargo fmt --all -- --check`: exit 0 tras ejecutar `cargo fmt --all` para normalizar el formato nuevo.
- `git diff --check`: exit 0.

Estado: 3.1 marcada `[x]` únicamente para estos fixtures/casos/macOS observado. No implica completar 3.3–3.6 ni el cambio; otras tareas incompletas conservan estado parcial.

### Revisión final del caso de `cwd`

- La revisión detectó que la versión inicial ocultaba el error de `cd` y por eso no demostraba que el shell hubiera cambiado de directorio. Se ajustó el test para observar el resultado: exige un diagnóstico `Operation not permitted` si el cambio se deniega; si el cambio se permite, exige que la lectura relativa del archivo externo reciba la denegación y no revele el secreto.
- `cargo test -p huginn-runtime run_shell_cannot_read_external_marker_after_changing_cwd -- --nocapture`: pasó en lib y `approval_resource` (1 caso por target). `cargo fmt --all -- --check` y `git diff --check`: pasaron. `cargo test -p huginn-runtime` completo pasó antes de esta última precisión del assertion; la suite adversarial completa también pasó antes de esa precisión.

## Continuación final de apply: CI, documentación y límites restantes (2026-11-09)

### Completado en esta continuación

- Se marcaron 1.14 y 3.3–3.6, y 4.1–4.4 solo con el alcance descrito en `tasks.md`. Los tests de perfil muestran deny-default, `dyld-support.sb` antes de deny-default, fork/exec explícitos, lectura runtime acotada a `/bin` y `/usr/lib`, read/write de workspace, ausencia de allow de red/sockets y ausencia de raíces amplias. Las pruebas reales de ejecución y enforcement permanecen macOS-only.
- El inventario de soporte en manifests/CI no declara una matriz de versiones macOS. Runtime separa dependencias por target macOS/Linux; CI existente usa `macos-latest` en jobs multios solo para push y excluye Huginn en Windows. El job nuevo `Test Huginn Seatbelt (macOS)` ejecuta también en PR; no se interpreta `macos-latest` como soporte para versiones fijas adicionales.
- Se agregó el job dedicado a `.github/workflows/ci.yml`, con comprobación explícita de `RUNNER_OS` y existencia ejecutable de `/usr/bin/sandbox-exec`, y ejecución sin filtro de `cargo test -p huginn-runtime`. El job conserva output y requiere que las pruebas de workspace read/write, path externo y TCP/UDP/Unix aparezcan como `ok`; un test ausente o ignorado no satisface esos checks. Se añadió al job `CI / Required` y al evaluador de resultados. El workflow test verifica runner, launcher, invocación no filtrada, casos obligatorios y gate required; `node --test .github/scripts/ci-*.test.cjs` pasó 309/309.
- Se actualizó README, arquitectura, config de ejemplo y ADR-0018. Se eliminó el texto obsoleto “not sandboxed” para shell, sin ampliar la afirmación a herramientas nativas/MCP ni a Linux/Windows. No se añadió configuración u opt-out.
- Para 4.1 se usaron tests preexistentes de caracterización: `rejected_effect_is_recorded_denied_and_not_executed` y `allow_once_authorizes_only_one_prepared_operation_and_keeps_profile_unchanged`. No se afirma TDD RED/GREEN en esta tarea ni que esos tests de aplicación constituyan evidencia del kernel Seatbelt. La integración real de `run_shell` confirma por separado el sandbox obligatorio.
- Clippy detectó inicialmente warnings en elementos crate-public dentro del módulo privado, un borrow redundante y una aserción duplicada entre ramas. Se ajustó visibilidad a `pub` dentro del módulo privado, se eliminó el borrow y se consolidó la aserción; no se añadió `allow` ni se alteró el perfil.

### Bloqueo de 2.1–2.2

- La cobertura disponible incluye perfil inválido observado por `run_shell` como status 65 sin marcador, launcher ausente/no ejecutable, perfil oversized, fallo de creación vía raíz no-directorio, propagación de error de writer y cleanup explícito normal/error. El `status 65` se conserva como resultado observable porque puede provenir tanto del launcher como de un shell válido; no se infiere su causa solo por el código.
- La API safe de `std::fs::File` no ofrece `close() -> io::Result`; `Drop` no informa errores de cierre. No se agregó `unsafe`, un override configurable ni seam público, y no se afirma error de close observado. Tampoco se forzó una falla arbitraria de unlink en el flujo completo del launcher: `Drop` queda best-effort en terminación abrupta, aunque existe cleanup explícito para resultados controlados. Por estos criterios pendientes, 2.1 y 2.2 siguen sin marcar. Se necesita una decisión/documentación del requisito de cierre verificable y del grado de garantía exigido para cleanup ante error de OS, o una API/plataforma distinta aprobada.

### Validación de esta continuación

- macOS 27.3 / build 26C5035r, `/usr/bin/sandbox-exec` presente: `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture` exit 0, 26/26 en lib y 26/26 duplicados en `approval_resource`; cubre los escenarios macOS existentes de lifecycle, rutas, sockets, rechazo de perfil y spawn.
- `cargo test -p huginn-runtime` sin filtro exit 0, ejecutado en macOS: lib 57/57; `adapter_security` 3; `approval_resource` 41; `coding_workflow` 34; `connections` 4 passed/1 ignored por requisito externo; `guardrails` 8; `host_secrets` 1; `inference_adapters` 31; `mcp_lifecycle` 2; `providers_integration` 9; `session_locks` 6; `sessions_persistence` 5; `skill_loading` 10; `skills_catalog` 9; `stream_persistence` 4; `tool_authoring` 4; `tool_lifecycle` 12; `transports` 6; wasm/doc tests 0. Sin fallos. Esta corrida fue posterior al último ajuste de código Rust.
- `cargo clippy -p huginn-runtime --all-targets -- -D warnings`: exit 0 después de corregir los tres hallazgos descritos.
- `just huginn-architecture`: exit 0; límites de arquitectura pasaron, incluido negative control.
- `cargo fmt --all -- --check`: exit 0. `git diff --check`: exit 0.
- `node --test .github/scripts/ci-*.test.cjs`: exit 0, 309/309. `pnpm exec markdownlint-cli2` para README, arquitectura, índice ADR y ADR-0018: exit 0, 0 issues.
- Tests focalizados de aprobación: `cargo test -p huginn-application allow_once_authorizes_only_one_prepared_operation_and_keeps_profile_unchanged` y `cargo test -p huginn-application rejected_effect_is_recorded_denied_and_not_executed` pasaron por separado (1 test cada uno).
- No se ejecutó `just ci-local`: no era requerido para validar los criterios focalizados del apply y agrega gates no relacionados con el cambio. No se ejecutó CI remoto; la garantía PR-blocking está implementada/testeada localmente, no observada en un run remoto. No se ejecutó `sdd-verify`.

Estado apply: parcial y bloqueado únicamente en 2.1–2.2 por evidencia safe de cierre y limpieza no disponible/forzada en el flujo completo. No se reclama aceptación ni se inicia verify.

## Reapertura de 2.1–2.2 bajo el contrato aprobado (2026-11-09)

- El usuario aprobó exigir cleanup explícito con propagación de errores en retornos controlados; en fallos pre-spawn, no iniciar el shell e informar error primario y cleanup; y dejar `Drop` abrupto como best-effort, sin garantía ante fallo del sistema operativo. No se exige un error independiente de `close()` porque `std::fs::File` safe API no lo expone.
- Para completar TDD, se reconstruyó temporalmente la conducta previa de composición después del primer cambio implementado. RED conductual: `pre_spawn_error_composition_preserves_primary_and_cleanup_failures` falló al perder el error secundario de cleanup; `post_spawn_error_composition_preserves_primary_and_possible_execution` falló al no conservar el timeout y el hecho de que el shell pudo correr. Se restauró la implementación y ambas pruebas pasaron. La prueba adicional `post_spawn_success_with_cleanup_error_does_not_claim_shell_was_skipped` también pasó. No se añadió `unsafe`, seam público, override ni dependencia.
- El cambio conserva cleanup explícito tras resultado controlado del launcher y propaga cleanup errors; fallos de creación/open/write antes del spawn intentan limpiar y componen error primario y cleanup. La composición post-spawn conserva el error de ejecución y no afirma que el shell no corrió. `Drop` continúa best-effort ante descarte abrupto.
- `cargo test -p huginn-runtime run_shell_seatbelt_tests -- --nocapture`: pasó, 29 tests en lib; el mismo módulo se compila/ejecuta también desde `approval_resource`.
- `cargo test -p huginn-runtime`: exit 0; 60 pruebas de lib, 44 de `approval_resource`, suites de integración pasaron y 1 prueba se ignoró por requisito externo.
- `cargo clippy -p huginn-runtime --all-targets -- -D warnings`, `cargo fmt --all -- --check` y `git diff --check`: exit 0.
- Los checks son locales; no se ejecutó CI remoto. Enforcement real sigue limitado a macOS 27.3/build 26C5035r. No se prueba ni promete eliminación ante fallo OS durante `Drop` abrupto, ni error de `close()` no observable.

Estado: 2.1 y 2.2 marcadas completas de acuerdo con los criterios revisados y aprobados. Apply está completo; repetir `sdd-verify` antes de cualquier QA/archive.
