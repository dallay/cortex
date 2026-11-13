# Owner, session, and namespace boundaries

## Prototype contract

A conversation is identified by `(owner_scope, session_id)`. The same
`session_id` can exist for two owners and for the ownerless namespace. Omitting
`user_identifier` selects only the latter; it never selects every owner.

`OwnerScope` keeps the exact identifier and rejects empty or whitespace-only
identifiers. Its encoding distinguishes `unowned` from `owned:<identifier>`,
even when the identifier itself contains those prefixes. The scope supplied by
the caller **is not authentication**.

The port requires a scope for resolution, ingestion, reads, listings, searches,
and deletion. Final mutations also filter by scope. An explicit UUID does not
allow adopting another owner's conversation or changing its session. A read by
session plus UUID requires both identities. A read by UUID alone remains a
different operation and is always scoped.

The schema keeps UUID uniqueness and adds composite uniqueness for
`(owner_scope, session_id)`. It does not backfill legacy properties at startup.
Operations on a relevant identity reject legacy conversations that have no
namespace. Listings and searches exclude those conversations. Migrating data
requires a separate explicit operation.

## Search and acceptance limits

Search first attempts a global ANN query through `db.index.vector.queryNodes`
with `k = limit`. Cypher applies the owner, optional session, embedding identity
and dimension, metadata, and threshold filters before returning results. If the
filtered result has fewer than `limit` rows, it runs a scoped exact search and
returns only that query's complete result, limited to `limit`. It does not mix
both lists or issue per-message queries.

The fallback computes cosine over messages that satisfy the scoped filters. It
prevents foreign candidates from hiding the owner's own results when ANN comes
back incomplete. If ANN fills the limit, ranking remains approximate: exact
equivalence and full recall are not guaranteed.

ANN errors for index, dimensions, or storage propagate without fallback. The
identity and dimension predicates applied after ANN do not protect the
procedure from dimensions that differ between the query and the index. There is
no oversampling, pagination, or additional configuration.

The fallback may scan every message in the scope and adds a second query when
ANN is incomplete. It does not demonstrate bounded performance, MiniLM
acceptance, C1 provenance, or closure of the H1 milestone.

The global administrative operation `clear_all_data` keeps its explicit guard
in the bridge and is not an everyday operation without an owner.

## Optional message idempotency

`add_message` accepts an optional `operation_key`. The key is limited to the
conversation resolved by `(owner_scope, session_id)`. The same key with the same
payload returns the stored message; the same key with a different payload
produces a conflict without adding a message. Without a key, repeated messages
remain distinct writes. The key can be reused after the message it was
associated with is deleted.

The payload includes role, content, metadata, and `generate_embedding`; it does
not include the generated UUID or the embedding provider's result. The
application first checks whether the operation already exists, so a replay does
not depend on another provider call. Writing also keeps the Neo4j adapter's
atomic check to protect races between concurrent requests. This key is not a
substitute for authentication or authorization.

Approved limit: the guarantee of not calling the provider covers replays of
already persisted messages. Two concurrent requests that check before the
message exists can both call the embedder; the atomic write prevents the
duplicate and resolves replay or conflict. The user accepts this limit to avoid
the operational cost of cross-process coordination/reservation and recovery of
pending operations.

## Focused validation

- Package tests: `cargo test -p muninn-core -p muninn-application -p muninn-neo4j -p muninn-bridge --locked`.
- Real opt-in test: `cargo test -p muninn-neo4j --test owner_namespace --locked -- --ignored`.
- The opt-in test requires `MUNINN_FIXTURE_URI` and `MUNINN_FIXTURE_PASSWORD`,
  only against a new synthetic Neo4j. It creates test data and must never point
  at an active database.

The real test covers identically named sessions, the ownerless namespace,
foreign or inconsistent UUIDs, individual and batch writes, reads, listings,
search with foreign candidates, scoped deletions, concurrent resolution, and
legacy rejection. It does not replace the TCK or the full chain and concurrent
failure matrix.
