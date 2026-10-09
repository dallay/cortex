# Plan de trabajo: tool calls estructurados en streaming de Rook

## Ruta
Delegated direct (investigación e implementación inline: no hay un agente de implementación disponible para esta misión; el código fuente no está indexado en CodeGraph).

## Presupuesto de revisión y estrategia
- Presupuesto recomendado: 400 líneas modificadas.
- Diff medido: 761 líneas modificadas (724 adiciones, 37 eliminaciones).
- Estrategia: `size-exception`, autorizada explícitamente por el usuario.
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

## Tareas
- [x] RPI-001 — Confirmar contratos actuales del dominio/provider/transport y elegir un tipo neutral para tool-call delta.
- [x] RPI-002 — Añadir pruebas RED verticales para parseo/preservación de tool-call deltas en OpenAI provider.
- [x] RPI-003 — Implementar tipo(s) de dominio y preservación de deltas en OpenAI provider; actualizar constructores afectados.
- [x] RPI-004 — Añadir pruebas RED/GREEN de wire format SSE OpenAI y serializar los deltas en transport-axum.
- [x] RPI-005 — Añadir cobertura de errores para chunks malformados y argumentos truncados al terminar el stream.
- [x] RPI-006 — Ejecutar suites focalizadas, revisar compatibilidad de otros providers y actualizar documentación.

## Evidencia
- Inicial: `StreamChunk` solo contiene `delta: String`; el parser de OpenAI descarta `delta.tool_calls`; el DTO SSE de transporte solo serializa `role` y `content`.
- Baseline: `cargo test -p providers-openai --test provider` — 11 passed, 0 failed.
- Decisión de alcance: soporte para OpenAI `delta.tool_calls`; los docs del agente indican que su MVP requiere tool calls, mientras la issue #279 indica que no son requisito del MVP personal. La implementación no declarará compatibilidad sin E2E.
- RED confirmado: la prueba de provider inicialmente no compiló porque `StreamChunk` no tenía `tool_calls`; la de transporte compiló y falló porque el campo no se emitía.
- Verificación final: `cargo test -p rook-core -p providers-openai -p providers-groq -p providers-anthropic -p providers-ollama -p rook-usecases -p transport-axum` — todo PASS, incluyendo la prueba de round-trip.
- Lint/formato: `cargo clippy -p rook-core -p providers-openai -p providers-groq -p providers-anthropic -p providers-ollama -p rook-usecases -p transport-axum --all-targets -- -D warnings` — PASS; `cargo fmt --all -- --check` — PASS.
- Compatibilidad del agente: no se declara; `docs/agent/validation.md` conserva el estado no compatible hasta una prueba E2E con Rook en ejecución.

## Estado
Ready — implementación, pruebas y documentación verificadas; `size-exception` autorizada; publicación en curso.
