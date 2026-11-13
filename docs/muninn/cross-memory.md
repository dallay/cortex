# Cross-memory `ABOUT` links

Muninn can now write the pinned cross-memory edge
`(:Preference)-[:ABOUT]->(:Entity)`. It is created only when a caller asks for
it through the existing `add_preference` request, it is property-free and
idempotent, and it never writes first. Nothing else changed: no new bridge
operation, no MCP surface, no read path and no schema migration.

This is H4 U3 implementation evidence, not Silver, Gold, GM, milestone or global
parity acceptance.

## Quick path

1. Send the existing `add_preference` call and add `about_entities` with entity
   UUIDs the caller already owns or can see.
2. Read the same response as before. No field is projected for the edge.
3. Verify the graph: `MATCH (p:Preference {id: $id})-[r:ABOUT]->(:Entity)`.
   Repeating the request keeps exactly one edge and no properties.

```json
{"category":"food","preference":"tea","user_identifier":"owner",
 "about_entities":["00000000-0000-0000-0000-000000000001"]}
```

## What the pinned reference actually defines

Authority: agent-memory `9d45af5529ecb5f2fe352013468a11087f3163d4`.

| Question | Pinned answer |
|----------|---------------|
| Nodes connected | `Preference` → `Entity` (`graph/queries.py:677–682`) |
| Direction | written `(p)-[:ABOUT]->(e)`, preference first |
| Properties | none; `MERGE` sets nothing and no `SET` clause exists |
| Which operation creates it | `LINK_PREFERENCE_TO_ENTITY`, which **no pinned caller invokes** |
| Additive, idempotent or derived | idempotent `MERGE`; repeating adds no edge and rewrites nothing |
| `MENTIONS` | `(Message)-[:MENTIONS]->(Entity)`; `MERGE` plus `ON CREATE SET` confidence/positions; the one writer today is `LINK_MESSAGE_TO_ENTITY` |
| `INITIATED_BY` | `(ReasoningTrace)-[:INITIATED_BY]->(Message)` |
| `TRIGGERED_BY` | `(ToolCall)-[:TRIGGERED_BY]->(Message)` |
| `TOUCHED` | `(ReasoningStep)-[:TOUCHED]->(Entity)`, three reference-resolution variants |

Two conflicts are recorded instead of resolved silently:

1. **Dead query versus live code.** The pinned preference→entity path that
   actually runs (`memory/long_term.py:1016–1053`) writes `APPLIES_TO`, not
   `ABOUT`; `LINK_PREFERENCE_TO_ENTITY` is unused. Muninn keeps `APPLIES_TO`
   exactly as it was, default-off, and adds `ABOUT` only on request. It does not
   rewrite or rename existing edges.
2. **`Fact` anchor.** The pinned TCK `SPEC.md` §1.6 also lists
   `ABOUT` | `Fact` | `Entity`, and the pinned `openwiki` diagram repeats it.
   The pinned implementation has no such query, and Muninn `Fact` has no entity
   reference (subject/predicate/object are strings). No `(:Fact)-[:ABOUT]->`
   edge is invented; the clause stays an unclaimed gap.
3. **No conversation/message/trace anchor.** The project spec §5.4 lists
   `ABOUT` among cross-tier edges, which reads as a conversation-level link.
   Neither authority anchors `ABOUT` on a `Conversation`, `Message` or
   `ReasoningTrace`; those tiers already use `MENTIONS`, `INITIATED_BY`,
   `TRIGGERED_BY` and `TOUCHED`. Muninn implements the pinned shape only.

## Request contract

`add_preference` accepts optional `about_entities` (list of UUID strings).
Absent, `null` and `[]` are equivalent and produce no edge.

| Input | Result |
|-------|--------|
| no `about_entities` | previous behavior byte for byte: no `ABOUT`, no new writes |
| `about_entities:[uuid]` | one property-free `ABOUT` edge from the caller's preference |
| repeated identical request | no second edge, no property change |
| id not an existing entity | `not found: entity <uuid>` (HTTP 404) with zero writes |
| non-list, or an entry that is not a UUID | HTTP 400 `'about_entities' must be a list of UUIDs` |

Unknown targets are preflighted with the existing `get_entity` lookup for the
whole `applies_to` + `about_entities` set before embedding, preference creation
and every edge write, so a rejected call leaves no preference node, no
`APPLIES_TO` edge and no `ABOUT` edge. The check is a read, not a lock:
concurrent entity deletion after it is not made atomic.

The deduplication hit path links the same edges onto the preference it returns,
so a deduplicated request does not lose the requested edge.

## Scope, isolation and failure modes

- `owner_scope` is preserved unchanged (S6). No conversation-traversal scoping,
  no new authorization rule, and no edge is readable across owners.
- Preference reads keep their existing `(:User)-[:HAS_PREFERENCE]->` filter.
  No read path traverses `ABOUT`; the edge is write-only today.
- Entities stay global, as before. `ABOUT` therefore adds no cross-owner
  visibility: the edge's only anchor is the preference the caller just created
  or deduplicated inside its own `user_identifier` scope.
- Pre-existing shared-path caveat, unchanged by U3: when a caller supplies no
  `user_identifier`, preference deduplication searches the unowned namespace
  globally, so a deduplicated hit may already belong to another user. In that
  case `ABOUT` — like the older `APPLIES_TO` write it mirrors — attaches the
  edge to that existing preference. Muninn does not tighten or re-interpret
  this scope rule here; it is recorded as an input to the S6 follow-up.
- Legacy adapters inherit the port default
  `not supported: preference ABOUT` (HTTP 501) and fail closed instead of
  silently dropping the requested edge. Like the rest of ingestion, the
  preference write is not rolled back, so this is not a new transaction.
- No DDL. Both endpoints are matched on `Preference.id` and `Entity.id`, which
  already carry unique constraints (`preference_id`, `entity_id`). No index,
  constraint or migration is added; a missing constraint makes the `MATCH`
  write nothing, which is a pre-existing schema fault, not an `ABOUT`
  requirement.

## Evidence and proof limits

Deterministic coverage:

- `application/src/long_term.rs` unit tests: default-off identity, requested
  edge, repeated request keeps one edge, unknown-target preflight with zero
  writes, cross-owner read attempt, and the legacy fail-closed default.
- `bridge/tests/bridge_router.rs` HTTP tests: unchanged response and
  `APPLIES_TO` behavior without `about_entities`, one edge when requested,
  malformed rejection before the store, and `404` with zero writes for an
  unknown target.
- `neo4j/src/queries/long_term.rs` unit tests: the statement keeps the pinned
  `MATCH`/`MERGE` shape, adds no property clause, and no preference read path
  gains an `ABOUT` traversal.

The ignored-by-default `neo4j/tests/cross_memory_edges.rs` target asserts the
live graph shape, property-free replay and owner isolation on a **new
authenticated empty synthetic** fixture with caller-owned teardown. The
explicit live run passed 1/1 on Neo4j 5.26; exit code and fixture cleanup are
recorded under
`.agents/rpi/plan/tasks/muninn-h4-about-live-acceptance/evidence/`.

Not claimed: no TCK scenario asserts `ABOUT`, so this is spec-and-code parity
for one edge, not TCK parity. GM-02/03/04/06/07 differential, Silver38 and
Gold cross-memory TCK gates have separate current PASS receipts under
`.agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory/evidence/`.
Those passes do not establish overall H4 acceptance or full product parity.
H2 A4 stays terminal and H3 keeps its accepted gaps.

Raw commands, streams, exits and source hashes live under
`.agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory/evidence/u3-cross-memory/`.
