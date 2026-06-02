# Spec Delta: API Key Repository

This delta updates `openspec/specs/api-key-repository/spec.md` to match the
SQLite implementation after #46. The original REQ-REP-1 through REQ-REP-6
remain valid; the changes here document the new JSON columns, the new
`rotate_hash` method, the new `list_paginated`/`count` methods, the
idempotent revoke semantics, and the lenient scope hydration.

The SQLite schema now includes two new columns added by
`crates/infrastructure/db-migration/src/migrations/V1__allowed_models_providers.sql`:

```sql
ALTER TABLE api_keys ADD COLUMN allowed_models_json   TEXT NOT NULL DEFAULT '[]';
ALTER TABLE api_keys ADD COLUMN allowed_providers_json TEXT NOT NULL DEFAULT '[]';
```

The `DEFAULT '[]'` means existing pre-#46 rows are retroactively
unrestricted — empty = unrestricted is the safe default.

---

## MODIFIED Requirements

### REQ-REP-6: Scopes JSON Serialization (UNCHANGED in intent, EXPANDED in path)

The repository SHALL serialize scopes as a JSON array of strings in
`scopes_json` (e.g. `["chat:read","chat:write"]`). The hydration path
`scopes_from_json` (`auth-sqlite/src/lib.rs:424`) MUST use
`ApiKeyScope::parse_lenient`, not `ApiKeyScope::parse`, so that pre-#46 rows
with the legacy `read`/`write` strings continue to be readable. See REQ-DOM-2
in `api-key-domain.md` for the lenient-vs-strict contract.

### REQ-REP-7: Restriction JSON Columns (NEW)

The repository SHALL persist `allowed_models` and `allowed_providers` as two
JSON columns, both `TEXT NOT NULL DEFAULT '[]'`:

- `allowed_models_json`: JSON array of model-id strings, e.g. `["gpt-4","claude-3-opus"]`.
- `allowed_providers_json`: JSON array of provider-id strings, e.g. `["openai"]`.

The serialization functions are:

- `models_to_json(&[ModelId]) -> String` (`auth-sqlite/src/lib.rs:443`):
  produces a JSON array of the IDs' string forms. An empty slice serializes
  to the literal string `"[]"`.
- `providers_to_json(&[ProviderId]) -> String` (`auth-sqlite/src/lib.rs:453`):
  same shape, for providers.

The deserialization functions are:

- `models_from_json(&str) -> Result<Vec<ModelId>, String>`:
  `Vec<String>` → `Vec<ModelId::new(_)>`.
- `providers_from_json(&str) -> Result<Vec<ProviderId>, String>`:
  same shape, for providers.

A deserialization failure (malformed JSON) MUST be propagated as
`ApiKeyRepositoryError::Database` by the calling query.

### REQ-REP-8: Restriction Hydration in find_active_by_hash (NEW)

`find_active_by_hash` MUST select `allowed_models_json` and
`allowed_providers_json` alongside the existing columns, and MUST populate
`ApiKeySubject.allowed_models` and `ApiKeySubject.allowed_providers` from
those columns. Implementation: `auth-sqlite/src/lib.rs:136` selects all six
JSON/relevant columns; `row_to_subject` (`auth-sqlite/src/lib.rs:370`)
parses both JSON columns and assigns them to the subject.

The 5-column SELECT in the pre-#46 version is now an 8-column SELECT (id,
label, scopes_json, tier, allowed_models_json, allowed_providers_json — and
the higher-level list/find variants also select the persistence-only fields).

### REQ-REP-9: Update Preserves Restriction Fields (NEW)

`update(record: &ApiKeyRecord)` MUST accept and persist changes to
`allowed_models` and `allowed_providers`. The SQL UPDATE statement
(`auth-sqlite/src/lib.rs:236`) sets `allowed_models_json = ?8` and
`allowed_providers_json = ?9` using `models_to_json` and `providers_to_json`
respectively. An empty `Vec<ModelId>` or `Vec<ProviderId>` SHALL be stored
as the literal JSON string `"[]"`, which the query layer reads back as an
empty Rust `Vec`. There is no separate "clear" code path; the empty vec IS
the cleared state (matches REQ-DOM-9 and REQ-DOM-10).

### REQ-REP-10: rotate_hash Replaces Credentials Only (NEW)

`rotate_hash(id, new_hash, new_prefix)` SHALL update only the `key_hash` and
`key_prefix` columns. All other fields (label, scopes, tier, is_active,
revoked_at, expires_at, created_at, last_used_at, allowed_models,
allowed_providers) SHALL be preserved unchanged. The SQL
(`auth-sqlite/src/lib.rs:321`) is:

