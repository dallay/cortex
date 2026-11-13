# TCK / inventory / DDL audit

Normative reference: `b6c72c36bb7486881b87053ee1610f2caf972fb6`.

## Scope

- Full upstream TCK: 189 cases; Bronze includes 1 Bolt-only case.
- Inventory: 13 clauses, 24 normative requirements, 88 DDL rows, and 161 API
  operations × 2 backends.
- Schema source of truth: `CONSTRAINTS`, `INDEXES`, `TEXT_INDEXES`,
  `VECTOR_INDEXES`, and `POINT_INDEXES` in `schema.rs`.
- These findings do not promote G0 or override the project's broad limitations.

## DDL parity

The Rust source preserves the identities, labels, properties, and types of
Neo4j 5.26 DDL declared in the oracle. Dimensions are injected by the provider;
cosine similarity remains unchanged. FULLTEXT is not added: v1 uses TEXT.

| Family | Contract source | Product source |
| --- | --- | --- |
| Uniqueness | `schema.py:CONSTRAINTS` | `schema.rs:CONSTRAINTS` |
| RANGE | `schema.py:INDEXES` | `schema.rs:INDEXES` |
| TEXT | `schema.py:TEXT_INDEXES` | `schema.rs:TEXT_INDEXES` |
| VECTOR | `schema.py:VECTOR_INDEXES` | `schema.rs:VECTOR_INDEXES` |
| POINT | `schema.py:POINT_INDEXES` | `schema.rs:POINT_INDEXES` |

## Corrected residual parity mismatch

- `clear_all_data` previously preserved `Name`, but the frozen
  `BaseMemory.clear_all` excludes only `Lock` from its deletion universe.
- The product now preserves `Lock` and its operational `MuninnSchema` metadata;
  every other label is deleted. A `Name` used with the `Lock` label remains
  protected by `Lock`. An isolated `Name` node is not contractual memory.
- An adversarial maintenance test creates an `OrphanRecord` carrying user data
  and verifies that global clearing deletes it. This aligns `clear_all` without
  concealing data under labels outside an allowlist.

## Verification

- `dev/muninn/tests/test_normative_tck.py` validates revision and 189/92+1/67/18/11.
- `crates/muninn/neo4j/tests/schema_migrations.rs` validates v0→v1, concurrency,
  retry, object collisions, and data preservation.
- `dev/muninn/tests/test_schema_runner.py` validates cleanup and log capture
  without Docker.

API coverage gaps and backend-specific unsupported paths are detailed in
`ddl-inventory-audit.json`; this audit claims no advanced API coverage without a test.
