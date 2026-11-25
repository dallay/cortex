# Proposal: Muninn Buffered Memory Writer (`BOLT-08-04`)

## Intent

Implement the opt-in buffered fire-and-forget write lifecycle (`BOLT-08-04`) matching the pinned Neo4j Agent Memory contract (`9d45af5529ec`, `memory/buffered.py`). Currently, Muninn only defines `WriteMode::Buffered` and `max_buffered_writes` in `crates/muninn/core/src/config.rs`, with zero runtime implementation. This gap accounts for **11 of 138 eligible Bolt API rows**; closing it raises the **hypothetical API upper bound** from 88.41% to 96.38% (not measured parity until all receipts pass).

## Scope

### In Scope

- **Two Explicit Write Surfaces (Preventing Domain-Invariant Bypass):**
  - **Surface 1 — Application / Domain Buffered Surface:** Typed, domain-validated buffered write operations (e.g., deferred reasoning/mention/audit writes) that enforce application invariants and authorization policy (`SDD-1D`) before enqueueing.
  - **Surface 2 — Bolt-Parity Raw Buffered Surface (`client.buffered.submit(query, params)`):** Matches upstream `memory/buffered.py` for Bolt API parity and differential testing (`GM09-B01..B08`), explicitly gated on write-enabled operator/composition-root authorization (rejected fail-closed on read-only clients) so arbitrary Cypher cannot become an unauthorized side door.
- **All 11 Mapped Bolt API Operations (`BOLT-08-04`):**
  - `client.buffered` (accessor; `None` before initialization / when disabled)
  - `client.buffered.is_buffered`
  - `client.buffered.pending` (queued items waiting for drainer)
  - `client.buffered.submit` (inline await in `WriteMode::Sync`; bounded channel with backpressure in `WriteMode::Buffered`; lazy drainer startup on first submit; rejects submissions after `stop()`)
  - `client.buffered.flush` (ordered barrier waiting for all accepted writes prior to the barrier; no-op in sync mode; safe to call repeatedly)
  - `client.buffered.wait_for_pending` (alias delegating to `flush`)
  - `client.buffered.errors` (snapshot of background write errors recorded since startup, newest last)
  - `client.buffered.stop` (idempotent graceful shutdown: stops accepting new writes, drains all previously accepted writes, joins drainer task)
  - Client-level facades: `client.flush()`, `client.wait_for_pending()`, `client.write_errors()` (empty list when writer absent or after close).
- **Formalized Concurrency & Lifecycle Guarantees:**

| Behavior | Normative Guarantee |
|----------|---------------------|
| `submit()` | Queue acceptance is **not** equivalent to database persistence. |
| `flush()` | Uses an **explicit ordered barrier message** in the channel (not a racy counter/`Notify` check) to wait until every operation accepted *before* the barrier has completed execution. |
| `stop()` | Atomically closes the acceptance gate, rejects subsequent `submit()` calls, and drains all previously accepted operations before returning. |
| Cancellation | Dropping a caller future or initiating ordered shutdown must not silently lose operations already acknowledged at the acceptance point. |
| Retry | Retries are permitted **only** for explicitly idempotent operations (e.g., `operation_key` / `MERGE`-based writes). |
| Error callback | Isolated, bounded (timeout / non-blocking policy), and never allowed to stall or poison the background drainer worker. |
| Process crash | In-memory queue does **not** guarantee durability across process crashes (explicitly documented). |

- **Comprehensive Test Suite (`GM09-B01` through `GM09-B08`):**
  - Unit tests (deterministic fake `WriteExecutor`) and live Neo4j integration tests (`crates/muninn/neo4j/tests/buffered_writer.rs` + `crates/muninn/bridge/tests/buffered.rs`) covering sync passthrough, deferred mode, bounded backpressure, ordered flush barriers, error capture/callback isolation, graceful shutdown, concurrent `submit`/`flush`/`stop` races, and idempotent retry / duplicate prevention.

### Out of Scope

- Converting return-value memory APIs (`add_step`, `add_message`) into implicit background writes (upstream keeps `client.buffered` opt-in).
- Disk-backed persistent WAL queues across process crashes.

## Approach

