# Issue #133: respuesta nativa de Ollama — plan de implementación

> Lista táctica aprobada para ejecutar inline, tramo por tramo. Cada cambio de comportamiento sigue RED → GREEN → REFACTOR; no se crearán commits salvo petición expresa.

## Meta y arquitectura

Preservar en Rook los datos estructurados que Ollama devuelve: motivo de cierre, razonamiento separado, llamadas a herramientas y duraciones. El modelo de dominio mantendrá semántica neutral al proveedor; los adaptadores OpenAI-compatible traducirán solo datos estructurados y generarán IDs válidos en el borde cuando falten. Las duraciones irán a `tracing`, sin persistencia SQLite. JSON dentro de `message.content` seguirá siendo texto.

## Rutas a modificar

- `crates/domain/rook-core/src/model.rs`: `Message`, `CompletionResponse`, `StreamChunk`, tipos neutrales para llamadas completas y `FinishReason` existente. `CompletionResponse` incluye `thinking`, `tool_calls` y `finish_reason` opcionales; `Message.tool_calls` es una colección que omite vacíos al serializar.
- `crates/infrastructure/providers-ollama/src/lib.rs`: DTO de respuesta, mapeo no-stream/NDJSON y eventos de `tracing` de duración.
- `crates/infrastructure/providers-ollama/tests/provider.rs`: casos WireMock para compatibilidad, done_reason, thinking, tool calls, duraciones y NDJSON en fragmentos.
- Adaptadores que construyen o consumen los tipos de dominio: OpenAI/Anthropic/Gemini/Groq y `transport-axum` (localizar inicializadores con `rg 'CompletionResponse \{|StreamChunk \{|Message \{' crates apps`). Ajustar campos opcionales/colecciones vacías sin inventar finish reasons o thinking.
- `crates/infrastructure/transport-axum/src/openai_adapter.rs` y `anthropic_adapter.rs`: conservar llamadas entrantes estructuradas en `Message.tool_calls`; el formato OpenAI no pierde llamadas adicionales. El wire OpenAI saliente conserva thinking/reason y finish reason tipados.
- `crates/infrastructure/transport-axum/tests/openai_tool_stream_round_trip.rs`: probar Ollama→dominio→wire mediante `OllamaProvider`; JSON textual no se vuelve llamada.
- Serializadores de solicitudes de providers OpenAI/Ollama: reenviar llamadas de `Message.tool_calls` como campos estructurados usando el contrato que cada API admite; proveedores que no exponen estos datos continúan usando lista vacía. Los IDs OpenAI sintéticos incluyen request, mensaje e índice para ser estables y únicos.

## Secuencia de tareas

### 1. Contrato tipado y compatible

- [x] RED: agregar pruebas de serialización/mapeo que exijan `CompletionResponse.finish_reason: Option<FinishReason>`, `StreamChunk.thinking: Option<String>` y `Message.tool_calls: Vec<...>` con valores ausentes válidos; verificar que `FinishReason` mapea razones conocidas y deja razones desconocidas como `None`.
- [x] Ejecutar las pruebas de contrato `cargo test -p transport-axum --lib assistant_message_preserves_every_structured_tool_call` y `cargo test -p transport-axum --lib anthropic_tool_use_block_converts_to_neutral_domain_call`.
- [x] GREEN: definir un tipo neutral de llamada completa con `id: Option<String>`, `name` y `arguments: serde_json::Value`; serializar argumentos solo al cruzar al formato de destino y nunca inspeccionar `message.content` para inferir llamadas.
- [x] Actualizar todos los inicializadores afectados con `None`/`Vec::new()`; los decoders OpenAI/Anthropic preservan tool calls estructurados en el tipo neutral, sin limitarse al primer elemento, y los serializadores de solicitudes OpenAI/Ollama reenvían el modelo neutral en sus formatos estructurados.
- [x] Ejecutar `cargo test -p providers-ollama` y `cargo check --workspace` para probar compatibilidad de constructores y valores neutrales.

### 2. Respuesta completa Ollama

