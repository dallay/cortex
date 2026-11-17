# H4 spaCy oracle image (U5a)

This document records the new spaCy oracle image built for H4 extraction. It is
one U5 infrastructure step, **not** the GM harness: no GM-02/03/04/06/07 capture,
no Gold run, no tolerance and no milestone acceptance exists yet. The existing
`muninn-gm05-oracle:20261104` image is untouched and stays authoritative for
every accepted reference capture.

## Quick path

```sh
# Rebuild (repository root, arm64 host; the model wheel is already cached)
docker build --platform linux/arm64 \
  -f dev/muninn/Dockerfile.oracle-spacy \
  -t muninn-h4-oracle-spacy:20261105 dev/muninn

# Prove the image needs no runtime network and loads its model
docker run --rm --network none -i muninn-h4-oracle-spacy:20261105 - <<'PY'
import spacy
model = spacy.load("en_core_web_sm")
print(model.meta["name"], model.meta["version"], model.pipe_names)
PY

# Static contract (no Docker, no network, no model import)
python3 -B dev/muninn/tests/test_oracle_spacy_image.py
```

The static test proves four things: every requirements line is exact and
`--hash=sha256` pinned, the model is copied by filename and digest-verified
rather than fetched, both stages pin the base image by digest, and the image
runs as `7474:7474` with no credential-bearing configuration.

## Why the oracle needed extending

The pinned reference (`agent-memory` `9d45af55`) implements the selected
extraction stage as spaCy `en_core_web_sm` with a constant confidence of `0.85`
(H4 decision S3). The accepted GM05 oracle image pins no spaCy, no model wheel
and no extraction dependency, so it cannot execute that stage at all. A separate
image was the only option that keeps the accepted oracle byte-identical: the new
image never rewrites, retags or replaces `muninn-gm05-oracle:20261104`.

## Exact image identity

| Item | Observed value |
| --- | --- |
| Tag | `muninn-h4-oracle-spacy:20261105` |
| Image id / repo digest | `sha256:0f54322068ba6bebf27718c6b0d44ea85391e79a4801e2995c8adf9941645678` |
| Created | `2026-11-05T18:20:55.889115995Z` |
| Platform | `linux/arm64` |
| Size | 634 MB (BuildKit reports it as a manifest list with provenance attestation) |
| Base image | `python:3.13.18-slim@sha256:76baad6037690d6e8c4397fbcaef17bca6f19eecef822b708fe8fa3417d30bb5` (same digest as `Dockerfile.oracle`; Debian trixie, glibc 2.41, CPython 3.13.18, pip 26.2.2) |
| Runtime user | `7474:7474`, `WORKDIR /harness`, `ENTRYPOINT ["/opt/venv/bin/python"]` |
| Unchanged witness | `muninn-gm05-oracle:20261104` `sha256:0f4d85e328e4…`, inspect JSON SHA-256 `01698419c1dc…` before and after this build |

Installed inventory (50 distributions): the 48 hash-pinned requirements in
`requirements-oracle-spacy.txt`, plus `en_core_web_sm==3.8.0` and the venv's
own `pip==26.2.2`. Key pins:
`spacy==3.8.16`, `thinc==8.3.13`, `blis==1.3.3`, `numpy==2.5.4`,
`pydantic==2.14.1`, `pydantic-core==2.50.1`, `neo4j==6.4.1`, `weasel==1.0.0`.

Loaded model (reported by the pipeline inside the image): meta name
`core_web_sm`, meta version `3.8.0`, `meta["spacy_version"]`
`>=3.8.0,<3.9.0`, components `tok2vec, tagger, parser, attribute_ruler,
lemmatizer, ner`, 18 NER labels, 50 tagger labels.

## How it is pinned

