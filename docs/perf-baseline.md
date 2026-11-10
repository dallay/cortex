# Rook perf baseline — 2026-10-15 (close out the plan today, no optimization)

Harness: `crates/infrastructure/transport-axum/tests/gateway_perf.rs` — REAL router + RouteRequest + FallbackRouter, instant in-process provider (isolates gateway overhead, no upstream network). Repro: `cargo test -p transport-axum --test gateway_perf -- --nocapture`.

## Results (this environment, debug build)

| Measurement                                           | Result                                                                                                                                         |
|-------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------|
| Sequential overhead N=200 `POST /v1/chat/completions` | p50 **529µs**, p95 **807µs**, p99 **915µs** (assert p95 < 25ms ✅)                                                                             |
| Concurrent N=50                                       | wall **0.026s**, **1901.7 r/s** ✅ (>20 r/s baseline)                                                                                          |
| RSS during tests (`/proc/self/status` VmRSS)          | **~15.4–15.7 MB** (test harness + in-process gateway only)                                                                                     |
| Streaming cancellation                                | drop of the SSE body after the 1st frame via gateway → in-flight **1→0** ✅ (Drop guard; no explicit CancellationToken, depends on tokio drop) |
| Backpressure N=100 concurrent                         | **100/100 OK** — no `Semaphore`/limit today (documented gap, not implemented)                                                                  |

## Cancellation and backpressure (code)

- Combo: per-step **10s** (`COMBO_STEP_TIMEOUT`), total **60s** (`route_request.rs:684-687`). Streaming: fallback only **before the first SSE event** (`architecture.md:104-105`); after that, only protocol error, with no model concatenation.
- There is no `CancellationToken` or concurrency `Semaphore` in the gateway; cancellation = dropping the tokio future/stream + reqwest timeouts per provider. Sufficient for local use, a gap if exposed to many concurrent clients.
- Circuit breaker: 3 failures → 30s cooldown (`router_impl.rs:25-28`).

## Verdict for 0.1.0

**Yes, it justifies moving to `0.1.0`** with the current scope (2 protocols, virtual combos `rook/*` + legacy header, ordered fallback, dynamic `/v1/models`, Anthropic tool-calling, deterministic E2E). Conditions before the tag:

1. `cargo test -p transport-axum --test gateway_e2e` 12/12 + `gateway_perf` 4/4 passing (this turn).
2. Do not expand providers or routing strategies for the tag; what is missing is operational (limits/backpressure, quota dashboard #132) and is out of scope for 0.1.0.
3. Test locally for the rest of the week as planned: 1 local key, 2 authorized connections, 1 combo, both protocols, streaming + fallback, revocation.

Gaps accepted for 0.1.0: no global rate limit documented as backpressure, Gemini/Groq stream stub, no explicit CancellationToken.
