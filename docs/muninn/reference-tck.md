# Oracle TCK baseline

## Scope and result

The pinned TCK tests ran against Agent Memory 0.8.0, Neo4j Community 5.26.32,
and controlled components. **This is not a run against Muninn.** Neither upstream
repository nor the six official test files were modified. Their hashes and the
local runner files appear in each report.

- `adapted` profile: **189 collected**, 166 passed, 9 failed, and 14 skipped.
  Overall result `FAIL`; runner exit code **2**.
- Adapted Bronze: **93/93 passed**.
- Adapted Silver: **64 passed and 3 failed**, out of 67.
- Adapted Gold: **9 passed, 6 failed, and 3 skipped**, out of 18.
- Adapted Platinum: **11 skipped**. These are not passes.
- `native-delete` profile: **13 selected** out of 189, 176 deselected;
  **1 passed and 12 failed**, without skips. All selected cases are Bronze.
  Result `FAIL`; exit code **2**. This profile does not run the rest of the TCK.

Thus, the adapter's passing Bronze does not demonstrate native Bronze for the
Python package: the official adapter replaces deletion. It also certifies no
Muninn tier.

## Reproducible evidence

- [Adapted report](../../.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/tck-adapted.json).
- [Native deletion report](../../.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/tck-native-delete.json).
- [Cases, clauses, tiers, results, and sources](specs/tck-baseline.csv).
- [Preparation and commands](../../dev/muninn/README.md).

The CSV preserves **202 historical results**: 189 from the adapted profile and
13 from the native profile. It adds **230 current product-profile results** on
the normative `b6c72c36bb7486881b87053ee1610f2caf972fb6` baseline. Profiles
overlap: these are not 432 distinct scenarios or 432 implemented capabilities.
The general `parity-matrix.csv` retains its `no_implementado` states: these
results describe the oracle, not a Rust implementation.

The report normalizes owned fields (`owner_id`, `session_id`, `id`, `msg_id`,
`message_id`, and `conversation_id`) to `<namespace>`. Assertions, scores, and
exception types remain unchanged. These JSON files describe execution, not
reusable golden contract values.

## Adapter mediation

The official Bolt adapter:

1. Deletes `TEXT INDEX message_content` and converts logical IDs to UUIDs.
2. Replaces `delete_message` with a direct Cypher deletion that returns `True`
   even when no node exists.
3. Uses a constant `[1.0, 0.0, ...]` embedding for every text and an LLM with
   fixed JSON responses.
4. Injects metadata manually and, when necessary, emulates session deletion
   and namespace clearing.

The local instrumentation records these hooks and overrides. The explicit
`clear_session` wrapper removes only data under the supplied owner/session;
it does not resolve all adapter failures.

## Observed gaps

- The native deletion subset confirms that native `delete_message` does not
  remove the node in these scenarios. The implementation contains two
  consecutive `WHERE` clauses; the method catches the exception and returns
  `False`. The reference is not changed to conceal that discrepancy.
- Silver: the deterministic LLM extractor does not reproduce expected entity
  and preference structures. This requires characterizing the LLM contract,
  not assuming model equivalence.
- Gold: relationships between entities and selected-message clearing differ.
- Skip reasons: 3 advanced retrieval cases, 5 PostgreSQL pgvector cases,
  4 experimental NAMS/GenAI cases, and 2 framework integrations.

## Pending gates

No tier, embedding semantics, graph retrieval, PostgreSQL, NAMS/GenAI, or
framework integration is claimed. A later functional acceptance run requires
real components or a fixture policy with explicit limits; the adapted result
does not satisfy that gate.
