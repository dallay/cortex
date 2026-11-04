# Preference filters and explicit targets

`POST /get_preferences_for` requires `user_identifier` and accepts:

- `active_only`: boolean, defaults to `true` (also for JSON `null`). Excludes
  preferences with an outgoing `SUPERSEDED_BY` edge, independently of `as_of`.
- `applies_to`: optional entity UUID string, requiring an actual `APPLIES_TO`
  edge from the user's preference to that entity. Unknown or unlinked targets
  return an empty list, as do unknown users.
- `as_of`: optional RFC3339 timestamp. The instant must be at or after
  `valid_from` and strictly before `valid_until`; null bounds are open.

All supplied filters are combined with AND before returning results. Results
are distinct preferences, newest `created_at` first, with no read limit.
To include superseded preferences in a historical query, set `active_only` to
`false`. Omitting `as_of` does not impose a current-time validity filter.
Malformed UUIDs, timestamps, and argument types return HTTP 400; they are not
silently treated as absent. Absent and JSON-null optional values are equivalent.

```json
{
  "user_identifier": "example-user",
  "applies_to": "01234567-89ab-4cde-8fab-0123456789ab",
  "as_of": "2026-01-01T00:00:00Z",
  "active_only": false
}
```

Rust callers can retain `get_preferences_for(user, active_only)` and
`LongTermStore::preferences_for(user, active_only)`. The additive methods
`get_preferences_for_filtered` and `preferences_for_filtered` take the user,
active flag, optional UUID, and optional UTC instant. A legacy store's default
filtered implementation delegates unfiltered calls but rejects nonempty filters
with `NotSupported`, rather than returning misleading unfiltered data.

## Preference creation

`POST /add_preference` retains its optional UUID-list `applies_to` input.
Every explicit target is looked up before embeddings, duplicate lookup, or any
writes. One missing target rejects the entire request with HTTP 404 and
`not found: entity <UUID>`, even in a mixed valid/missing list or when the
preference would otherwise deduplicate. Successful new and deduplicated
preferences still link all requested targets. No-target creation is unchanged.
No entity is implicitly created from a name or reference.

SOURCE-CONFLICT: pinned upstream `9d45af5529ecb5f2fe352013468a11087f3163d4`
silently skips nonexistent explicit target IDs. Whole-list rejection is the
explicitly approved product deviation, not upstream parity.

This is a preflight check, not a transaction redesign: concurrent deletion
between lookup and persistence can still leave a preference or partial links.
It does not promise atomicity across those calls.

## Verification scope

Application and HTTP regression tests cover preflight rejection, successful
linking, forwarding, defaults, and malformed input. The ignored Neo4j test
`preference_filters_and_boundaries_distinct_history_and_unknowns` exercises the
real adapter's AND filtering, both interval boundaries, open bounds, independent
historical activity, distinctness, ordering, and unknown/unlinked targets.
Run it only with `MUNINN_FIXTURE_URI` and `MUNINN_FIXTURE_PASSWORD` pointing to a
fresh, isolated, authenticated synthetic fixture, never an active database.
Fixture lifecycle and cleanup belong to the disposable fixture runner.

### Current live proof

Both real-adapter targets now passed against separate fresh authenticated Neo4j
fixtures: `preference_filters_and_boundaries_distinct_history_and_unknowns` and
`long_term_global_nodes_scoped_preferences_and_session_lifecycle`. Each run
reported one passed, zero failed, zero ignored, and zero filtered out, exit 0.
The exact current-source Linux executables ran with `--ignored --nocapture`;
this is distinct from the ordinary suite leaving these tests ignored.

The approved topology used an inspected `Internal=true` Docker network with no
host-published ports. Each Neo4j fixture used installed immutable image
`sha256:249a1ec6e3c52d82101a5ed33d6ed2b5ed5b0017b21a28c0bfebc335e87fb177`,
UID 7474, a 2 GiB memory limit and owned tmpfs. Authentication and empty data
were verified before tests. Linux Rust peers ran non-root with dropped
capabilities and no-new-privileges; executable hashes matched freshly compiled
outputs from the read-only source snapshot. No existing database was accessed.

Human-approved preparation created new explicitly executable task-owned tmpfs
and fetched only Cargo.lock-fixed dependencies into a private cache. Builds and
checks then ran offline with installed pinned Rust/Cargo 1.99.1; source and
lockfile hashes remained unchanged. Fresh host package checks reported 121 passed
and 11 ignored; Clippy with `-D warnings` and formatting passed. The opt-in
reasoning adapter test's ordinary-suite `ok` is not additional live proof.
Containers, anonymous fixture volumes, internal network and scratch were cleaned
up, with resource absence confirmed.

See [source-bound live proof and command evidence](../../.agents/rpi/plan/tasks/muninn-h2-delivery/evidence/h2-preference-live-bolt/peer-exec-3efb3d9d8e4e49ac834787c82ba33319/summary.md)
for source/executable hashes, exact commands, complete stdout/stderr, ignored
counts and cleanup records.

### Earlier preparation failures

Previous host-loopback attempts stopped before tests because `dory` either did
not publish Bolt or returned wildcard bindings instead of loopback. Those runs
were cleaned up and are preserved in the
[bounded Bolt fixture incident evidence](../../.agents/rpi/plan/tasks/muninn-h2-delivery/evidence/h2-preference-live-bolt/incident-1169b3fb788241b6b4ab5fff120d36db.md).
The component discarding the host address remains unestablished; the successful
peer topology does not depend on correcting it.

The initial offline Linux preparation also stopped at bind-target archive
mapping and default-tmpfs execution errors; isolated ordinary checks lacked
`atomic-waker v1.1.2`. That history remains in the
[offline peer preparation evidence](../../.agents/rpi/plan/tasks/muninn-h2-delivery/evidence/h2-preference-live-bolt/peer-2b7e2854f4614fe8a5fac4ead2713cf3/summary.md).
The later explicit storage/fetch authorization resolved these prerequisites
without modifying denied storage, global permissions, shared caches or product
source.

This product slice does not establish GM05 or H2 acceptance and does not replace
the earlier unavailable/blocked legacy verifier.
