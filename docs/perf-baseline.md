# Rook Perf Baseline — 2026-10-15 (close plan today, unoptimized)

Harness: `crates/infrastructure/transport-axum/tests/gateway_perf.rs` — REAL router + RouteRequest + FallbackRouter, instant in-process provider (isolates gateway overhead, without upstream network). Repro: `cargo test -p transport-axum --test gateway_perf -- --nocapture`.

## Results (this environment, debug build)

| Measurement | Result |
|---|---|
| Sequential overhead N=200 `POST /v1/chat/completions` | p50 **529µs**, p95 **807µs**, p99 **915µs** (assert p95 < 25ms ✅) |
| Concurrent N=50 | wall **0.026s**, **1901.7 r/s** ✅ (>20 r/s baseline) |
| RSS during tests (`/proc/self/status` VmRSS) | **~15.4–15.7 MB** (only test harness + in-process gateway) |
| Streaming cancellation | SSE body drop after 1st frame via gateway → in-flight **1→0** ✅ (Drop guard; no explicit CancellationToken, relies on tokio drop) |
| Backpressure N=100 concurrent | **100/100 OK** — no `Semaphore`/limit today (documented gap, not implemented) |

## Cancellation and Backpressure (Code)

- Combo: per-step **10s** (`COMBO_STEP_TIMEOUT`), total **60s** (`route_request.rs:684-687`). Streaming: fallback only **before the first SSE event** (`architecture.md:104-105`); afterwards protocol error only, without concatenating models.
- No `CancellationToken` or concurrency `Semaphore` in gateway; cancellation = drop of tokio future/stream + reqwest timeouts per provider. Sufficient for local, gap if exposed to many concurrent clients.
- Circuit breaker: 3 failures → 30s cooldown (`router_impl.rs:25-28`).

## Verdict 0.1.0

**Yes, justifies proceeding to `0.1.0`** with current scope (2 protocols, `rook/*` virtual combos + legacy header, ordered fallback, dynamic `/v1/models`, Anthropic tool-calling, deterministic E2E). Conditions before tagging:

1. `cargo test -p transport-axum --test gateway_e2e` 12/12 + `gateway_perf` 4/4 passing (this turn).
2. Do not expand providers or routing strategies for the tag; remaining items are operational (limits/backpressure, quota dashboard #132) and remain outside 0.1.0.
3. Test locally the rest of the week as planned: 1 local key, 2 authorized connections, 1 combo, both protocols, streaming + fallback, revocation.

Accepted gaps for 0.1.0: no global rate-limit documented as backpressure, Gemini/Groq stream stub, no explicit CancellationToken.
