# ADR-0014: Providers integrados y enrutamiento del modelo por turno

- Estado: Accepted
- Fecha: 2026-10-13
- Producto: Huginn
- Base de decisión: aprobación explícita del usuario para formalizar la dirección
  Providers y preparar cinco entregas P0 y una P1 OAuth. Accepted acepta la
  dirección; no declara implementación, verificación ni aceptación del producto.

## Contexto

Huginn necesita gestionar varias conexiones y elegir modelos sin acoplar el
agente a una terminal ni mezclar credenciales. Hoy `ModelProvider` y
`ModelService` delimitan la inferencia; la composición publica `agent:model@1`.
El adaptador `OpenAiProvider` implementa streaming compatible con OpenAI y
desactiva redirecciones. La configuración actual construye un proveedor desde
CLI, TOML y variables de entorno; no constituye un gestor de conexiones.

[ADR-0013](0013-authorable-plugin-contract-and-loading.md) establece composición
nativa estática, contribuciones declarativas y registros por generación.
Los callbacks UI actuales son síncronos y no bloqueantes: las acciones asíncronas
y los formularios secretos de esta decisión son trabajo futuro, no APIs actuales.
El bucle puede inferir varias veces, ejecutar herramientas y compactar contexto
dentro de un turno; resolver la selección en cada llamada produciría cambios
silenciosos de modelo o de identidad durante una misma operación.

## Decisión

### Catálogo, gestión y router

Providers será una capacidad integrada de Huginn, compuesta estáticamente según
ADR-0013, con responsabilidades separadas: catálogo de proveedores/modelos,
gestión de conexiones y selección, y router de inferencia. El router será el
único publicador de `agent:model@1`; los adaptadores se registrarán en el
catálogo y no competirán por ese servicio. Las identidades y tipos concretos de
los nuevos contratos se definirán durante P0.1, sin declarar aquí APIs existentes.

Catálogo, acciones, registros y tareas estarán asociados a la generación de
activación. La publicación será transaccional: una activación fallida revierte
todos sus efectos; el desmontaje revoca sus registros y cancela sus tareas.
Un identificador de una generación antigua no puede ejecutar su reemplazo.
El rollback de activación no borra conexiones duraderas preexistentes; una
operación de gestión fallida tampoco puede dejar metadatos y secretos incoherentes.

`builtin.providers.core` proporcionará catálogo, conexiones y routing en modo
headless. `builtin.providers.ui` dependerá del servicio Providers y de las
contribuciones UI; registrará `/login`, `/providers`, `/model` y `/logout`.
Los adaptadores serán plugins internos independientes registrados en el catálogo.
La contribución UI será opcional: el host posee TTY, foco, teclado, renderizado,
formularios y tratamiento de campos secretos. Las acciones serán declarativas
y asíncronas, con estados de espera, éxito, error y cancelación; no se hará I/O
en callbacks síncronos ni se exigirá importar internals de Ratatui al proveedor.
Los secretos no se introducirán como argumentos de comandos, historial o texto
de contribuciones. Esta frontera no convierte extensiones nativas confiables
en código aislado ni en un sandbox.

### Selección por turno y revocación

Al comenzar un turno se fija la identidad de conexión, modelo y generación
autorizada. Todas sus inferencias, incluidas continuación tras herramientas y
compactación, usan esa fijación. Cambiar la selección afecta al siguiente turno;
no hay fallback automático a otro modelo, conexión o proveedor ante errores.

La autorización se comprueba antes de cada inferencia, no solo al fijar el
turno. Revocar, eliminar o reemplazar las credenciales de la conexión, o revocar
su generación, invalida las inferencias posteriores incluso dentro del mismo
turno; no se reutiliza una credencial retenida ni se adopta su reemplazo.
La cancelación de una petición en curso es best effort: puede detener consumo
local, pero no prueba que el servidor haya cancelado, dejado de facturar o
revocado remotamente la credencial. El estado observable debe distinguirlo.
En P1, un refresh autorizado de tokens dentro de la misma autorización no será
un cambio de conexión/modelo; deberá distinguirse de rotación o revocación y
jamás restaurar una autorización invalidada.

