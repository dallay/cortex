# Opt-in entity resolution

`add_entity` now returns `(Entity, DeduplicationResult)` in Rust. Existing callers
still default to no resolution. Explicit HTTP `resolve: true` or
`deduplicate: true` enables the single-entity resolution policy; either switch
requests the same decision and both default to false (including null).
This does not wire extraction or message ingestion.

## Request and response

```json
{"name":"Jane Mary","entity_type":"PERSON","deduplicate":true}
```

The HTTP response retains every existing entity field at the top level and adds
`deduplication`, containing `is_duplicate`, `action`, `matched_entity_id`,
`matched_entity_name`, `similarity_score`, and `match_type`. Actions are `none`,
`flagged`, and `merged`; unmatched fields are null and the unmatched score is 0.
This additive projection is the deliberate S5 compatibility decision, **not**
upstream tuple-serialization parity. No bridge operation was added.

Optional `resolution` is an object overriding policy defaults. Unknown fields
and malformed types are rejected. `generate_embedding` defaults to true; false
suppresses the embedding call without disabling textual resolution. Malformed
boolean controls return HTTP 400 before dependencies. Configuration is validated
before embedding, candidate lookup, or writes, including an explicitly supplied
configuration when resolution is disabled.

## Defaults and source conflict

The pinned deployment measured by GM-04 runs without the optional rapidfuzz
extra, so the reference's ImportError path disables fuzzy scoring entirely;
the frozen GM-04 fixture therefore declares `use_fuzzy_matching: false` for
both engines (the product honors it, the reference reaches the same state
through its missing dependency). The authority for the algorithm itself is
agent-memory `9d45af5529ecb5f2fe352013468a11087f3163d4`:
`src/neo4j_agent_memory/config/settings.py:423–510` and
`memory/long_term.py:33–107,725–896,1692–1811`.

- Auto-merge: **0.90**, review/flag: **0.85**, candidate limit: **12**.
- Exact: 1.0, fuzzy: 0.85, semantic: 0.8.
- Same-type matching and embedding blocking are enabled by default.
- The fuzzy scorer is fixed to `token_sort_ratio`; normalized indel similarity
  is computed without a new dependency.
- `resolve_on_ingest=true` and `context_window_chars=90` are retained policy
  defaults, not ingestion wiring or a claim that this API captures context.

**Documented deviation from upstream documentation:** its published
0.95/0.9/10 defaults are stale relative to its pinned executable configuration.
Muninn follows the code values above, not those documentation values. This is
not a tolerance adjustment. Thresholds must be finite and within [0,1], review
must not exceed auto-merge, and the candidate limit must be at least one.
The ordering error follows the upstream configuration text:
`resolution.review_threshold must be <= resolution.auto_merge_threshold (got X > Y)`.

## Decisions and graph effects

Candidate generation is a `LongTermStore` port. Legacy adapters return
`not supported: entity resolution candidates`, never an inferred distinct result.
The Neo4j adapter fetches bounded textual and vector buckets, excludes merged
nodes, applies the optional type restriction, deduplicates candidate UUIDs, and
returns at most `candidate_limit` candidates. Exact surfaces rank first; vector
scores rank the remainder. Textual blocking compares name, canonical name and
aliases, and shares whole tokens. Vector-index and provider failures propagate;
resolution does not use the legacy search helper that suppresses index errors.
ANN filtering can return fewer candidates; this does not establish recall.

Exact surfaces score 1.0. Other candidates use token-sort similarity when it
clears the fuzzy threshold, averaged with qualifying embedding evidence when
both are available; otherwise qualifying embedding evidence supplies the score.
The best candidate determines the inclusive bands:

- **Merged, score >= 0.90:** return the matched entity, append the incoming
  surface as an alias, and backfill an absent/wrong-width embedding. No temporary
  incoming node or SAME_AS edge is created. Re-adding the stored name itself
  follows the original upsert so supplied descriptions still apply.
- **Flagged, 0.85 <= score < 0.90:** create/upsert exactly one incoming node and
  link it to the untouched candidate with one SAME_AS edge, status `pending`.
  No manual merge, property copying, edge rewiring or candidate deletion occurs.
  Repeating identical input keeps the incoming UUID and the existing edge.
- **Distinct, score < 0.85 or no candidate:** original entity upsert, no edge.

These graph writes reuse entity upsert/lookup and `link_same_as`, including its
existing `UPDATE_SAME_AS_STATUS` behavior. Upstream `long_term.py:754–780` returns
early for a differently named merged surface; `:803–896` upserts the incoming
node before adding a pending edge. Manual `merge_entities` remains separate.
Multiple writes are not a new transaction; concurrent candidate deletion or
provider/storage failures can leave earlier successful writes persisted.

Without an embedder, textual candidates still resolve, following the reference
resolver's explicit no-vector behavior (`resolution/ontology.py:599–606,985–989,1071–1072`).
This is not an always-distinct fallback. U1 does not implement the entire
ontology resolver: alias gazetteers, type-specific normalization, prefix/context
rules and intra-episode clustering are not claimed as parity by this single-name
path. GM differential acceptance remains pending.

## Preserved boundaries and evidence

Preference/fact duplicate folding remains at **0.95**, unchanged. Entities remain
global; preferences retain their existing scope; conversations retain Muninn's
`owner_scope`. No conversation-traversal scoping or authorization rule is added.
No model is installed. The existing Embedder port supplies vectors, including
verified-bundle MiniLM when configured by the product. Test-only scores/vectors
do not establish C1, MiniLM numerical acceptance, semantic quality, Silver38,
Gold, GM acceptance or milestone acceptance.

HTTP regression tests in `bridge/tests/bridge_router.rs` cover exact band
boundaries, rejection before mutation, same-type matching, candidate budgets,
repeat stability, explicit unsupported adapters and no-resolution preservation.
The ignored `neo4j/tests/entity_resolution.rs` target checks real graph effects
only when explicitly configured against a **new authenticated empty synthetic**
fixture. Its caller owns teardown. An ignored count means NOT_RUN.

Raw commands, streams, exits and hashes belong under
`.agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory/evidence/u1-resolution/`.
The parent owns review, further live verification and human acceptance.
