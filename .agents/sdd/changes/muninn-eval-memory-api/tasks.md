# Tasks: Muninn `EvalMemory` Product API (`SDD-5B`, `F-13`, `BOLT-08-07`)

- [x] **5B.1:** Implement pure deterministic metrics (`recall_at_k`, `set_f1`, `bcubed_f1`), `EvalRequest`, `EvalReport` (with `has_skipped_required` release-gate guard), and `EvalMemory::run` in `crates/muninn/application/src/eval.rs`.
- [x] **5B.2:** Expose `MemoryClient::eval()` in `crates/muninn/application/src/client.rs` and re-export from `crates/muninn/application/src/lib.rs`.
- [x] **5B.3:** Add unit tests covering `Recall@k`, `set_f1`, `bcubed_f1`, and skipped-required-dimension detection.
- [ ] **5B.4:** Wire `client.eval.run` into `muninn-bridge` and execute live Neo4j integration fixture when Docker is available.
