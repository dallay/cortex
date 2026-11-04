# Long-term isolation and entity deletion

Scope of the verified behavior for the long-term surface: what separates two
owners, and what deleting an entity actually removes.

## Ownership model

The pinned reference and Muninn agree on the shape:

- **Entities, facts and relationships are global graph nodes.** They are not
  partitioned by owner; a fact is addressed by `subject`, an entity by its UUID.
- **Preferences are owner-scoped** through the `HAS_PREFERENCE` edge from a
  `User` node. Every preference read filters on that edge.

`user_identifier` is supplied by the caller and does not authenticate anybody.
Isolation here is data scoping, never an authorization boundary.

## Isolation guarantees

For `get_preferences_for` and its filtered variant:

- A read for one owner returns only preferences reachable from that owner's
  `User` node.
- Two owners that target the same entity never receive each other's records,
  with or without a target filter.
- An unknown owner returns an empty list, never an error and never a fallback to
  a global read.
- `preferences_by_category` and the entity/fact reads stay global by design;
  they are not owner-scoped, matching the reference.

## Entity deletion

`POST /delete_entity` runs `MATCH (e:Entity {id}) DETACH DELETE e`:

- The entity node and **all of its edges** are removed (`HAS_PREFERENCE`,
  `APPLIES_TO`, `MENTIONS`, `RELATED_TO`, `SAME_AS`, provenance links).
- The call returns `true` the first time and `false` for an already-absent id;
  a missing id is not an error.
- Preference and fact **records survive**. They are independent nodes: a
  preference keeps its owner link and remains readable through
  `get_preferences_for`, but every target filter referring to the deleted entity
  returns empty, because the `APPLIES_TO` edge no longer exists.
- `get_related_entities` for the deleted id returns empty, and the surviving
  endpoints of a deleted relationship remain readable.

## Not provided

There is no deletion API for preferences, facts or relationships. The pinned
reference exposes only `delete_entity_provenance` on the long-term surface and
`delete_message` on short-term, so adding owner-facing deletes would be a product
extension, not parity. Consequence for the privacy work in H6: deleting an entity
is **not** an erasure primitive for user data; per-record erasure still needs a
deliberate, separately authorized design.

## Evidence

- Test: `crates/muninn/neo4j/tests/long_term_isolation_deletion.rs`
  (`long_term_user_isolation_and_entity_deletion_semantics`), ignored by default
  and run only against an explicitly configured fixture.
- Live run: one test passed, zero failed, zero ignored, against a fresh
  authenticated empty Neo4j container published on loopback only; the listener
  was confirmed as `127.0.0.1` and the container was removed afterwards.
- Fixture contract: `MUNINN_FIXTURE_URI` and `MUNINN_FIXTURE_PASSWORD`.
