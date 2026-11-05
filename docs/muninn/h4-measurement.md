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

## What remains unmeasured

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
deduplication) harness is implemented and deterministic-tested. **No real
measurement has run**: the H4 differential belongs to U6, and every verdict
below is `NOT_RUN` until that unit captures both sides. The harness is built by
composing the accepted components rather than adding a framework: `gm05_build`
for the source-bound offline build, `gm08_runner` for the dory lifecycle,
preflight and faithful-copy helper, `gm08_peer` for the loopback bridge,
binary-consume and fresh-database guards, and `gm08_contract` for the
fail-closed comparison shape. No `gm05` or `gm08` file is modified.

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

## Declared tolerances and rules (disclosed by hash)

The only numeric tolerance is an absolute **`1e-9`** applied to resolution
similarity scores and `SAME_AS` confidence. Every other value — including
`MENTIONS` confidence, band thresholds, spans and labels — is compared exactly.
The declaration digest is
`33f20952b97fdd6ec5f49d788a7e21fc6ec9fc177f6236786b2a805cfb7533f8`
(SHA-256 over `gm_h4_contract.DECLARATIONS`, the sixteen pre-declared rules).
Any observed difference that would need a wider tolerance is a finding to
report, never a relaxation.

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

- **Product extraction stage is wired, default off.** U5 measured the gap that
  `apps/muninn` registered no extraction stage, so GM-02 `auto` produced no
  product mentions. The follow-up unit closed it: the shipped binary now builds
  the spaCy stage from `--extraction-provider` / `NAM_EXTRACTION__PROVIDER`
  (default `none`) through the existing `MemoryClient::with_extractor`, so the
  previous finding no longer holds. A GM-02 run without that flag still
  measures the default-off path, which is the correct comparison for an
  unconfigured deployment.
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
PYTHONDONTWRITEBYTECODE=1 python3 -B dev/muninn/tests/test_gm_h4_runner.py
```

Raw RED and GREEN streams, exits, per-file hashes and the declaration digest are
retained under
`.agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory/evidence/u5b-harness/`.
