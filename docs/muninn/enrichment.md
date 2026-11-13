# Opt-in enrichment and geocoding

Muninn can enrich an entity from Wikimedia or Diffbot and geocode a `LOCATION`.
Both features are **disabled by default**, like the pinned reference: without
explicit configuration no provider is reachable and every existing call,
payload, stored property, counter and vector is unchanged. Nothing here runs
automatically — enrichment is never queued by `add_entity`.

```text
POST /enrich_entity  {"entity_id": "..."}
POST /geocode_entity {"entity_id": "..."}
```

Both controls require a `MemoryClient` built with `with_enrichment(...)` or
`with_geocoding(...)` **and** a property writer. Without the service the bridge
answers `501` with `not supported: enrichment is not configured` (or
`geocoding is not configured`); with a service but no writer it answers `501`
with the port's `not supported:` message. Both mutate nothing.

`Neo4jLongTermStore` implements the writer: successful enrichment stores the
documented properties on the entity node and successful geocoding stores
`(:Entity).location` as a point. Coordinate writes additionally require the
`entity_location_idx` point index and fail closed when it is absent. See
[Properties that may be written](#properties-that-may-be-written) and
[Persistence](#persistence-and-the-point-index-prerequisite).

## Configuration

Authority: agent-memory `9d45af5529ecb5f2fe352013468a11087f3163d4`,
`config/settings.py:613–726`, `enrichment/*.py`, `services/geocoder.py`.

| Setting | Default | Note |
|---------|---------|------|
| `enabled` | `false` | Nothing is called while false |
| `providers` | `["wikimedia"]` | Priority order; `"none"` selects nothing |
| `diffbot_api_key` | absent | Required only when Diffbot is selected |
| `wikimedia_rate_limit` | `0.5` | **Seconds between requests**, not requests/second |
| `diffbot_rate_limit` | `0.2` | Same unit quirk as above |
| `cache_results` | `true` | Successful and absent answers only |
| `cache_ttl_hours` | `168` | Minimum `1` |
| `background_enabled` | `true` | Retained as a configuration value; see limitations |
| `queue_max_size` | `1000` | Minimum `1` |
| `max_retries` | `3` | Non-negative |
| `retry_delay_seconds` | `60.0` | Minimum `1` |
| `entity_types` | `PERSON, ORGANIZATION, LOCATION, EVENT` | Empty means every type |
| `min_confidence` | `0.7` | Within `[0,1]` |
| `language` | `en` | Also picks the Wikipedia subdomain |
| `user_agent` | `neo4j-agent-memory/1.0` | Required by the providers' terms |
| `geocoding.enabled` | `false` | `LOCATION` entities only |
| `geocoding.provider` | `nominatim` | `google` requires `api_key` |
| `geocoding.cache_results` | `true` | Caches found answers only |
| `geocoding.rate_limit_per_second` | `1.0` | Applied as seconds between requests |
| `geocoding.user_agent` | `neo4j-agent-memory` | Required by the Nominatim ToS |

Unknown fields and unknown provider names are rejected, never ignored.
`validate()` runs before any provider call, cache insert or property write:

```text
invalid input: enrichment.wikimedia_rate_limit must be greater than 0
invalid input: enrichment.diffbot_rate_limit must be greater than 0
invalid input: enrichment.cache_ttl_hours must be at least 1
invalid input: enrichment.queue_max_size must be at least 1
invalid input: enrichment.max_retries must not be negative
invalid input: enrichment.retry_delay_seconds must be at least 1
invalid input: enrichment.min_confidence must be between 0 and 1
invalid input: enrichment.user_agent must not be blank
invalid input: enrichment.diffbot_api_key is required when the diffbot provider is selected
invalid input: enrichment.providers must not be empty when enrichment is enabled
invalid input: enrichment requires at least one configured provider adapter
invalid input: geocoding.rate_limit_per_second must be greater than 0
invalid input: geocoding.user_agent must not be blank
invalid input: geocoding.api_key is required when the google provider is selected
```

A rejected configuration leaves no entity, property, cache entry or counter
change. `ApiKey`'s `Debug` renders `Some(<redacted>)`, so a configuration dump
never prints a credential.

## Adapters and failure handling

| Adapter | Endpoint | Answer |
|---------|----------|--------|
| `WikimediaEnrichment` | `GET /page/summary/{title}` | description, summary, `wikipedia_url`, `wikidata_id`, `image_url` |
| `DiffbotEnrichment` | `GET /dql?type=query&token=…&query=…&size=1` | description (or summary), summary, `diffbot_uri`, image |
| `NominatimGeocoder` | `GET /search?q=…&format=json&limit=1&addressdetails=1` | coordinates, display name, place type, importance |

Every adapter enforces the configured spacing between requests (concurrent
callers are serialized), sends its declared `User-Agent`, follows at most three
redirects, and caps the response body at 1 MiB. Names are percent-encoded into a
single path segment. Requests never go to a non-http(s) endpoint carrying
credentials, and no test performs a live Wikipedia, Diffbot or Nominatim call:
wiremock replays recorded payloads.

Failures are in-band and non-fatal, with a closed vocabulary rather than raw
error text, so a message can never carry a URL, key or payload:

| `failure` | `status` | Meaning |
|-----------|----------|---------|
| `timeout` | `error` | The request exceeded the configured timeout |
| `transport` | `error` | No response (connection, TLS, redirect cap) |
| `service` | `error` | Non-success status other than 401/403/429 |
| `credential` | `error` | `401`/`403`: the provider rejected the credential |
| `response` | `error` | Malformed, oversized or out-of-range payload |
| `rate_limited` | `rate_limited` | `429`; the provider stops the chain |
| `unsupported_type` | `skipped` | No configured provider serves this entity type |

Absence is not a failure: `404`, a disambiguation page and an empty result set
produce `status: "not_found"` with `failure: null`. A `skipped` status with a
null failure means the caller's own filters (disabled, `min_confidence`,
`entity_types`) excluded the entity. A failing provider falls through to the
next configured one; `rate_limited` stops the chain, as upstream's composite
provider does.

## Cache and idempotency

The cache is explicit, in-memory and per service:

- Key: `TYPE:name`, upper-cased type and trimmed lower-cased name.
- Stored: successes and absences only, with the properties computed at insert.
- TTL: `cache_ttl_hours`; expired entries are dropped on read.
- Capacity: 10 000 entries, evicting the oldest tenth, as upstream.

A warm hit performs **no** provider call and repeats the *same* property values,
including the original `enrichment_timestamp`, so repeating an enrichment never
creates a second, differently-timestamped property set or any edge. Provider
failures are not cached, so a transient timeout is retried on the next call.

## Properties that may be written

Only the upstream `to_entity_attributes` surface, persisted on the `(:Entity)`
node by `Neo4jLongTermStore`'s `EntityEnrichmentStore` implementation:

| Attribute | Node property | Write rule |
|-----------|---------------|------------|
| `enriched_description` | `enriched_description` | Replaced on every success |
| `enriched_summary` | `enriched_summary` | Replaced on every success |
| `enrichment_provider` | `enrichment_provider` | Replaced on every success |
| `enrichment_timestamp` | `enriched_at` | Replaced; written with `datetime($enriched_at)`, not `datetime()`, so a cached replay stores the same instant |
| `wikipedia_url` | `wikipedia_url` | `coalesce`, keeps a stored value when the new answer has none |
| `wikidata_id` | `wikidata_id` | `coalesce` |
| `diffbot_uri` | `diffbot_uri` | `coalesce` |
| `image_url` | `image_url` | `coalesce` |

`diffbot_uri` and `image_url` appear only when a provider returned them. The
write is one `MATCH … SET … RETURN` statement on entity identity, using the
existing `id` parameter and `point({latitude, longitude})` conventions. No
ontology label, relationship, embedding, alias, counter, credential or raw
payload is written, and provider-private reasoning is never captured.

Coordinates are written to `(:Entity).location` with
`point({latitude: $latitude, longitude: $longitude})`, the same expression the
entity upsert already uses for `LOCATION`. A later `add_entity` upsert keeps a
stored point: the existing `ON MATCH` clause only replaces `location` when a new
coordinate is supplied.

## Persistence and the point-index prerequisite

- **Enrichment properties need no extra schema.** The writer only sets node
  properties that already exist on the reference's entity shape.
- **Coordinates need `entity_location_idx`.** Before writing, the adapter reads
  the live catalog (`SHOW INDEXES … WHERE name = 'entity_location_idx'`) and
  requires `POINT` on `(:Entity {location})` in state `ONLINE`. It never
  creates, repairs or drops schema. When the index is absent or unusable the
  write fails closed with:

  ```text
  storage error: entity location coordinates require the entity_location_idx point index
  on (:Entity {location}); run schema setup; no schema was changed
  ```

  `SchemaManager::setup_all` is the product schema path that creates the index
  (for example `just`-style setup or the app's schema step); the enrichment and
  geocoding services never do. The bridge maps this to HTTP `500`, while a
  missing **writer** configuration stays HTTP `501`.
- **Rejections change nothing.** A refused write leaves every property, the node
  count and the index inventory untouched.

## Explicit limitations

The `EntityEnrichmentStore` port keeps default implementations that return
`not supported: entity enrichment properties` and
`not supported: entity location coordinates`, so a client configured without a
writer still refuses loudly (`501`) instead of silently discarding data.

- **No read-back into `Entity.metadata`.** The documented properties are stored
  on the node (and `location` as a point), but `entity_from_value` does not fold
  them into `Entity.metadata` yet, so `get_entity` does not expose them. The
  values are readable through Cypher, which is how the live target asserts them.
- **No background queue or worker.** `background_enabled`, `queue_max_size`,
  `max_retries` and `retry_delay_seconds` are validated and retained, but
  enrichment happens only when a caller asks for it; upstream's asynchronous
  queue is not reproduced.
- **No fuzzy Wikipedia fallback.** A `404` is reported as `not_found`; upstream
  would retry through its search API.
- **No `related_entities`, `images` list or `enrichment_data` blob.** Only the
  fields in the table above are read from a payload and persisted; upstream's
  `enrichment_data` JSON blob is not written.
- **No Google geocoder adapter.** Its configuration is validated (an API key is
  required), but only Nominatim is implemented.
- **GM-11 schema work is not absorbed here.** No migration, index or Cypher
  change belongs to this unit.

## Shipped-binary registration (U6a)

The `muninn` binary builds both stages from the same conventions as the other
providers. Disabled by default: with no flag the binary behaves exactly as
before and contacts nothing.

| Flag | Environment | Default | Meaning |
| --- | --- | --- | --- |
| `--enrichment-enabled` | `NAM_ENRICHMENT__ENABLED` | `false` | Opt in to enrichment |
| `--enrichment-providers` | `NAM_ENRICHMENT__PROVIDERS` | `wikimedia` | Comma-separated priority order |
| `--enrichment-wikimedia-base-url` | `NAM_ENRICHMENT__WIKIMEDIA_BASE_URL` | pinned upstream root | REST root override |
| `--enrichment-diffbot-base-url` | `NAM_ENRICHMENT__DIFFBOT_BASE_URL` | pinned upstream root | REST root override |
| `--enrichment-diffbot-api-key` | `NAM_ENRICHMENT__DIFFBOT_API_KEY` | none | Required only for `diffbot`; never logged |
| `--enrichment-timeout-secs` | `NAM_ENRICHMENT__TIMEOUT_SECS` | `10` | Per-request deadline; zero rejected |
| `--geocoding-enabled` | `NAM_GEOCODING__ENABLED` | `false` | Opt in to geocoding |
| `--geocoding-provider` | `NAM_GEOCODING__PROVIDER` | `nominatim` | `google` validates but is not implemented |
| `--geocoding-base-url` | `NAM_GEOCODING__BASE_URL` | pinned upstream root | Search root override |
| `--geocoding-api-key` | `NAM_GEOCODING__API_KEY` | none | Never logged |
| `--geocoding-rate-limit-per-second` | `NAM_GEOCODING__RATE_LIMIT_PER_SECOND` | upstream default | Config value is authoritative |
| `--geocoding-timeout-secs` | `NAM_GEOCODING__TIMEOUT_SECS` | `10` | Per-request deadline; zero rejected |

An invalid or credential-bearing endpoint fails fast at startup with a redacted
message (exit 1) before Neo4j is contacted. An unreachable endpoint surfaces at
request time as an explicit provider error, never as a silent empty enrichment.
The persistence writer is the Neo4j store itself, so enriched properties and the
location point land on the entity node as documented above. `get_context` is
documented separately in `context.md`.

## Boundaries and evidence

Enrichment adds no field to any existing response: `add_entity` keeps its exact
payload and does not call a provider even when enrichment is configured.
Entity resolution, `owner_scope` isolation and every other memory tier are
unchanged.

Regression tests: `crates/muninn/core/src/enrichment.rs`,
`crates/muninn/providers/src/enrichment.rs` (wiremock, recorded payloads),
`crates/muninn/application/src/enrichment.rs`,
`crates/muninn/bridge/tests/enrichment.rs` and the query-shape tests in
`crates/muninn/neo4j/src/queries/long_term.rs`.

The live graph target is `crates/muninn/neo4j/tests/entity_enrichment.rs`. It is
**ignored** unless a caller provides a NEW authenticated empty synthetic
database and sets `MUNINN_ENRICHMENT_FRESH_SYNTHETIC=1`,
`MUNINN_FIXTURE_URI` and `MUNINN_FIXTURE_PASSWORD`; the caller owns teardown. It
asserts the persisted documented properties, the `location` point, warm-cache
idempotency, that a failed provider writes nothing, the pre-schema fail-closed
error, and that no write adds or removes an index.

That target was executed once against a disposable fixture: the locally
validated `neo4j:5.26-community@sha256:414adfe41c6f…` image, a fresh container
with `tmpfs` storage, a runtime-generated throwaway credential and no reuse —
the lifecycle documented in `dev/muninn/README.md`. Result: `1 passed`. The
container and the temporary credential were removed afterwards. A single
observed run is not milestone acceptance and is not a substitute for the
parent's fixture plan.

Raw commands, streams, exits and hashes belong under
`.agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory/evidence/u4b-enrichment-writer/`
(the earlier port-only state is retained under `.../u4-enrichment/`).

NOT_RUN and not claimed in this worktree: live provider calls, the live Neo4j
target (ignored until a fixture is provided), Silver/Gold/GM acceptance, and
milestone acceptance.