| Input | Source of truth |
| --- | --- |
| Base image | digest `76baad60…`, identical to the accepted oracle base |
| Python packages | `dev/muninn/requirements-oracle-spacy.txt`, 48 exact pins, every line carries `--hash=sha256`, wanted by `--require-hashes` |
| Package provenance | exported from the pinned reference `uv.lock` (SHA-256 `30a0c40d5bba…`), `spacy` extra closure; not a fresh latest resolution |
| Model wheel | `en_core_web_sm-3.8.0-py3-none-any.whl`, SHA-256 `1932429db727d4bff3deed6b34cfc05df17794f4a52eeb26cf8928f7c1a0fb85`, copied from the build context, checked with `sha256sum -c` and installed `--no-deps --no-index` |
| Build context | the Dockerfile-specific ignore file exposes only the requirements file and that wheel (12.89 MB transferred) |

The requirements file also carries the reference project's own base dependency
pins (`neo4j`, `pydantic`, `pydantic-settings`, `python-dotenv`, `pytz`,
`typing-*`), because the closure is exported from the reference lock rather than
re-resolved here. `colorama` (Windows) and `numpy==2.4.6` (Python < 3.12) are
dropped: this image targets one platform and installs no conditional lines.

Regenerate pins only by re-exporting the lock (see
`dev/muninn/requirements-oracle-spacy.in`); never by resolving new versions.

### Deliberate difference from `Dockerfile.oracle`

The accepted oracle builds its venv at `/tmp/venv` and copies it to `/opt/venv`.
Console scripts copied that way keep a dead shebang (`/tmp/venv/bin/python`),
which the first build of this image reproduced: `pip` and `spacy` failed with
`exec … no such file or directory` while `python -m pip` still worked. This
image creates the venv at its final path `/opt/venv` instead. The runtime user,
base digest, hash-pinned install and single-entrypoint shape are unchanged, and
the static test pins both the working path and the absence of `/tmp/venv`.

## Pinned confidence and label mapping reference

S3 fixes the stage as spaCy `en_core_web_sm` with **confidence `0.85`**. That
`0.85` is the pinned extractor's constant `SpacyConfig.default_confidence`, not a
model score: spaCy NER emits no per-entity probability here, so every extracted
entity of this stage carries `0.85`. The image only makes the model runnable; it
does not change, re-weight or re-derive the constant, and U2's inclusive `0.85`
filter stays as documented in `docs/muninn/extraction.md`.

The pinned `DEFAULT_TYPE_MAPPING` / `DEFAULT_SUBTYPE_MAPPING` of
`extraction/spacy_extractor.py` at `9d45af55` maps exactly the 18 labels this
image reports:

| spaCy label | POLE+O type | Subtype |
| --- | --- | --- |
| `PERSON` | `PERSON` | — |
| `ORG` | `ORGANIZATION` | — |
| `NORP` | `ORGANIZATION` | `GROUP` |
| `GPE` | `LOCATION` | `GEOPOLITICAL` |
| `LOC` | `LOCATION` | `GEOGRAPHIC` |
| `FAC` | `LOCATION` | `FACILITY` |
| `EVENT` | `EVENT` | — |
| `PRODUCT` | `OBJECT` | `PRODUCT` |
| `WORK_OF_ART` | `OBJECT` | `CREATIVE_WORK` |
| `LAW` | `OBJECT` | `LEGAL_DOCUMENT` |
| `LANGUAGE` | `OBJECT` | `LANGUAGE` |
| `DATE` | `EVENT` | `DATE` |
| `TIME` | `EVENT` | `TIME` |
| `MONEY` | `OBJECT` | `CURRENCY` |
| `QUANTITY` | `OBJECT` | `QUANTITY` |
| `ORDINAL` | `OBJECT` | — |
| `CARDINAL` | `OBJECT` | — |
| `PERCENT` | `OBJECT` | — |

Observed on a synthetic sentence (recorded stream in the evidence path):
`Alice/PERSON 0–5`, `Acme Corporation/ORG 15–31`, `Paris/GPE 35–40`,
`2024/DATE 47–51`, with character offsets. This is a smoke observation of the
model, not an accuracy measurement and not a Gold or differential result.

