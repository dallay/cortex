# Fused prompt context

`get_context` is the pinned fused prompt document. The bridge method
`get_context` reproduces `MemoryClient.get_context` from the pinned reference
(`src/neo4j_agent_memory/__init__.py`, revision `9d45af5529ecb5f2fe352013468a11087f3163d4`).

## Request

```json
{"query": "Ada", "session_id": null, "include_short_term": true,
 "include_long_term": true, "include_reasoning": true, "max_items": 10,
 "user_identifier": "alice"}
```

Only `query` is required. `max_items` defaults to `10`, the three `include_*`
flags default to `true`, and `user_identifier` scopes the caller's namespace.
Malformed arguments are rejected before any store read.

## Document

Three sections joined by a blank line; an empty section is omitted, and an
empty memory returns the empty string:

- `## Conversation History` from the short-term tier. With a `session_id` the
  recent conversation comes first (`### Recent Conversation`), then the
  semantically relevant past messages (`### Relevant Past Messages`).
- `## Relevant Knowledge` from the long-term tier: `### User Preferences` and
  `### Relevant Entities`. Preferences follow the caller's namespace; entities
  are global, matching the store's own scoping model.
- `## Similar Past Tasks` from the reasoning tier, limited to
  `max_items / 2` traces, success-only, rendered without the tier's own
  `###` header because the fused section supplies the `##` header.

## Isolation

Messages and preferences are read inside the requesting `user_identifier`
namespace. Reasoning traces are global in Muninn's model (an owner link exists
on writes, but similarity search is not owner-filtered), so a trace section is
the same for every caller. A test drives a cross-owner attempt and proves no
foreign message or preference leaks.

## Differences from the pinned reference

- The pinned client renders the reasoning section by nesting the tier's own
  `### Similar Past Tasks` header inside the fused `## Similar Past Tasks`
  header. Muninn deduplicates that header: the fused document renders the
  entries once under the `##` header. This is a deliberate, documented wire
  decision, not an upstream behaviour claim.
- `user_identifier` scoping of the knowledge sections is stricter than the
  pinned client, which passes no owner filter to the long-term tier. Muninn
  keeps its verified isolation model (see `cross-memory.md`).

## Checks

`crates/muninn/bridge/tests/context.rs` pins the document shape, the empty
state, the switched-off sections, argument rejection without reads, the
session-prefixed document and cross-owner isolation through the public HTTP
surface.
