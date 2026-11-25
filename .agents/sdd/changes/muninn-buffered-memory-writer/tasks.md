# Tasks: Muninn Buffered Memory Writer (`BOLT-08-04`, SDD-2)

- [x] **2.1:** Define `BufferedWriteError`, `BufferedWriteJob`, and `WriteExecutor` port in `crates/muninn/core/src/ports.rs`.
- [x] **2.2:** Implement `BufferedWriter` in `crates/muninn/application/src/buffered.rs` with ordered barrier `flush()` (`WriterCommand::Flush`), bounded backpressure (`mpsc::channel(max_pending)`), bounded error ring retention, bounded `on_error` callback timeout, explicit idempotent-only retry, graceful `stop()`, and two surfaces (`submit_domain_write` & Bolt-parity `submit`).
- [x] **2.3:** Wire `with_buffered_writer` (rejected fail-closed on read-only clients), `buffered()`, `flush()`, `wait_for_pending()`, and `write_errors()` on `MemoryClient` in `crates/muninn/application/src/client.rs`.
- [x] **2.4:** Implement `WriteExecutor` for `Neo4jClient` in `crates/muninn/neo4j/src/client.rs`.
- [x] **2.5:** Verify all 8 scenarios (`GM09-B01..B08`) in `buffered::tests` (6/6 PASS) and `cargo clippy --locked -p muninn-core -p muninn-application -p muninn-neo4j --all-targets -- -D warnings` (0 warnings).
- [x] **2.6:** Synchronize `crates/muninn/application/BUILD.bazel` — verified via `python3 dev/bazel/generate_build_files.py --check` (`No drift detected in 43 generated package(s)`).