- [x] RED: añadir WireMock para `done_reason: "length"`, `"stop"`, `"tool_calls"`, razón desconocida, thinking, tool_calls estructurados y JSON textual en `message.content`.
- [x] Ejecutar `cargo test -p providers-ollama <filtro>` y confirmar que los casos fallan por datos no preservados.
- [x] GREEN: ampliar DTOs con campos opcionales/default para `created_at`, `thinking`, `tool_calls`, `images`, `logprobs`, `prompt_eval_cached_count`, contadores de tokens y duraciones del esquema ChatResponse; mapear finish reason solo en valores reconocidos, llamadas solo desde `message.tool_calls`, y conservar content literal.
- [x] Emitir evento `tracing` de éxito con solo duraciones disponibles (unidades nombradas como `_ns`, según Ollama); no incluir prompts, contenido, thinking, llamadas/argumentos ni credenciales.
- [x] Ejecutar los tests del provider y verificar que fixtures antiguos sin campos opcionales siguen pasando.

### 3. Streaming NDJSON Ollama

- [x] RED: agregar respuesta NDJSON con línea final sin `done_reason`, eventos de thinking/tool_calls y contenido dividido entre fragmentos de bytes/line-buffer; incluir terminación `length` y razón desconocida.
- [x] Ejecutar tests focalizados para observar fallos de thinking, tool calls, finish reason y datos de usage.
- [x] GREEN: mapear deltas estructurados a `StreamChunk`, mantener thinking fuera de `delta`, tolerar `done_reason` ausente en chunks intermedios/finales, conservar el parser line-buffer y emitir la última línea pendiente cuando EOF no trae salto de línea.
- [x] Emitir una sola métrica/evento de duración en la respuesta final cuando la respuesta contiene tiempos; omitir valores no entregados.
- [x] Ejecutar `cargo test -p providers-ollama` y confirmar tests Cloud Bearer existentes sin regresión.

### 4. Adaptador OpenAI-compatible e integración de extremo a extremo

- [x] RED: extender `openai_tool_stream_round_trip.rs` para alimentar una respuesta de Ollama y serializar sus `StreamChunk` a `choices[].delta.tool_calls`; afirmar índices, nombre, argumentos exactos y finish reason `tool_calls`/`length`.
- [x] RED: cubrir ID ausente y afirmar que el wire tiene ID no vacío y válido; comprobar que un JSON en `delta`/`message.content` permanece en `content` y no crea `tool_calls`.
- [x] Ejecutar `cargo test -p transport-axum --test openai_tool_stream_round_trip` y confirmar que falla por falta del mapeo.
- [x] GREEN: traducir calls neutrales a estructura OpenAI y generar un ID válido únicamente en el adaptador si la fuente no entregó uno; no tocar argumentos ni transformar contenido JSON textual en llamadas.
- [x] Mapear finish reason y exponer thinking como campo separado compatible con el contrato OpenAI existente del proyecto.
- [x] Ejecutar la prueba end-to-end y tests focalizados del adaptador.

### 5. Regresión integrada y revisión

- [x] Ejecutar `cargo test -p providers-ollama`.
- [x] Ejecutar `cargo test -p transport-axum --test openai_tool_stream_round_trip` y los tests de adaptadores afectados.
- [x] Ejecutar `cargo check --workspace` para detectar contratos compartidos incompletos.
- [x] Revisar diff: no migraciones/tablas SQLite, no logs de contenido/razonamiento/credenciales, no conversión de JSON textual a tool call, y campos opcionales no fuerzan datos inventados en proveedores.
- [x] Revisar el diff contra el presupuesto de 400 líneas. El diff tiene 1011 líneas modificadas; el usuario aprobó una excepción de tamaño porque la ampliación del contrato compartido requiere actualizar constructores y adaptadores coordinados. La aprobación es para el tamaño del cambio, no para publicar. No se crean commits ni se abre PR.
- [x] Registrar resultados exactos y cualquier bloqueo; no declarar terminado sin pruebas ejecutadas.

## Fuera de alcance y riesgo

No se modifica SQLite, analítica histórica, selección de modelos ni ejecución de herramientas. El cambio del modelo compartido exige actualizar constructores en varios crates; mantener sus valores ausentes como `None` o colecciones vacías y validar el workspace antes de cerrar.

## Evidencia

- `cargo fmt --all -- --check`: PASS.
- `git diff --check`: PASS.
- `cargo check --workspace`: PASS.
- `cargo test -p providers-ollama`: PASS (13 unitarias, 18 WireMock).
- `cargo test -p transport-axum --lib`: PASS (95 tests).
- `cargo test -p transport-axum --test openai_tool_stream_round_trip`: PASS (3 pruebas de recorrido).
- `cargo test -p providers-openai`, `providers-anthropic`, `providers-gemini` y `providers-groq`: PASS.
- `cargo test --workspace --no-run`: PASS antes de la última prueba añadida de normalización Anthropic; crates de producción se verificaron de nuevo con `cargo check --workspace`.