## What the image deliberately does not contain

- No GLiNER2, `rapidfuzz`, `sentence-transformers` or MiniLM artifact: the
  embedding space stays the byte-verified `MiniLmEmbedder` bundle outside this
  image.
- No model download path: no `spacy download`, no index lookup for the model,
  no `--index-url`/`--find-links` override, no `curl`/`wget`.
- No credentials, tokens, provider SDKs or environment secrets; config carries
  only the base image's `PATH`, `LANG`, `GPG_KEY`, `PYTHON_VERSION`,
  `PYTHON_SHA256`.
- No Neo4j server, no fixture, no harness code and no GM runner: those belong to
  the next unit and to compose-based fixtures.
- No tolerance, threshold or pre-registration: this step introduces none, so
  nothing here is a deviation from an accepted tolerance.
- No runtime network dependence: every probe in the evidence stream ran with
  `--network none`.
- Nothing was pushed, published, retagged or staged.

## U5a status at the time of that unit

- Extraction quality: precision/recall/F1 of this model on any fixture, and
  statistics for merge and ranking, are **NOT_RUN**. No baseline exists, so no
  tolerance can be pre-registered yet.
- The U2 HTTP sidecar is not wired into this image; extraction throughput,
  timeouts and the `0.85` filter are unverified against reference spans beyond
  the four-entity smoke observation.
- Gold cross-memory, GM-02/03/04/06/07 and Silver38 differentials are
  **NOT_RUN**; the GM harness that composes the existing gm01/gm05/gm08
  components on top of this image is the next unit.
- Live authenticated Neo4j ingestion of these spaCy entities is **NOT_RUN**.

## Evidence

Raw command streams, exits, image identity, build log, runtime probe, inventory,
the unchanged-GM05 witnesses and per-file hashes are retained under
`.agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory/evidence/u5a-oracle-image/`.

## U5b GM-02/03/04 harness

The GM-02 (extraction modes), GM-03 (spaCy fidelity) and GM-04 (resolution and
deduplication) harness is implemented and deterministic-tested. At the U5b
implementation cutoff, no real measurement had run; U6 later captured both
sides as recorded below. The harness is built by
composing the accepted components rather than adding a framework: `gm05_build`
for the source-bound offline build, `gm08_runner` for the dory lifecycle,
preflight and faithful-copy helper, `gm08_peer` for the loopback bridge,
binary-consume and fresh-database guards, and `gm08_contract` for the
fail-closed comparison shape. No `gm05` or `gm08` file is modified. GM-06 and
GM-07 extend the same modules in U5c (see the last section).

## Quick path (the U6 command)

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -B dev/muninn/gm_h4_runner.py \
  --sources dev/muninn/.cache/sources \
  --cargo-cache "$HOME/.cargo/registry" \
  --output .agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory/evidence/<new-run>
```

The runner builds the current workspace offline, creates one fresh authenticated
tmpfs database per side on an internal network with no published ports, and runs
one peer per side inside `muninn-h4-oracle-spacy:20261105`. `--binary`,
`--tolerance`, `--epsilon` and `--deviation` do not exist. Comparison only, from
two retained captures:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -B dev/muninn/gm_h4_contract.py \
  --expected reference.json --actual product.json
```

Exit 0 is an offline comparison pass, exit 2 is invalid input or a difference.

## What each differential compares