```sql
UPDATE api_keys SET key_hash = ?1, key_prefix = ?2 WHERE id = ?3
```

This is the storage primitive behind `ManageApiKeys::rotate`, which
re-fetches the row after the update so the returned record reflects the new
prefix but otherwise still references the pre-rotation metadata.

### REQ-REP-11: Idempotent Revoke (NEW semantics — COALESCE)

`revoke(id, revoked_at)` SHALL use `COALESCE(revoked_at, ?1)` so that
revoking an already-revoked key preserves the original `revoked_at`
timestamp. The SQL (`auth-sqlite/src/lib.rs:306`):

```sql
UPDATE api_keys SET is_active = 0, revoked_at = COALESCE(revoked_at, ?1) WHERE id = ?2
```

This is a deliberate divergence from the old `now()`-every-time behavior:
once a key is revoked, the `revoked_at` is fixed to the first revocation
time. The repository SHALL still return `Ok(())` on the second call (idempotent
semantics), as before. The difference is in **which** timestamp survives.

(Previously: `revoked_at` was overwritten on each call.)

---

## ADDED Requirements

### REQ-REP-12: list_paginated and count (NEW)

The repository SHALL expose:

- `list_paginated(limit: i64, offset: i64) -> Result<Vec<ApiKeyRecord>, _>`:
  Returns a slice of records ordered by `created_at DESC`, filtering on
  `is_active = 1` (active-only, excluding revoked). Implementation:
  `auth-sqlite/src/lib.rs:334`.
- `count() -> Result<i64, _>`: Returns the count of active records
  (`is_active = 1`). Used by the use case to compute the `pagination.total`
  field. Implementation: `auth-sqlite/src/lib.rs:359`.

These two methods are how `ManageApiKeys::list_paginated` produces
`(records, total)` without loading the entire table.

### REQ-REP-13: Validation in Create/Update Round-Trip (NEW invariant)

For any record persisted via `create` and later read back via `find`,
`list`, or `find_active_by_hash`, the following equality MUST hold:

- `record.allowed_models.as_slice()` round-trips through `models_to_json` and
  `models_from_json` to a value that is `==` the original slice
  (semantically — `Vec<ModelId>` equality, not pointer equality).
- The same for `allowed_providers`.

This invariant is exercised by the regression test
`create_api_key_with_allowed_models_and_providers_persists_correctly`
(`auth-sqlite/src/lib.rs:1258`).

---

## Scenarios

### Scenario: Update with empty allowed_providers stores "[]"

- GIVEN an existing API key with `allowed_providers = [ProviderId::new("openai")]`
- WHEN `update` is called with `allowed_providers = vec![]`
- THEN the row's `allowed_providers_json` column is updated to the literal
  string `"[]"`
- AND a subsequent `find_active_by_hash` returns an `ApiKeySubject` whose
  `allowed_providers` is an empty Rust `Vec`
- AND the request middleware treats the key as unrestricted

(Implementation: `auth-sqlite/src/lib.rs:455` — `serde_json::to_string(&[])` →
`"[]"`.)

### Scenario: Legacy "read" scope persists round-trip

- GIVEN a row in `api_keys` whose `scopes_json` is `["read"]` (pre-#46 data)
- WHEN `find_active_by_hash` queries that row
- THEN the returned `ApiKeySubject.scopes` contains one `ApiKeyScope` whose
  `as_str()` is `"read"`
- AND no error is raised (lenient hydration)
- AND a tracing WARN is emitted with `scope=read`

### Scenario: rotate_hash preserves restrictions

- GIVEN a key with `allowed_models = [ModelId::new("gpt-4")]` and
  `allowed_providers = [ProviderId::new("openai")]`
- WHEN `rotate_hash(id, "new-hash", "rk-newxy")` is called
- THEN `key_hash` is updated to `"new-hash"` and `key_prefix` to `"rk-newxy"`
- AND `allowed_models` and `allowed_providers` are unchanged in the row
- AND a subsequent `find(id)` returns the same `allowed_*` values

### Scenario: Revoke twice preserves the original revoked_at

- GIVEN a key whose `revoked_at` is `"2026-05-01T10:00:00Z"` and `is_active = 0`
- WHEN `revoke(id, "2026-05-02T11:00:00Z")` is called a second time
- THEN the call returns `Ok(())`
- AND the row's `revoked_at` remains `"2026-05-01T10:00:00Z"` (COALESCE
  preserves the first value)
- AND `is_active` remains `0`