### Conexiones, secretos y precedencia

Los metadatos persistentes de conexión y selección se guardarán separados de
los secretos. Cada conexión tiene identidad propia y referencia a sus
credenciales; ni sesiones, TOML, logs, errores, eventos JSON ni contribuciones
UI deben recibir valores secretos. No se presupone que el historial general
carezca de información sensible aportada por el usuario.

En macOS se usará Keychain. En Linux se usará un backend seguro cuando esté
disponible; si falta o está bloqueado, se informa el estado y solo se ofrece
modo efímero explícito, nunca persistencia silenciosa en texto plano. El modo
efímero no promete recuperación de credenciales tras reiniciar. Persistencia,
fallos parciales, eliminación y reconciliación necesitan pruebas de rollback.

La precedencia será: CLI explícita > configuración explícita o entorno elegido
> selección persistida > `unconfigured`. Dentro del segundo nivel, la
configuración explícita conserva prioridad sobre valores por defecto de entorno;
el entorno utilizado debe quedar determinado y asociado a la conexión elegida.
La resolución valida una conexión coherente: no combina endpoint/modelo de una
con credenciales de otra ni completa una elección explícita inválida recurriendo
a la selección persistida. Sin selección se permite diagnóstico sin inferencia;
un intento de inferencia falla con un estado accionable y sin secretos.
La migración de la configuración actual preservará este aislamiento y explicará
su procedencia sin presentar un nuevo esquema como ya implementado.

### Transporte y autenticación

Los endpoints personalizados requieren TLS validado, excepto loopback local
autorizado explícitamente para transporte sin TLS. No se amplía la excepción
a redes privadas ni se desactiva validación de certificados. Las redirecciones
se rechazan por defecto; cualquier política futura debe impedir fugas de
autorización a otro origen y degradación a HTTP. La configuración existente que
acepta HTTP general deberá adecuarse y documentarse en P0.4.

P0 incluye un adaptador OpenAI-compatible y un adaptador local Ollama cuya
compatibilidad se verificará, no se inferirá del nombre. OAuth oficial queda
para P1, evaluado por proveedor y solo mediante flujos documentados y permitidos.
Una cuenta o suscripción ChatGPT no equivale a credenciales ni acceso a la API
OpenAI. No se importan cookies, tokens de sesión ni flujos privados de productos
de consumo. Rook, carga dinámica, hot reload y runtimes externos quedan fuera.

## Alternativas consideradas

- Un publicador de `agent:model@1` por proveedor: reduce intermediación, pero
  crea colisiones de composición y acopla el bucle a la gestión de conexiones.
  Se elige un router único con catálogo separado.
- Resolver selección por inferencia y aplicar fallback: mejora disponibilidad,
  pero cambia identidad, coste y comportamiento durante un turno. Se elige
  fijación por turno con fallo explícito y revocación comprobada en cada llamada.
- Retener indefinidamente la credencial fijada: facilita continuidad, pero
  ignora revocaciones. Se conserva identidad, no autorización irrevocable.
- Gestión implementada solo en TUI: entrega visual más rápida, pero impide
  automatización y pruebas sin terminal. Se eligen casos de uso headless y UI
  opcional con acciones declarativas.
- Secretos en TOML/SQLite o fallback automático en Linux: simplifica despliegue,
  pero aumenta exposición y oculta pérdida de garantías. Se eligen almacenes
  seguros y modo efímero explícito, con coste de integración por plataforma.
- HTTP personalizado general: facilita servidores remotos de desarrollo,
  pero expone credenciales y contenido. Se restringe la excepción a loopback
  explícito, a costa de migrar configuraciones inseguras.