| Item | Cross-side values | Per-side invariants (an equal-but-corrupt pair fails) |
| --- | --- | --- |
| GM-02 | scenario order, entity inventory, `MENTIONS` edges with `confidence`/`start_pos`/`end_pos`, `RELATED_TO` inventory | auto spans reproduce their entity name at 0.85, skip creates no mention and no entity, explicit links only the supplied references, replay does not duplicate nodes |
| GM-03 | `name`, POLE+O `type`, `subtype`, `start_pos`, `end_pos`, constant `0.85`, `spacy` extractor, in document order | offsets in range and in character order, span reproduces the name, frozen texts verbatim, empty/whitespace text yields no entity, the stage stream is never shorter than the GM-02 mentions of the same text |
| GM-04 | attempt order, observed band/action, matched candidate, score, match type, canonical identity, `SAME_AS` inventory | band derived from the observed score with the declared 0.90/0.85 thresholds, matched candidate inside the declared pool, merge returns the matched identity, a flagged mention keeps its own node with exactly one pending `SAME_AS` edge, a null match reports 0.0 |
| GM-06 | the five shared enrichment properties, the stored geocode point, the observed enrichment status and the recorded provider request totals | the property document carries exactly the declared names, the recorded provider answer reproduces each value, a warm replay repeats them with zero requests, the geocode point is the recorded coordinate, spacing between recorded requests respects the declared rate limit, the closed failure vocabulary is product-only |
| GM-07 | entity and message ids with their scores in the returned order, the `get_context` document, the `RELATED_TO` inventory and the declared graph neighbours | every id is a declared token, scores are finite and non-increasing along each ranked list, the declared edge exists, the context document reproduces the declared section order, and the scoped search returns no foreign-owner message |

## Declared tolerances and rules (disclosed by hash)

The only numeric tolerance is an absolute **`1e-9`** applied to resolution
similarity scores, `SAME_AS` confidence and the GM-07 entity/message scores.
Every other value — including `MENTIONS` confidence, band thresholds, spans,
labels, the geocode point and ranked order — is compared exactly. The
declaration digest for the current live measurement is
`57caed06516c58af2ddcc73ae2e36ffc52b99e68748d42a55b502f67cede2122`
(SHA-256 over `gm_h4_contract.DECLARATIONS`: the nineteen carried rules plus
`GM06-01..07` and `GM07-01..07`, 33 in total). Any observed difference that
would need a wider tolerance is a finding to report, never a relaxation.

Two declared asymmetries, both recorded before measurement:

1. **Additive MENTIONS provenance.** The pinned reference writes only
   `confidence`, `start_pos` and `end_pos` (`graph/queries.py::LINK_MESSAGE_TO_ENTITY`).
   Muninn also writes `derived`, `extractor`, `support`, `source_message_ids` and
   `evidence`. Those five columns are validated on the product side against the
   frozen fixture and are never compared across servers; their presence on the
   reference side fails.
2. **Reference-only extraction fields.** The pinned `ExtractedEntity` also
   carries `context`, `attributes` and `id`; the Muninn stage emits none of them,
   so H4 claims no parity for them. They are accepted and dropped, and an
   unknown field on either side fails.

## Known limits of this harness

- **Product enrichment and geocoding stages are wired, default off.** H4-U5C-F1
  recorded that the shipped binary registered neither stage and that the bridge
  lacked `get_context`. The follow-up unit closed both: the binary builds the
  stages from `--enrichment-*` / `--geocoding-*` flags (or `NAM_ENRICHMENT__*` /
  `NAM_GEOCODING__*` environment) through the existing client builders, and the
  bridge exposes the pinned fused `get_context` document. A GM-06/07 run
  without those flags measures the default-off path, which is the correct
  comparison for an unconfigured deployment.
- **The flagged band is not deterministically reachable.** Both sides receive
  the same controlled vectors (TCK `MockEmbedder` SHA-256 1536 for the
  reference, the byte-identical `hash` provider for the product), whose cosines
  between distinct names are near zero. Exact surfaces force `1.0`/merged and
  absent candidates force `0.0`/none; a mid-band score depends on the observed
  vector/fuzzy mix and is compared, never predicted. GM-04 validates the
  pending-`SAME_AS` structure whenever a side reports `flagged`, and compares
  the merge structure that the forced cases do exercise.
- **The vector space is the declared test-only one.** GM-04 feeds both sides
  the same SHA-256 1536 embedding — the reference TCK mock and the byte-identical
  product `hash` provider — so the differential measures resolution policy, not
  the MiniLM space S4 selects for production. Resolution inside the pinned
  `MiniLmEmbedder` bundle is **NOT_RUN** here and C1/MiniLM numerical acceptance
  stays a separate pending H1 item.
