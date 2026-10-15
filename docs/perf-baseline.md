# Rook perf baseline — 2026-10-15 (cerrar plan hoy, sin optimizar)

Harness: `crates/infrastructure/transport-axum/tests/gateway_perf.rs` — router + RouteRequest + FallbackRouter REALES, provider instantáneo in-process (aisla overhead gateway, sin red upstream). Repro: `cargo test -p transport-axum --test gateway_perf -- --nocapture`.

## Resultados (este entorno, debug build)

| Medición | Resultado |
|---|---|
| Overhead secuencial N=200 `POST /v1/chat/completions` | p50 **529µs**, p95 **807µs**, p99 **915µs** (assert p95 < 25ms ✅) |
| Concurrentes N=50 | wall **0.026s**, **1901.7 r/s** ✅ (>20 r/s baseline) |
| RSS durante tests (`/proc/self/status` VmRSS) | **~15.4–15.7 MB** (solo test harness + gateway in-process) |
| Cancelación streaming | drop del body SSE tras 1er frame vía gateway → in-flight **1→0** ✅ (Drop guard; sin CancellationToken explícito, depende de drop tokio) |
| Backpressure N=100 concurrentes | **100/100 OK** — sin `Semaphore`/límite hoy (gap documentado, no implementado) |

## Cancelación y backpressure (código)

- Combo: per-step **10s** (`COMBO_STEP_TIMEOUT`), total **60s** (`route_request.rs:684-687`). Streaming: fallback solo **antes del primer evento SSE** (`architecture.md:104-105`); después solo error del protocolo, sin concatenar modelos.
- No hay `CancellationToken` ni `Semaphore` de concurrencia en gateway; cancelación = drop de future/stream de tokio + timeouts reqwest por provider. Suficiente para local, gap si se expone con muchos clientes concurrentes.
- Circuit breaker: 3 fallos → 30s cooldown (`router_impl.rs:25-28`).

## Veredicto 0.1.0

**Sí, justifica avanzar a `0.1.0`** con alcance actual (2 protocolos, combos virtuales `rook/*` + legacy header, fallback ordenado, `/v1/models` dinámico, tool-calling Anthropic, E2E determinista). Condiciones antes del tag:

1. `cargo test -p transport-axum --test gateway_e2e` 12/12 + `gateway_perf` 4/4 en verde (este turno).
2. No ampliar proveedores ni routing strategies para el tag; lo que falta es operativo (límites/backpressure, dashboard cuotas #132) y va fuera del 0.1.0.
3. Probar local resto de semana como planeaste: 1 key local, 2 conexiones autorizadas, 1 combo, ambos protocolos, streaming + fallback, revocación.

Gaps aceptados para 0.1.0: sin rate-limit global documentado como backpressure, Gemini/Groq stream stub, sin CancellationToken explícito.
