# Opt-in message extraction

Muninn can run a local spaCy `en_core_web_sm-3.8.0` stage through an HTTP
`EntityExtractor` adapter. Extraction is opt-in: configuring the adapter alone
never changes existing message ingestion. This is H4 U2 implementation evidence,
not Silver38, Gold, GM, global parity or milestone acceptance.

## Offline setup and lifecycle

**The cached model wheel does not establish an installed spaCy runtime.**
A compatible Python interpreter, pip/venv, spaCy `>=3.8.0,<3.9.0` and all of its
platform-compatible transitive dependencies are prerequisites. Obtain reviewed
runtime wheels separately, outside this task, and place the complete wheelhouse
at `/tmp/muninn-spacy-wheelhouse`. Missing prerequisites must stop setup; never
fall back to PyPI, `spacy download`, another model or an empty extraction result.
The commands below are operator instructions, **NOT_RUN** in U2.

From the repository root, use an isolated runtime outside the worktree:

```sh
python3 -m venv /tmp/muninn-spacy-runtime
/tmp/muninn-spacy-runtime/bin/python -m pip install --no-index \
  --find-links /tmp/muninn-spacy-wheelhouse 'spacy>=3.8.0,<3.9.0'
/tmp/muninn-spacy-runtime/bin/python -m pip install --no-index --no-deps \
  dev/muninn/.cache/model-evidence/en_core_web_sm-3.8.0-py3-none-any.whl
/tmp/muninn-spacy-runtime/bin/python -m pip check
MODEL_ROOT=$(/tmp/muninn-spacy-runtime/bin/python -B -c \
  'import sysconfig; print(sysconfig.get_path("purelib"))')
PYTHONDONTWRITEBYTECODE=1 /tmp/muninn-spacy-runtime/bin/python -B \
  dev/muninn/spacy_service.py \
  --model "$MODEL_ROOT/en_core_web_sm/en_core_web_sm-3.8.0" --port 8765
```

The exact installation input is the already-cached wheel above. The installed
model directory is `en_core_web_sm/en_core_web_sm-3.8.0` under that isolated
interpreter's `purelib`; it is not the `.whl` file itself. Do not install into or
import the pinned reference tree. U2 neither installs dependencies nor executes
the model. Its runtime availability and model accuracy remain **NOT_RUN**.

The stdlib server loads that trusted local model once at startup and binds only
`127.0.0.1:8765`. It runs in the foreground; Ctrl-C stops it. The operator owns
its isolated runtime and process lifecycle. It needs no graph credentials and
must not be exposed as a public service. The service has no authentication model;
this does not add authentication or authorization to Muninn.

## Rust configuration

The composition root registers the adapter explicitly; no environment variable,
bridge endpoint or model-download hook is added:

```rust,ignore
use std::{sync::Arc, time::Duration};
use muninn_providers::SpacyEntityExtractor;

let stage = SpacyEntityExtractor::new(
    "http://127.0.0.1:8765/extract",
    Duration::from_secs(10),
)?;
let memory = MemoryClient::new(ports, &options).with_extractor(Arc::new(stage));
```

Use `MemoryClient::short_term().add_message` or HTTP `POST /add_message` with
explicit extraction controls. The direct legacy `ShortTermMemory::new`
constructor has no long-term port; extraction-enabled calls on that object fail
with `not supported: message extraction wiring`. Unconfigured calls still work.

## Ingestion controls

```json
{"session_id":"synthetic","role":"user","content":"Alice in Paris",
 "extraction_mode":"auto","extract_entities":true,"extract_relations":false,
 "generate_embedding":false,"user_identifier":"owner"}
```

- `auto`: run the configured stage when `extract_entities` is true. Supplying
  `auto` defaults that switch to true; false disables it. Without a configured
  extractor, auto produces no entity links, like the pinned implementation.
- `skip`: never run extraction or create MENTIONS edges, even with references.
- `explicit`: never run extraction; link only `explicit_mentions`. The
  `extract_entities` switch has no effect in this mode.
- No mode and no `extract_entities` override: preserve today's no-extraction
  behavior, even with a configured adapter. `extract_entities:true` alone opts
  into auto. Supplying references alone does not opt into explicit linking.
- `extract_relations` defaults to true and is forwarded to the extraction port;
  false suppresses relationship persistence. The selected spaCy stage returns no
  relations or preferences. This is not an LLM fallback pipeline or ABOUT work.
- `generate_embedding` defaults to true. False disables both message and typed
  entity embedding calls. Entity resolution otherwise uses the existing U1 policy.

Null mode/entity switches behave as absent. Boolean switches reject non-boolean
values. Invalid mode strings return HTTP 400 with
`'extraction_mode' must be auto, skip or explicit`. Other malformed controls are
rejected before dependencies. The reused reference parser retains its existing
`'touched_entities' must be an array` diagnostic for a non-array reference list;
its behavior was not changed to rename that legacy diagnostic.

The application preserves `owner_scope` exactly. Explicit entity references are
not an authorization mechanism: entities remain global under the existing model.
No new permission rule or cross-owner conversation traversal is introduced.

## Explicit identity and ambiguity

```json
{"session_id":"synthetic","role":"tool","content":"Approved entity results",
 "extraction_mode":"explicit","explicit_mentions":[
   {"id":"00000000-0000-0000-0000-000000000001"},
   {"name":"Alice","type":"PERSON"},
   {"name":"Existing alias"}
 ]}
```

The existing bridge reference parser chooses **id > name+type > name**. Its only
change is visibility within the private reasoning module, allowing reuse.