- OAuth genérico o reutilizar autenticación ChatGPT: aparenta onboarding
  uniforme, pero no demuestra permisos ni acceso oficial a inferencia.
  Se difiere OAuth oficial por proveedor a P1.
- Integración Rook o loader dinámico ahora: amplía reutilización y extensibilidad,
  pero añade contratos y confianza ajenos al objetivo. Se mantiene composición
  integrada y estática.

## Consecuencias

- La selección es predecible y se puede probar sin red ni terminal; la UI no
  impide avanzar al núcleo ni a los adaptadores.
- Se introducen gestión de generaciones, carreras de revocación, transacciones
  entre almacenes y estados de credenciales bloqueadas o ausentes.
- Un error o revocación puede detener el turno; no se prioriza disponibilidad
  sobre identidad. Cancelación local no implica cancelación remota.
- La seguridad de Linux depende del backend disponible. El modo efímero y los
  fallos de recuperación deben ser visibles antes de inferir.
- El alcance de pruebas automatizadas no equivale a aceptación de onboarding
  ni a validación de todas las combinaciones de proveedores reales.

## Seguimiento y evidencia

Base inspeccionada: [`application/src/composition.rs`](../../../crates/huginn/application/src/composition.rs),
[`runtime/src/model.rs`](../../../crates/huginn/runtime/src/model.rs),
[`core/src/kernel.rs`](../../../crates/huginn/core/src/kernel.rs),
[`plugin-api/src/v1.rs`](../../../crates/huginn/plugin-api/src/v1.rs),
[`config.rs`](../../../apps/huginn/src/config.rs) y
[`config.example.toml`](../../../apps/huginn/config.example.toml).

Entregas previstas, no implementadas por este registro:

1. P0.1: contratos, catálogo y router; demostración headless con dobles y
   pruebas de fijación por turno, rollback y revocación.
2. P0.2: conexiones, secretos y precedencia headless sobre P0.1; pruebas de
   aislamiento, fallos parciales, reinicio y ausencia de fugas.
3. P0.3: UI independiente sobre P0.1/P0.2; dobles hasta disponer de P0.4,
   formularios secretos, acciones cancelables y generaciones obsoletas.
4. P0.4: adaptadores OpenAI-compatible y Ollama local sobre P0.1/P0.2;
   streaming, herramientas, transporte seguro y compatibilidad comprobada.
5. P0.5: integración automatizada de P0.1–P0.4, desde selección hasta turnos
   y recuperación, con rollback, revocación, reinicio y secret leakage.
6. P1: OAuth oficial por proveedor sobre el flujo P0 integrado; renovación,
    expiración, revocación y ausencia de credenciales de productos de consumo.

Seguimiento de ingeniería creado en GitHub:
[P0.1 / #350](https://github.com/dallay/cortex/issues/350),
[P0.2 / #351](https://github.com/dallay/cortex/issues/351),
[P0.3 / #352](https://github.com/dallay/cortex/issues/352),
[P0.4 / #353](https://github.com/dallay/cortex/issues/353),
[P0.5 / #354](https://github.com/dallay/cortex/issues/354) y
[P1 / #355](https://github.com/dallay/cortex/issues/355).
Abrir las issues no acredita la implementación.

Los tipos concretos, formato de metadatos, backend Linux, política de
reconciliación y matriz Ollama se concretarán con evidencia en esas entregas.
No son decisiones de implementación cerradas por este ADR.

Las QA [#276](https://github.com/dallay/cortex/issues/276),
[#332](https://github.com/dallay/cortex/issues/332),
[#307](https://github.com/dallay/cortex/issues/307) y
[#280](https://github.com/dallay/cortex/issues/280) siguen independientes:
este ADR y sus entregas no las cierran ni las convierten en bloqueos para
desarrollar Providers. La aceptación de onboarding es un gate separado;
las pruebas de entrega aportan evidencia, no sustituyen ese gate.
