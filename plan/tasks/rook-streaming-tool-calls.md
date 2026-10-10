# Plan de trabajo: tool calls estructurados en streaming de Rook

## Ruta
Delegated direct (investigación e implementación inline: no hay un agente de implementación disponible para esta misión; el código fuente no está indexado en CodeGraph).

## Presupuesto de revisión y estrategia
- Presupuesto recomendado: 400 líneas modificadas.
- Diff de la implementación original: 761 líneas modificadas.
- Diff de seguimiento: 651 líneas modificadas (594 adiciones, 57 eliminaciones).
- Estrategia: `size-exception`, previamente autorizada por el usuario para mantener una PR cohesionada; el usuario pidió una PR nueva tras fusionarse #311 por error.
- Justificación: provider, dominio, transporte y pruebas forman un recorrido funcional; dividirlo dejaría una capa intermedia sin completar el round-trip.

## Objetivo
Cerrar la brecha de la issue #279: preservar tool-call deltas OpenAI desde el provider hasta la respuesta SSE del transporte, sin perder streaming de texto.

## Criterios de aceptación
- El dominio representa deltas estructurados de llamadas a herramientas sin tipos JSON específicos del provider.
- El adaptador OpenAI preserva índice, identidad, nombre y fragmentos de argumentos en orden.
- El transporte OpenAI serializa esos campos con el wire format esperado.
- Se mantienen compatibles los streams solo de texto y los finish reasons `stop`/`tool_calls`.
- Entrada upstream malformada o tool call incompleta produce error determinista; nunca se presenta como una llamada válida completa.
- Pruebas cubren una llamada fragmentada, múltiples llamadas, intercalado texto/tool, regresión texto y errores de parsing/incompletitud.
- La documentación de providers refleja el estado comprobado; compatibilidad del agente solo se afirma con prueba end-to-end.
- EOF con bytes SSE pendientes se reporta como error; nunca se transforma en cierre exitoso.
- Las líneas `data:` de un mismo evento se unen conforme al formato SSE antes de decodificar JSON.
- Se limitan cantidad de tool calls, argumentos acumulados y tamaño de eventos SSE.
- Un test de `/v1/chat/completions` valida el wire SSE observable, `[DONE]` y errores propagados.

## Tareas
- [x] RPI-001 — Confirmar contratos actuales del dominio/provider/transport y elegir un tipo neutral para tool-call delta.
- [x] RPI-002 — Añadir pruebas RED verticales para parseo/preservación de tool-call deltas en OpenAI provider.
- [x] RPI-003 — Implementar tipo(s) de dominio y preservación de deltas en OpenAI provider; actualizar constructores afectados.
- [x] RPI-004 — Añadir pruebas RED/GREEN de wire format SSE OpenAI y serializar los deltas en transport-axum.
- [x] RPI-005 — Añadir cobertura de errores para chunks malformados y argumentos truncados al terminar el stream.
- [x] RPI-006 — Ejecutar suites focalizadas, revisar compatibilidad de otros providers y actualizar documentación.
- [x] RPI-007 — Añadir prueba RED y rechazar EOF si SseBuffer retiene un evento parcial.
- [x] RPI-008 — Añadir prueba RED y unir las líneas `data:` de un evento SSE antes del parseo.
- [x] RPI-009 — Añadir pruebas RED y límites explícitos para tamaño de evento, cantidad de llamadas y argumentos.
- [x] RPI-010 — Probar `/v1/chat/completions` por HTTP SSE, incluidos tools, finish_reason, `[DONE]` y error de upstream.
- [ ] RPI-011 — Verificar crates afectados, formato, Clippy y publicar una PR de seguimiento.

## Evidencia
- Inicial: `StreamChunk` solo contiene `delta: String`; el parser de OpenAI descarta `delta.tool_calls`; el DTO SSE de transporte solo serializa `role` y `content`.
- Baseline: `cargo test -p providers-openai --test provider` — 11 passed, 0 failed.
- Decisión de alcance: soporte para OpenAI `delta.tool_calls`; los docs del agente indican que su MVP requiere tool calls, mientras la issue #279 indica que no son requisito del MVP personal. La implementación no declarará compatibilidad sin E2E.
- RED confirmado: la prueba de provider inicialmente no compiló porque `StreamChunk` no tenía `tool_calls`; la de transporte compiló y falló porque el campo no se emitía.
- Verificación final: `cargo test -p rook-core -p providers-openai -p providers-groq -p providers-anthropic -p providers-ollama -p rook-usecases -p transport-axum` — todo PASS, incluyendo la prueba de round-trip.
- Lint/formato: `cargo clippy -p rook-core -p providers-openai -p providers-groq -p providers-anthropic -p providers-ollama -p rook-usecases -p transport-axum --all-targets -- -D warnings` — PASS; `cargo fmt --all -- --check` — PASS.
- Compatibilidad del agente: no se declara; `docs/agent/validation.md` conserva el estado no compatible hasta una prueba E2E con Rook en ejecución.
- Revisión de la PR verificada: el EOF parcial, parsing multilínea y acumulación sin límites ocurrían en código; la prueba round-trip existente no pasaba por el endpoint HTTP.
- RED/GREEN RPI-007: `stream_returns_error_for_eof_inside_sse_event` falló inicialmente con cero items; tras exponer `SseBuffer::pending_len()` y validar EOF, pasó con error provider explícito.
- RED/GREEN RPI-008: `stream_joins_multiple_data_lines_in_one_sse_event` falló con EOF JSON en la primera línea; ahora el payload SSE se une con newline antes de deserializarse.
- RED/GREEN RPI-009: pruebas verifican máximo de 64 calls, 1 MiB agregado de argumentos y 2 MiB por evento SSE.
- RED/GREEN RPI-010: prueba real por Router HTTP verifica tool-call fragments y finish_reason en SSE, `[DONE]` y propagación del EOF truncado como error SSE.
- Verificación de seguimiento: todas las pruebas de `sse-stream`, providers afectados y `transport-axum` pasan; Clippy con `-D warnings`, formato y diff-check pasan.
- Nombre del nuevo branch: `fix/rook-streaming-review-findings`; la PR #311 anterior fue mergeada por error y ahora el branch parte de `main` actualizado.

## Estado
Working — hallazgos P1/P2 y cobertura HTTP resueltos; suites y Clippy pasan; preparando nueva PR al branch fix/rook-streaming-review-findings.