An ID links that existing node or nothing. A malformed or missing ID never
falls back to a supplied name/type. A typed reference uses the same prepared
`add_entity` policy/upsert as extracted entities, including U1 resolution defaults.
A name alone never creates a node and never uses fuzzy/vector evidence: only one
exact active surface match may link. Unknown and ambiguous names link nothing.
The bounded candidate port must order exact matches before other candidates;
two exact matches suffice to establish ambiguity. U2 reuses that port and
`matches_surface`, not the legacy lookup that returns an arbitrary first row.

Pinned `short_term.py:1005–1093` uses `FIND_ENTITY_IDS_BY_SURFACE_FORM` and links
only when there is exactly one result. Muninn reproduces that ambiguity rule,
with a disclosed normalization limitation: U1's helper trims and compares ASCII
case-insensitively, whereas the reference lowercases Unicode and replaces `|`
with spaces in its surface keys. U2 does not broaden shared U1 normalization.
These Unicode/pipe equivalences are **not claimed as parity**; no fuzzy fallback
is used to hide a missing match.

## Evidence, errors and replay

The one Neo4j MENTIONS writer uses `MERGE` and `ON CREATE SET`. New ingestion
links carry confidence, optional character offsets, `derived`, extractor name,
`support=1`, the single source message UUID and the supplied message content as
`evidence`. Explicit links have `derived=false`; direct ID/name links have no
extractor or span. Auto spaCy links have `derived=true` and `extractor="spacy"`.
Repeated links do not increment support or overwrite the first evidence. The
old confidence-only API delegates to the same writer with default provenance;
legacy adapters retain that API and explicitly reject the new provenance entry
point rather than silently dropping evidence.

Extraction and all required entity/message inference finish before opt-in
conversation creation or graph mutation. A provider failure or invalid extracted
entity leaves nodes, edges, stored counters and vectors unchanged. Validation
rejects unknown POLE+O types, non-finite/out-of-range confidence and malformed
spans. spaCy offsets are Unicode character positions, not UTF-8 byte offsets;
the stripped span must equal its reported name. An invalid item rejects the
whole response, including earlier valid entities.

The HTTP adapter rejects credential-bearing URLs and redirects, uses the
configured nonzero timeout, and bounds responses to 1 MiB before JSON decoding.
Timeout, connection, status and decode failures are explicit `MemoryError::Provider`
errors (HTTP 500 through the bridge), never silent empty extraction. Error
messages contain no request/response payload or credential-bearing URL. The
sidecar bounds request bodies to 1 MiB, returns static JSON errors (400 for invalid
input, 404 for an unknown endpoint, 500 for extraction failure), and does not log
request paths, headers or content. Only explicitly supplied message content is
read; provider-private reasoning is never captured or derived.

`operation_key` replays use the persisted message without inference or duplicate
links. Opt-in payloads include extraction controls, so changing the mode or
references conflicts rather than reusing stale links. Unconfigured payloads
remain byte-identical to the previous schema, including embedding and metadata
controls. These are sequential store calls, **not** a new graph transaction:
storage errors, races or concurrent deletion after inference can leave earlier
writes persisted. Replaying a message after a partial storage failure is not a
link-repair protocol. U2 does not claim that guarantee.

## Reference decisions and proof limits

Authority: agent-memory `9d45af5529ecb5f2fe352013468a11087f3163d4`,
`memory/short_term.py:894–1110`, `config/settings.py:293–420`, and
`extraction/spacy_extractor.py`. Documented differences are:

1. **Compatibility default:** pinned `add_message` defaults to auto/true; Muninn
   stays no-extraction unless explicitly requested, preserving existing clients.
2. **Failure ordering:** pinned ingestion writes the message before extraction;
   Muninn preflights providers first to satisfy the no-partial-provider-write rule.
3. **Chosen stage/floor:** the spaCy stage assigns constant **0.85**, not model
   scores. U2 uses the selected **0.85 inclusive filter**; upstream's general
   multi-stage pipeline defaults to 0.5 and enables other stages. U2 adds none.
4. **Name normalization limit:** ambiguity is enforced, but Unicode/pipe
   normalization parity and the remainder of U1 ontology resolution stay unclaimed.
5. **Additive evidence:** legacy confidence-only calls now initialize default
   provenance on new edges through the same writer. Existing edge properties and
   first evidence are not rewritten. New evidence columns are an explicit Muninn
   addition, not upstream serialization parity.

The documentation and Muninn source-file SHA-256 hashes (upstream reference files are pinned by revision instead), raw RED/GREEN streams,
command exits, and base/current differential are retained under
`.agents/rpi/plan/tasks/muninn-h4-extraction-connected-memory/evidence/u2-extraction/`.
`summary.json` hashes this document as the deviation disclosure; no statistical
quality tolerance was introduced. The differential normalizes only generated
UUID/timestamp values in API observations, not semantic results or error text.

Deterministic tests use frozen stages, wiremock loopback responses and synthetic
vectors. They are not spaCy accuracy measurements. The ignored
`neo4j/tests/extraction_mentions.rs` target requires a NEW authenticated empty
synthetic fixture with caller-owned teardown; ignored means **NOT_RUN**.
Silver38, live Neo4j and live spaCy-model execution are **NOT_RUN** here and belong
to U6 integrated evidence. U3 ABOUT, U4 enrichment and U5 harness work remain
outside U2. H3 stays accepted/committed; H2 A4 is neither retried nor replaced.
Independent item2 verification and human milestone acceptance remain parent-owned.