- **Extraction quality statistics, Gold cross-memory, Silver38 and live Neo4j
  ingestion are NOT_RUN** in U5b. They do not belong to a harness unit.

## Deterministic checks

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -B dev/muninn/tests/test_gm_h4_contract.py
PYTHONDONTWRITEBYTECODE=1 python3 -B dev/muninn/tests/test_gm_h4_extract.py
PYTHONDONTWRITEBYTECODE=1 python3 -B dev/muninn/tests/test_gm_h4_search.py
PYTHONDONTWRITEBYTECODE=1 python3 -B dev/muninn/tests/test_gm_h4_runner.py
```

Raw RED and GREEN streams, exits, per-file hashes and the declaration digest are
retained under
`.agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory/evidence/u5b-harness/`
and, for GM-06/GM-07, under `.../evidence/u5c-harness/`.

## U5c GM-06 and GM-07 harness

GM-06 (enrichment and geocoding) and GM-07 (text, vector and graph search plus
`get_context`) are implemented in the same harness discipline and are
**deterministic-tested only**. No live differential ran in U5c: the H4
measurement belongs to U6. No live Wikipedia, Diffbot or Nominatim call, no
network access and no model download exists in any test here.

### GM-06 — recorded providers, no live call

The frozen fixture holds the provider answers (`gm06.providers`) and the five
attempts. The reference side builds its pinned chain
`CachedEnrichmentProvider(WikimediaProvider)` through the pinned factory and
binds `httpx.AsyncClient` to a recorded transport: every request is matched by
path against the fixture, recorded with a timestamp, and an unmatched request
fails closed before any write. The product side drives the bridge
`enrich_entity` / `geocode_entity` controls. Recorded attempts:

| Attempt | Operation | Reference answer | Product answer |
| --- | --- | --- | --- |
| `enrich-success` | enrich | `200` summary | same recorded payload |
| `enrich-replay` | enrich | warm cache, zero requests | warm cache, zero requests |
| `enrich-not-found` | enrich | `404` then the pinned fuzzy search (2 requests) | `404`, no fuzzy fallback (1 request) |
| `enrich-failure` | enrich | `503`, `error` | `503`, `error` + `service` |
| `geocode-success` | geocode | Nominatim `200`, point written | same recorded payload |

Declared, hashed GM-06 divergences (product side only, never compared across
servers): `gm06-closed-failure-vocabulary`, `gm06-geocode-requires-point-index`,
`gm06-no-background-worker`, `gm06-no-fuzzy-fallback`. Any other divergence id
fails.

### GM-07 — ranked lists compared, never sorted

Run order on the fresh database is GM-04, GM-07, GM-02, GM-06, so the search
section sees only its own declared inventory and no earlier message corpus. The
captured surfaces are `search_entities` (vector), `search_messages` (text),
`get_related_entities` plus the symmetric `RELATED_TO` inventory (graph) and the
fused `get_context` document. Ranked lists keep the returned order positionally;
only graph results are sorted, by their token. Declared reference-only
divergence: `gm07-reference-search-unscoped` (the pinned `SEARCH_MESSAGES_BY_EMBEDDING`
has no owner filter, so a foreign-owner message can be returned; the product
search is scoped and must not). The capture records the foreign-owner keys a
scoped search returned and the contract requires exactly the declared value for
each side, so a leak fails on either side.

### Disclosed product gap (blocks the U6 measurement, not the harness)

The shipped `muninn` binary builds no enrichment provider and exposes no
enrichment-specific configuration, so `enrich_entity`/`geocode_entity` answer
`501`; it also exposes no `get_context` bridge method. Observed from source:
`apps/muninn/src/main.rs` has `Neo4jArgs`, `EmbeddingArgs`, `ExtractionArgs` and
no enrichment/geocoding argument, and `crates/muninn/bridge/src` dispatches
`enrich_entity`/`geocode_entity` only for a client that was built with those
services. The harness therefore captures and compares GM-06/GM-07 correctly but
fails closed on the product side until that surface is wired; no tolerance,
exclusion or post-hoc deviation was introduced for it. This is a product gap of
the same class as `H4-U5B-F1`, recorded here instead of being tolerated.

### Known limits of GM-06/GM-07

- **GM-06 provider answers are recorded, not live.** The differential measures
  adapter and writer behaviour against the pinned payloads; it says nothing
  about provider quality or upstream drift.
- **Spacing is validated, not compared.** Wall-clock request gaps differ per
  machine, so each side must satisfy `rate_limit_seconds - clock_slack_seconds`
  (`0.5 - 0.05` for wikimedia, `1.0 - 0.05` for Nominatim) and only the recorded
  request totals are compared with the declared per-side counts.
- **The reference timestamp is not compared.** `enriched_at` is written with
  `datetime()` by the pinned background writer and from the cached value by the
  product; only its presence is captured (declared in `GM06-02`).
- **Ranking ties are not resolvable by the contract.** Both sides receive the
  same controlled vectors, so ties are not expected; if a real run produces a
  tie the positional comparison fails and the finding is reported, never
  smoothed by sorting.
- **GM-06/GM-07 accuracy and MiniLM numerical acceptance remain NOT_RUN.**
  Semantic extraction quality is **UNVERIFIABLE for this H4 closure by human
  disposition**: no approved labeled corpus, scoring formula/baseline, or
  threshold exists. No quality score is claimed. Current live differential,
  Silver38 and Gold cross-memory TCK results are recorded below. Gold does not assert
  `Preference-[:ABOUT]->Entity`; that relation has a separate live test.

## U6 current live differential

The current-source run is retained at
`.agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory/evidence/gm-h4-live-27/`.
Its summary records TCK commit
`b6c72c36bb7486881b87053ee1610f2caf972fb6`, reference commit
`9d45af5529ecb5f2fe352013468a11087f3163d4`, the `muninn-h4-oracle-spacy:20261105`
image, and the current workspace source manifest. GM-02, GM-03, GM-04, GM-06,
and GM-07 each report `pass` with no diffs. The sole numeric epsilon is `1e-9`,
the declaration digest is
`57caed06516c58af2ddcc73ae2e36ffc52b99e68748d42a55b502f67cede2122`, and no
deviations were declared. The receipt confirms container cleanup, source and
snapshot integrity, and removal of temporary runner state.

This is a pass for the declared differential only. The receipt retains
`h4_acceptance: not_established` and `native_live: NOT_RUN`.

## Current-source Silver38 and Gold cross-memory

`product-tck-live-03/summary.json` records two profiles against the product
binary, using a fresh isolated Neo4j fixture and the normative b6c7 TCK tree:

- `silver-long-term`: all **38/38** selected cases passed; selected and observed
  node IDs match, and the report validator returned `valid: true`.
- `gold-cross-memory`: all **18/18** Gold-marked cases in
  `v1/test_cross_memory.py` passed; selected and observed node IDs match, and
  the report validator returned `valid: true`.

The Gold profile is limited to the exact 18 pinned cross-memory cases. It does
not test the `Preference-[:ABOUT]->Entity` edge. The ABOUT live acceptance is a
separate U3 check and is not counted as Gold. These TCK passes do not measure
general extraction precision, recall or F1.

## H4 milestone disposition

On 2026-11-06, the human owner accepted H4 with semantic extraction quality
explicitly classified as **`UNVERIFIABLE`**. No quality score or full feature
parity claim is made. The approval does not change historical runner receipts:
U6's `h4_acceptance: not_established` and `native_live: NOT_RUN` remain as
recorded for that runner and invocation. Separate live ABOUT, Silver38 and Gold
receipts are listed above. This disposition is limited to H4 and does not
accept H5, H6, H7, or the full parity specification.