1. **Hexagonal Design:** Define `BufferedWriteError` and `WriteExecutor` port in `muninn-core`, `BufferedWriter` in `muninn-application::buffered`, `Neo4jClient` adapter in `muninn-neo4j`, and bridge exposure in `muninn-bridge`.
2. **Ordered Barrier Protocol:** Model the channel payload as `enum WriterCommand { Write(WriteJob), Flush(oneshot::Sender<()>), Shutdown }`. A single acceptance gate tracks `accepting: bool`; `submit()` acquires the gate, enqueues `WriteJob` with bounded backpressure (`tokio::sync::mpsc::channel(max_pending)`), and releases. `flush()` sends `Flush(tx)` and awaits `rx`. `stop()` flips `accepting = false`, sends `Shutdown`, and awaits drainer completion.
3. **Bounded Error Retention & Security:** Cap retained `BufferedWriteError` entries (ring buffer + dropped-error counter), never log credentials or sensitive payload parameters, and bound `on_error` callback execution with a timeout.
4. **Staged Delivery:** `spec` and `design` proceed in parallel with `SDD-1`; production wiring into `apps/muninn/` is conditioned on `SDD-1C` (F-01) and `SDD-1D` (F-04).

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `crates/muninn/core/src/ports.rs` | Modified | `BufferedWriteError`, `WriteExecutor` port, bounded error-hook contract. |
| `crates/muninn/application/src/buffered.rs` | New | `BufferedWriter` with ordered barrier protocol, two write surfaces, backpressure, and graceful shutdown. |
| `crates/muninn/application/src/client.rs` | Modified | Expose `buffered()`, `flush()`, `wait_for_pending()`, `write_errors()`, and shutdown draining. |
| `crates/muninn/neo4j/src/client.rs` | Modified | Implement `WriteExecutor` for `Neo4jClient`. |
| `crates/muninn/bridge/src/` & `tests/buffered.rs` | Modified / New | Expose buffered API over bridge and add contract tests. |
| `crates/muninn/neo4j/tests/buffered_writer.rs` | New | Live Neo4j integration tests for `GM09-B01..B08`. |

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Raw `submit(query, params)` bypasses domain invariants or tenant boundaries. | Medium | Separate typed domain buffered surface from explicitly authorized operator/Bolt-parity surface; reject on read-only clients. |
| Slow or panicking `on_error` callback stalls the drainer indefinitely. | Medium | Wrap callback execution in a bounded timeout and catch/isolate errors without stopping the drainer loop. |
| Race between `submit()` and `stop()` drops an accepted write or deadlocks `flush()`. | Medium | Single acceptance gate + ordered `WriterCommand::Flush` / `Shutdown` barrier messages in the FIFO channel. |

## Rollback Plan

`BufferedWriter` is purely additive and defaults to `WriteMode::Sync`. Reverting removes `buffered.rs`, `MemoryClient` accessors, and bridge/test bindings without impacting existing sync write paths or Neo4j schemas.

## Dependencies

- `spec` and `design` can run in parallel with `SDD-1`.
- Production integration into `apps/muninn/` is conditioned on `SDD-1C` (`F-01`) and `SDD-1D` (`F-04`).

## Success Criteria & Common Definition of Done

- [ ] All 11 Bolt buffered API operations (`client.buffered.*`, `client.flush`, `client.wait_for_pending`, `client.write_errors`) match upstream semantics.
- [ ] Domain-buffered surface and authorized Bolt-parity surface are cleanly separated; read-only clients reject buffered writes fail-closed.
- [ ] Ordered barrier `flush()`, graceful `stop()`, bounded error callback, and all 8 scenarios (`GM09-B01..B08`) pass under unit and live Neo4j tests.
- [ ] Contract compared with the pinned upstream implementation (`9d45af5529ec`).
- [ ] Approved deviations explicitly documented.
- [ ] Hexagonal architecture boundaries preserved.
- [ ] RED → GREEN → REFACTOR evidence recorded.
- [ ] Unit and contract tests passed.
- [ ] Mandatory isolated Neo4j integration tests passed (`MUNINN_REQUIRE_NEO4J=1`).
- [ ] Concurrency, cancellation and failure paths verified.
- [ ] Owner isolation and authorization verified.
- [ ] Differential acceptance scenarios executed.
- [ ] Bazel and Cargo build inventory synchronized.
- [ ] Documentation and evidence registry synchronized.
- [ ] Acceptance receipts bound to the candidate commit.
- [ ] No skipped or ignored scenario counted as passed.
