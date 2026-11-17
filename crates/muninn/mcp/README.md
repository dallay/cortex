# Base MCP de Muninn por stdio

Este crate implementa una base de protocolo, no un servidor de memoria completo.
El binario independiente no conecta Neo4j ni registra handlers de aplicación.
`tools/list` devuelve una lista vacía. No demuestra paridad MCP, Bronze ni cierre H1.

## Ejecución

```sh
cargo run -p muninn-mcp --bin muninn-mcp --locked
```

El proceso lee una petición JSON por línea desde stdin. Solo escribe mensajes
JSON-RPC en stdout. Al cerrar stdin, termina. Rechaza mensajes de más de 1 MiB;
la longitud excluye el salto de línea final. Un mensaje sobredimensionado termina
la sesión con error, sin copiar su contenido al diagnóstico.

## Versión y descubrimiento

La versión implementada es MCP `2026-07-28`. Esta versión usa metadatos por
petición y `server/discover`, no el handshake de versiones anteriores.

Ejemplo de una línea de entrada:

```json
{"jsonrpc":"2.0","id":1,"method":"server/discover","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}
```

La versión y capacidades del cliente son obligatorias. `clientInfo` es opcional,
pero debe tener nombre y versión cuando se incluye. El servidor valida la versión
en cada petición y devuelve `-32022` cuando no la soporta.

Se implementan `server/discover`, `ping`, `tools/list` y `tools/call`. Las respuestas
exitosas incluyen `resultType: complete` e identidad propia en `_meta`. No se copian
las capacidades ni las instrucciones de la referencia que prometen funciones ausentes.

## Catálogo y handlers

`registry/` conserva las capturas de referencia como datos de contrato, incluida
la captura histórica `initialize.json`. Conservar esa captura no implica implementar
su handshake ni copiar la identidad `neo4j-agent-memory`.

`Catalog` devuelve las definiciones exactas de herramientas de cada fixture.
`Server::register` vincula explícitamente una definición con un `ToolHandler`.
Solo las herramientas registradas aparecen en `tools/list`. El registro se congela
con la primera petición para mantener `listChanged: false`.

El adaptador de aplicación debe validar los argumentos según el esquema y aplicar
autorización antes de invocar puertos. La base solo exige un objeto de argumentos;
no implementa validación completa de JSON Schema. Los errores del handler se convierten
en resultados `isError: true` con un mensaje genérico, sin exponer el error interno.

Los handlers actuales son síncronos. La integración futura con puertos asíncronos debe
resolver ejecución, cancelación y límites sin bloquear un runtime asíncrono. Esta base
procesa peticiones secuencialmente; no implementa concurrencia ni cancelación.
No admite versiones MCP anteriores ni transporte HTTP.

## Verificación focalizada

```sh
cargo test -p muninn-mcp --locked
cargo clippy -p muninn-mcp --all-targets --locked -- -D warnings
cargo fmt -p muninn-mcp -- --check
```

Las pruebas no usan servicios externos. Incluyen contrato de catálogo, descubrimiento,
metadatos por petición, herramientas disponibles, errores saneados, framing, límite
de entrada y una prueba real del proceso con stdin/stdout.

## Siguiente integración

Después de integrar long-term y reasoning, conectar handlers al contrato de aplicación,
añadir validación de argumentos y autorización por herramienta, y comparar los resultados
con la referencia fijada. No reutilizar el resultado de estas pruebas como aceptación
de operaciones de memoria que todavía no se ejecutan.
