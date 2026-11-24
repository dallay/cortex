# Muninn schema operations

## Scope

This document describes v1 migrations and schema initialization. It does not
declare H1 or the remaining Muninn requirements accepted. Migrations run no
inference and do not depend on embedding-provider identity.

## Coordination and version

`SchemaManager::setup_all` executes this protocol:

1. Create and validate the `muninn_schema_id` constraint on `MuninnSchema(id)`.
2. Use `MERGE` to initialize the reserved `MuninnSchema {id: 'product'}` marker
   with `version=0`, `state='ready'`, and `revision=0`, only if absent.
3. Open a coordinator connection independent of the application pool.
4. Acquire the marker's exclusive transactional lock by incrementing `revision`
   through a direct dependency on its previous value.
5. Reject negative, future, or invalid versions and states other than `ready`
   before executing product DDL.
6. Inspect existing definitions and reject collisions or incompatible constraints
   and RANGE, TEXT, POINT, and VECTOR indexes.
7. Create missing objects with `IF NOT EXISTS`, without deleting or rebuilding
   existing objects. Constants in `schema.rs` are the canonical inventory.
8. Wait for managed indexes and validate their definitions and availability.
9. Commit `version=1`, `state='ready'`, and the revision in the coordinator
   transaction. Release the lock on commit or rollback.

Supported versions are 0 and 1. Version 0 is a retry point; it does not establish
a completed migration. Version 1 does not skip schema validation. There is no
automatic downgrade path.

The lock serializes processes that follow this protocol, not administrators or
writers that bypass coordination. The dedicated connection avoids exhausting a
single-connection application pool. Integration tests in an ephemeral database
cover concurrent bootstrap and upgrade.

## Product DDL order

1. Constraints from `CONSTRAINTS`.
2. Composite `conversation_owner_session` and `message_operation` constraints.
3. RANGE indexes from `INDEXES` and TEXT indexes from `TEXT_INDEXES`.
4. Indexes from `VECTOR_INDEXES` with the requested dimension and cosine similarity.
5. POINT indexes from `POINT_INDEXES`.
6. Non-vector waiting and validation, followed by vector waiting and validation.

POINT indexes are required for version 1. Their errors are no longer converted
to warnings. The method creates no FULLTEXT indexes. TEXT and FULLTEXT are
different types.

## Partial failures and recovery

Neo4j does not permit this DDL and the marker in a single transaction. Each
successful DDL operation may persist even if a later operation fails. There is
no global atomicity or automatic object rollback.

On failure, the client rolls back the coordinator transaction; it does not
advance the version or revision. A new database retains the v0 marker created
before locking. Compatible objects already created allow the process to be retried.

Before retrying a failed operation:

1. Retain a verified backup when persistent data are involved.
2. Inspect `SHOW CONSTRAINTS`, `SHOW INDEXES`, and the specific error.
3. Determine whether an incompatible definition needs external correction.
4. Authorize any modification or restoration separately.
5. Repeat initialization and check the result.

Do not delete objects, data, or the marker as routine recovery. `clear_all_data`
deletes memories but preserves `MuninnSchema` nodes; it is not a schema-history
reset mechanism.

## Legacy conversation migration

`SchemaManager::setup_all` never writes `owner_scope`. A conversation created
before the owner namespace existed stays unscoped, and every operation that
resolves its `session_id` fails with
`ambiguous legacy conversation requires explicit migration`.

No automated migration ships with Muninn. The required action is an explicit
operator migration that assigns the correct `owner_scope` to each legacy
conversation before the session is used again.

1. Retain a verified backup of the database.
2. List the unscoped conversations and their sessions:
   `MATCH (c:Conversation) WHERE c.owner_scope IS NULL RETURN c.id, c.session_id`.
3. Determine the owner of every listed session. The identifier supplied by the
   caller is not authentication, and the legacy rows carry no owner evidence.
   A session that belongs to no owner takes the `unowned` namespace.
4. Write the scope with the storage encoding the application uses: `unowned`
   for the ownerless namespace, `owned:<identifier>` otherwise. Keep the
   identifier exact, including any whitespace.
   `MATCH (c:Conversation) WHERE c.id = $id SET c.owner_scope = $owner_scope`.
5. Resolve the duplicates before writing. The composite uniqueness of
   `(owner_scope, session_id)` rejects a legacy conversation that would land
   on a session already owned by the target scope. Merge or remove the
   duplicate in a separate, authorized operation.
6. Verify that the list is empty:
   `MATCH (c:Conversation) WHERE c.owner_scope IS NULL RETURN count(c)`.

The migration is not part of schema initialization, and the schema manager does
not perform it. It changes stored data, so it needs the same backup and
authorization as any other data operation. See
[Owner, session, and namespace boundaries](owner-session-namespace.md#legacy-conversations-and-summary-entities)
for the rejection that the migration removes.

## Startup and commands

`apps/muninn/src/main.rs` constructs `MemoryClient` with
`SchemaManager::setup_all`. This path is also used by `health`: the command can
modify schema and embedding bindings. **It is not a read-only diagnostic.**

Focused local tests:

```sh
cargo test -p muninn-neo4j --lib
cargo clippy -p muninn-neo4j --lib --tests -- -D warnings
cargo test -p muninn-neo4j --test schema_migrations --no-run
```

The ignored `schema_migration_contract` integration test requires a new empty
database and `MUNINN_SCHEMA_ISOLATED_TEST=1`, with that fixture's URI and password.
Never direct these tests at application data. `dev/muninn/schema_runner.py`
provides ephemeral containers on an `internal` network, without published ports.

## Evidence

All paths below are relative to
`.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/`:

- `schema-migration-red-20261103/`: initial failure from the absent marker.
- `schema-collision-red-20261103/`: late vector-collision rejection reproduced
  before correction.
- `schema-point-red-20261103/`: POINT collision reproduced.
- `schema-maintenance-red-20261103/`: improper metadata deletion during
  maintenance reproduced.
- `schema-migrations-final-20261103-r1/`: expanded integration passed, including
  concurrent bootstrap, invalid versions, retries, data preservation, and collisions.
- `schema-non-vector-v1-20261103/`: five scenarios passed.

These receipts establish their specific scope. They do not replace Bronze,
GM-01, or C1.

## Sources

- [Implementation](../../crates/muninn/neo4j/src/schema.rs).
- [Version policy](../../crates/muninn/neo4j/src/migrations.rs).
- [Integration](../../crates/muninn/neo4j/tests/schema_migrations.rs).
- [Non-vector tests](../../crates/muninn/neo4j/tests/schema_non_vector.rs).
- [Startup](../../apps/muninn/src/main.rs).
- [Development harness](../../dev/muninn/README.md).
