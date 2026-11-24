# Tasks: Muninn Integrity, CI Stability, and Evidence Foundation (SDD-1)

## Unit 1A — CI Stability (`F-10`, `F-11`, `F-21`)
- [x] **1A.1 (F-11):** Gate `PinnedMiniLmManifest::synthetic_oversized_for_tests` and `synthetic_for_tests` in `crates/muninn/c1-exporter/src/model_manifest.rs` with `#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]`. Verified with `cargo clippy --locked -p muninn-c1-exporter --all-targets -- -D warnings` and `model_manifest` unit tests (12/12 PASS).
- [x] **1A.2 (F-10):** Add `docker_supports_linux_containers` check + `auth.flush()` + safe exit-status diagnostics in `crates/muninn/neo4j/tests/common/fixture.rs` with unit tests (`fixture_tests` 2/2 PASS in `consolidation` and `durability_reconnect`).
- [ ] **1A.3 (F-21):** Triage OSV-Scanner GitHub Actions log once network/CI log access is available (blocked locally: `osv-scanner` not installed and no network in sandbox).

## Unit 1B — Evidence & Traceability Integrity (`F-18`, `F-19`, `F-20`)
- [x] **1B.1 (F-20):** Add `summarize_report`, `render_markdown`, `validate_markdown_sync`, and `--check-md` to `dev/muninn/api_test_traceability.py` via TDD (`test_api_test_traceability.py -k markdown` 2/2 PASS) and regenerate `docs/muninn/specs/api-test-traceability.md`.
- [x] **1B.2 (F-19):** Implement fail-closed `dev/muninn/parity_evidence_validator.py` with 10 TDD tests (`test_parity_evidence_validator.py` 10/10 PASS) checking 92 capabilities, zero missing file refs, receipt invariants, and 4 simultaneous non-averaged gates; reconcile `docs/muninn/specs/bolt-parity-final-closure.md`.
- [x] **1B.3 (F-18):** Add isolated target recipe `muninn-bolt-parity-target` and `ci-muninn-evidence` to `justfile`.

## Unit 1C — Transactional Ingestion (`F-01`)
- [x] **1C.1:** Define `AtomicExtractionEntity`, `AtomicExtractionRelation`, `AtomicExtractionBatch`, and `ShortTermStore::append_message_atomic` in `crates/muninn/core/src/ports.rs`.
- [x] **1C.2:** Write TDD tests (`atomic_ingestion_commits_single_txn_and_rolls_back_without_orphans` in `crates/muninn/application/src/local_operator_tests.rs` and `atomic_extraction_batch_reports_emptiness` in `ports.rs`) covering single-txn commit, rollback without orphan messages, `operation_key` retry after rollback, post-commit idempotent replay without re-extracting, and conflicting payload rejection.
- [x] **1C.3:** Implement single-transaction `Neo4jShortTermStore::append_message_atomic` in `crates/muninn/neo4j/src/short_term.rs` (message + entities + `MENTIONS` + relationships inside one `neo4rs::Txn`, rolling back cleanly on any error) and wire `ShortTermMemory::add_message` + `IngestionExtraction::to_atomic_batch`.
- [x] **1C.4:** Verify with `cargo clippy --locked -p muninn-core -p muninn-application -p muninn-neo4j -p muninn-bridge --all-targets -- -D warnings` (0 warnings) and `cargo test --locked -p muninn-core -p muninn-application -p muninn-bridge` (all PASS).

## Unit 1D — Authorization & Owner Isolation Policy (`F-04`)
- [x] **1D.1:** Define `MemoryKind`, `MemoryScopeClass` (`Private`, `Workspace`, `Global`, `RestrictedAdmin`), `AccessOperation`, `AccessContext`, `TargetResource`, and `MemoryAccessPolicy` in `crates/muninn/core/src/ports.rs`.
- [x] **1D.2:** Write TDD tests (`memory_access_policy_classifies_all_memory_kinds`, `memory_access_policy_enforces_private_workspace_global_and_admin_boundaries`) covering all 4 scope classes and blank workspace rejection.
- [x] **1D.3:** Implement `MemoryAccessPolicy` and verify zero regressions across `muninn-core`, `muninn-application`, `muninn-neo4j`, and `muninn-bridge`.
