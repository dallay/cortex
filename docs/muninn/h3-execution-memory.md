# H3 execution-memory traceability and baseline

**Fresh source-bound Silver29 baseline: 29/29 PASS, with exact ID equality and no skips.**
The single authorized path-only setup correction succeeded; the first setup failure is preserved.
T1 evidence is retained; T2/T3 have scoped deterministic implementation coverage.
H3 is not accepted; final live graph, Silver29 and GM-08 proof remain pending.

## Review path and authority

1. Read the RM mapping and T2/T3 implementation contracts below before reviewing changes.
2. Inspect the [baseline summary][summary] and its exact command/stream artifacts.
3. Review the fresh build/binary identity and exact-owned cleanup before product fixes.
4. Implement and verify GM-08 separately; Silver29 alone cannot establish H3 parity.

The parent-owned tracker is `odd/tasks/muninn-h3-execution-memory.md`, T1, S1–S5.
The human-selected normative document is the main worktree's
`tmp/plans/muninn-feature-parity-spec.md`, §5.3, §10.2 and §10.6.
That file and milestone status are read-only in this unit.

Pinned authorities, in descending precedence:

- Normative TCK: `b6c72c36bb7486881b87053ee1610f2caf972fb6` (`tck-b6c7`).
  Registry: `tck/registry/scenario_ids.yaml:711–985`; tests:
  `tck/tests/v1/test_reasoning.py`; written contract: `SPEC.md:321–360`.
- Implementation oracle: agent-memory
  `9d45af5529ecb5f2fe352013468a11087f3163d4`.
  Paths below prefixed `upstream/` mean `src/neo4j_agent_memory/` at this pin.
- Differential TCK remains separately pinned at
  `e60b410f1046dc21db50f9aba48e171dc8a9b5a1`; it is not the Silver authority.
- Current product inspection: `8554ccbe27f69eacd763323413590c529f23e489`.
  [Summary][summary] retains source hashes and the fresh build-input manifest hash.

H2 remains accepted with its explicitly accepted A4 verifier gap. This unit neither
reopens H2 nor retries/replaces that verifier. H1 pending work, hosted/auth policy,
H6 operations and actual Huginn integration (H7) are not added to H3.

## RM-01–RM-05 mapping

**RM-01 — Traces (MUN-RS-001/004).**

- Existing application: `crates/muninn/application/src/reasoning.rs:162–319`:
  start, append, complete, full retrieval, list and session retrieval.
- Existing bridge: `crates/muninn/bridge/src/reasoning.rs:14–128`:
  lifecycle, full retrieval, session/limit list and similar-task dispatch.
- Persistence: `crates/muninn/neo4j/src/reasoning.rs:237–535` and
  `crates/muninn/neo4j/src/queries/reasoning.rs` implement the real graph operations.
- Silver: S-022–023, S-031–038, S-064 (full official IDs listed below).
- T2 now forwards existing success/date/pagination/order controls through `TraceQuery`.
  Public HTTP mock tests verify forwarding, defaults and rejection before dependencies.
  Live filter/pagination evidence remains a T5 requirement.
  Native `get_trace` compatibility alias is absent; full retrieval exists.
  Similarity and extended lifecycle behavior need GM-08 evidence, not assumptions.

**RM-02 — Structures (MUN-RS-002/003).**

- Existing model: `crates/muninn/core/src/model.rs:204–332` preserves six statuses,
  optional step fields, JSON arguments/results, duration/error and trace completion.
- Application options and writes: `application/src/reasoning.rs:27–102,200–277`.
- Silver: S-024–032, S-057–063; full-chain read also S-033–034.
- T2 adds `TraceOutcome` and `complete_trace_with_outcome` while preserving the native
  `Option<&str>` plus success signature. The bridge maps the pinned schema object's
  fields onto that operation; completion output remains the legacy string/null shape.
- Bridge completion now forwards `generate_step_embeddings`, still false when omitted
  or null. It reuses the existing deferred batch embedding path.

**RM-03 — Cross-tier provenance (MUN-CR-001).**

- Existing application links: `application/src/reasoning.rs:423–470`;
  options expose initiating message, tool message and touched entities.
- Actual graph edges: trace `INITIATED_BY` message; tool call `TRIGGERED_BY` message;
  step `TOUCHED` entity; trace `HAS_STEP` step; step `USES_TOOL` tool call;
  tool call `INSTANCE_OF` tool. Do not substitute diagram-only edge names.
- `neo4j/src/queries/reasoning.rs:10–132,203–267` contains these writes.
- No Silver29 case proves these cross-tier edges. FIX-CROSS-01 and GM-08 are NOT_RUN.
  T2 structured completion must reuse existing touched-edge identity precedence
  (`id` before name+type before name), not create a parallel graph writer.

**RM-04 — Instrumentation and statistics (MUN-RS-003).**

- Stats exist end to end: `application/src/reasoning.rs:322`, bridge
  `reasoning.rs:92`, `neo4j/src/reasoning.rs:469`, `core/src/model.rs:305`.
- Silver: S-039, S-065–067. These cover counts/rates, not recorder/observer behavior.
- T3 adds an explicitly started streaming recorder and construction-time ordered
  tool-call observers with a narrow hook context. Deterministic application tests
  cover these surfaces; live graph and differential proof remain T5 work.
- Source-confirmed T3 contracts: `upstream/memory/reasoning.py:127–383,588–605,715–728`.
  Recorder starts/completes traces; exceptions imply failure; no active step causes
  `call:<tool_name>` auto-step; missing duration uses elapsed step time; observations
  are writable; explicitly set outcome is applied on exit. Preserve embedding switches.
- Hooks run in registration order after call, auto-observation and explicit touched
  writes. Each hook may add touched edges; hook exceptions are logged and suppressed.
  Suppression does not authorize payload logging or swallowing persistence errors.

**RM-05 — Similarity (MUN-RS-004).**

- Existing native trace and step search: `application/src/reasoning.rs:329–385`;
  Cypher: `neo4j/src/queries/reasoning.rs:169–195`.
- Defaults: traces limit 5, steps limit 10, success-only true, threshold 0.7.
  Task embeddings and labeled nonempty step parts feed the configured embedder.
  Missing embedder returns empty results; storage/index failures are propagated.
- Similar traces have bridge dispatch; step search and reasoning context do not.
  These are upstream native capabilities, not permission to invent extra public
  bridge/MCP methods. T2 must establish the chosen compatibility transport contract.
- Silver29 has no similarity cases. GM-08 trace/step ranking and filtering are NOT_RUN.
  Hash1536 functional checks cannot establish semantic retrieval quality.

Product paths abbreviated above are under `crates/muninn/`.
The 29 listed Silver cases are **PASS** for this functional baseline only. Existing surfaces
outside their assertions remain **IMPLEMENTED_UNVERIFIED**; Silver does not prove all RM behavior.
Structured completion and existing bridge controls have deterministic T2 mock coverage;
real graph persistence is **IMPLEMENTED_UNVERIFIED** until T5. Recorder and observers
have T3 deterministic coverage, not a fresh Silver29 or GM-08 parity verdict.

## Exact Silver29 inventory

The [machine-readable inventory][inventory] contains complete pytest nodeids, clauses,
RM ownership and per-case status. Every nodeid below has the exact common prefix
`v1/test_reasoning.py::`; concatenate it with the suffix verbatim.
The registry-to-AST inventory matches observed collection and execution exactly.
Every listed case is **PASS** in the corrected baseline, including setup/call/teardown.
All 29 remain **NOT_RUN** in the preserved first attempt; no history was relabeled.

```text
SCN-S-022 TestStartTrace::test_start_trace_basic
SCN-S-023 TestStartTrace::test_start_trace_unique_ids
SCN-S-024 TestAddStep::test_add_step_basic
SCN-S-025 TestAddStep::test_add_step_sequential_numbering
SCN-S-026 TestAddStep::test_add_step_partial_fields
SCN-S-027 TestRecordToolCall::test_record_tool_call_basic
SCN-S-028 TestRecordToolCall::test_record_tool_call_with_result
SCN-S-029 TestRecordToolCall::test_record_tool_call_failure_status
SCN-S-030 TestRecordToolCall::test_record_tool_call_timeout_status
SCN-S-031 TestCompleteTrace::test_complete_trace_with_outcome
SCN-S-032 TestCompleteTrace::test_complete_trace_failure
SCN-S-033 TestGetTraceWithSteps::test_get_trace_with_steps
SCN-S-034 TestGetTraceWithSteps::test_get_trace_includes_tool_calls
SCN-S-035 TestGetTraceWithSteps::test_get_trace_nonexistent
SCN-S-036 TestListTraces::test_list_traces_all
SCN-S-037 TestListTraces::test_list_traces_by_session
SCN-S-038 TestListTraces::test_list_traces_respects_limit
SCN-S-039 TestGetToolStats::test_get_tool_stats_after_calls
SCN-S-057 TestAddStep::test_add_step_observation_only
SCN-S-058 TestAddStep::test_add_step_all_fields
SCN-S-059 TestAddStep::test_add_many_steps_maintain_numbering
SCN-S-060 TestRecordToolCall::test_record_tool_call_error_status
SCN-S-061 TestRecordToolCall::test_record_tool_call_cancelled_status
SCN-S-062 TestRecordToolCall::test_record_tool_call_pending_status
SCN-S-063 TestRecordToolCall::test_record_multiple_tool_calls_per_step
SCN-S-064 TestListTraces::test_list_traces_empty
SCN-S-065 TestGetToolStats::test_get_tool_stats_multiple_tools
SCN-S-066 TestGetToolStats::test_get_tool_stats_no_calls
SCN-S-067 TestGetToolStats::test_get_tool_stats_success_rate
```

Required measurement invariant: expected = collected = executed **sets**, with 29 unique
IDs and no extras, skips, xfails or omitted phases. Use supported `profile_selector`
with `silver-reasoning`, not the full suite root plus a nodeid selection.
The prepared runner calls unchanged `validate_profile_result`; a failed validation
must remain a failure and does not erase individual case/phase outcomes.

## GM-08 fixed-fixture requirements (implemented; live run pending T5)

The normative fixed inputs are FIX-TRACE-01 (one successful and one failing task;
three ordered steps; tool success, failure and timeout) and FIX-CROSS-01
(conversation mentions an entity; trace triggered by a message; tool references entity).
The approved T4 allocation is three ordered steps **per task** (six total), frozen in
`dev/muninn/fixtures/gm08-v1.json` before any live GM08 measurement.

The later differential must freeze synthetic input and expected output files, seeds,
provider versions, setup/teardown scope and source of each oracle expectation.
Use separate fresh authenticated empty reference and Muninn databases, same installed
Neo4j version and controlled Hash1536 embeddings; make no external model calls.

Compare these independently, retaining raw oracle and product artifacts:

1. Public API results and errors; IDs remain unique, stable and correctly referenced.
2. Persisted trace/step/tool/entity nodes, properties and directed cross-tier edges.
3. Per-tool totals, successes, failures, durations and rates, including missing duration.
4. Similar-trace and step results, success/threshold/limit filters and ranked order.
5. Follow-on API reads and graph state after reconnect, not merely immediate returns.

Only bijective UUID mapping, bounded server-time normalization and representation-only
JSON normalization are allowed. Preserve step and ranked result ordering. Predeclare
float tolerances and do not relax them after observing differences. Retain comparator,
normalization rules, hashes, raw outputs, expected/actual diffs and verdict per item.
The live GM-08 differential is NOT_RUN, not an inferred pass from source inspection,
deterministic tests or historical H2 evidence. See the T4 implementation section below.

## Private-reasoning policy

Optional `thought` and hosted `reasoning` compatibility do not grant permission to
collect provider-private reasoning. Store only explicit application-approved summaries,
actions, observations, outcomes and safe tool content. Never automatically subscribe
to hidden provider reasoning/token streams. Explicit synthetic test values are not
provider reasoning. T3 must pin this boundary with behavior tests; H7 owns real Huginn
event integration. Redaction/auth/hosted policy extensions require their own decisions.

## Consequential contract questions, not new API decisions

- **SOURCE-CONFLICT: get_trace shape.** `upstream/core/protocols.py:571–573` calls it
  header-only; `upstream/memory/reasoning.py` implements an alias to full steps/tools.
  Preserve both source references and obtain disposition before changing an observable
  shape. This inventory does not resolve that conflict by silently choosing a shape.
- **Wire coverage.** T2 adds controls only to existing `complete_trace` and `list_traces`.
  The structured input maps `schema/models.py::TraceOutcome` fields to native completion;
  it does not claim that the pinned TypeScript client's string-only option already
  emits structured objects. Step search, context and linkage dispatch remain unchanged.
  No new endpoint, alias, hosted/auth rule or MCP tool is introduced.
- **Stats graph versus API.** `neo4j/src/queries/reasoning.rs:86–93` documents that
  persisted `failed_calls` counts only error/timeout while reported failed calls are
  total minus successes. GM-08 must compare both, not rewrite one into the other.
- **Touched-created entity IDs.** `neo4j/src/queries/reasoning.rs:236–267` documents
  existing UUID-vs-upstream name-derived IDs. Do not hide this as UUID normalization
  unless the mapping also validates upstream formats and creation semantics.

## Fresh baseline evidence and bounded correction

Evidence root: `.agents/rpi/plan/tasks/muninn-h3-execution-memory/evidence/`.
The [corrected summary][summary] binds all source/helper hashes and per-case results.

### Preserved first attempt

`silver29-baseline/` remains byte-for-byte unchanged. Its preparation exited 0;
orchestration exited 2 because fetch exited 125: dory could not see the system-temp
source bind. Expected 29, collected/executed 0; all cases NOT_RUN. No Neo4j/product was
started. Its scratch was removed and no container/network was created. This remains
setup-failure evidence, not a functional failure or an overwritten result.

### Corrected source-bound run

`silver29-correction/` contains the new attempt. Parent authorization (ODD L7) allowed
only excluded removable in-project state for this Docker bind. `git check-ignore`
succeeded before creation of `evidence/.silver29-state-65ceca64a8da49bf951cd0c22f58031c/`.
That exact directory was removed in `finally` and its absence rechecked.

- `prepare.py`: exit 0. `helper-provenance.json` records old/new helper hashes,
  the path-only correction and first-attempt artifact hashes. Runner bytes are identical;
  no selectors, tests, tolerances, loopback guards or fixture controls changed.
- `orchestrate.py`: exit 0. Lock-only fetch, offline current-source build, export and
  consumed-binary hashes succeeded. No old binary was reused.
- `results/silver29-result.json`: pytest exit 0; stdout: `29 passed in 5.83s`;
  stderr empty. All 29 setup, call and teardown phases passed. Expected = collected =
  executed exact unique ID sets; failed/error/skipped/xfailed/xpassed all zero.
- JSON, JUnit XML, full/selected/execution collections, commands and raw streams are
  retained under `results/`. Per-case official IDs and phase results are in the inventory.
- Authenticated empty synthetic Neo4j was verified before measurement (`count = 0`);
  a wrong password was rejected. Only the task-owned disposable database was cleared.
- Builder/product peer ran as `1000:1000`, Neo4j as `7474:7474`; memory/CPU/PID caps,
  dropped capabilities and no-new-privileges were configured. Builder/private cache used
  task-owned tmpfs/state; shared Cargo registry remained read-only. Fetch downloads were
  checked against Cargo.lock. No dependency lock or toolchain/image installation changed.
- Internal Docker network, zero published ports, real product and pytest in one owned
  Python peer, product listener `127.0.0.1:3001`; loopback guard unchanged. Hash1536 only,
  no external model calls or semantic-quality claim.
- All three owned containers and the owned network were removed by exact ID, with
  absence rechecked. Source and copied snapshot hashes remained unchanged.

Fresh identities:

```text
HEAD: 8554ccbe27f69eacd763323413590c529f23e489
source manifest SHA-256:
5b296c90a1594f939f7c3e2fdba658ea8c48f73b6fbd2cfc87d3192d304da5dd
build = export = consumed executable SHA-256:
3fe842d85cd3303c9dc0863a6e9ff5df8ab15b320949a647e666d88798277806
```

Exact build: `cargo build -p muninn --bin muninn --locked --offline`, Linux aarch64,
Rust/Cargo 1.99.1, dev profile without debug symbols or incremental compilation.
Installed image IDs, toolchain details and Python/package versions are retained in
`runtime-identities.stdout`, `exec-probe.stdout` and `results/python-runtime.json`.

No artificial RED applies: T1 changes documentation/evidence only. `report.py` audits
source integrity, original-artifact preservation, ID/phase equality, JSON/XML results,
build identity, fixture controls, exact cleanup and document structure. Cargo tests,
Clippy/fmt and functional RED/GREEN were not run here; later implementation units own them.
GM-08 remains NOT_RUN. T2/T3 gaps above are not disproved by this green Silver baseline.

**Risk:** only documentation and evidence scripts/artifacts changed; no product behavior
changed. Evidence is gitignored and must be retained deliberately by the parent.
Parent owns tracker transitions, review and explicit human milestone disposition.

## T2 structured completion and existing bridge controls

Native callers retain `complete_trace(id, Option<&str>, Option<bool>, embeddings)`.
Use `complete_trace_with_outcome(id, &TraceOutcome, embeddings)` for structured data.
The new model lives in `muninn_core::model`; it contains required `success` and `summary`,
optional `error_kind`, related `EntityRef` values and numeric metrics. Only explicit,
application-approved content is accepted; there is no provider event subscription.

The existing HTTP `complete_trace` accepts the same fields as an `outcome` object:

```json
{
  "trace_id": "00000000-0000-0000-0000-000000000001",
  "generate_step_embeddings": true,
  "success": true,
  "outcome": {
    "success": false,
    "summary": "Approved task summary",
    "error_kind": "timeout",
    "metrics": {"latency_ms": 125.0},
    "related_entities": [{"name": "Synthetic entity", "type": "OBJECT"}]
  }
}
```

Structured success wins over outer success. String/null outcomes and their success flag
remain supported. `generate_step_embeddings` defaults to false, including explicit null.
All fields are parsed before embedding or store access. Metrics must be finite numbers;
related entities must resolve to a nonempty identity. Invalid fields produce HTTP 400
without store operations or embedding calls. Shared UUID/date/bool parsers are unchanged.
Structured entity parsing reuses the existing id > name+type > name precedence.

Completion preserves the pinned runtime order:

1. Generate requested deferred step embeddings using the existing batch helper.
2. Write summary, success and completion time through the legacy store operation.
3. If error kind is present or metrics are nonempty, write top-level `error_kind` and
   `metrics_json`. Empty metrics serialize to null using the existing Python JSON codec.
   These are not additions to the legacy `metadata` JSON or the HTTP response.
4. Link related entities through the highest-numbered step, using existing touched helpers.
   No steps means no links and no synthetic step. Check missing trace after these operations.

These are separate writes, not a new transaction: errors propagate, later writes stop,
and a structured-property/link error can leave a trace completed. The additive store
method defaults to an explicit unsupported error for older adapters rather than silently
losing requested fields. No existing trait signature or required literal field changes.

The existing `list_traces` accepts `session_id`, `success_only` (null/all, true/successful,
false/failed), RFC 3339 `since`/`until`, nonnegative integer `limit`/`offset`,
`order_by` (`started_at` or `completed_at`) and `order_dir` (`asc` or `desc`). Defaults remain
100, 0, started_at and desc. Null optional controls use their defaults; zero limit is valid.
The existing store query retains inclusive date boundaries. No `get_trace` alias is added.

Evidence: `.agents/rpi/plan/tasks/muninn-h3-execution-memory/evidence/t2-completion/`.
The RED streams retain the original failures, including the missing-trace write-order
regression. A first GREEN assertion incorrectly expected an HTTP metadata field; the
correction preserved the actual legacy response rather than expanding it. Cargo's ignored
live tests are not parity passes. T5 still owns fresh real-graph, Silver29 and GM-08 proof.

## T3 explicit recorder and ordered observers

Use `StreamingTraceRecorder::start(&memory, session, task, options).await` to begin a trace.
`ReasoningMemory::new` and existing callers remain unchanged. The recorder borrows the service;
there is no implicit provider subscription, async Drop, background task or shared registration lock.

```rust
use muninn_application::{RecorderOptions, RecorderStep, StreamingTraceRecorder, ToolCallOptions};

# async fn example(memory: &muninn_application::ReasoningMemory)
#     -> muninn_core::MemoryResult<()> {
let mut recorder = StreamingTraceRecorder::start(
    memory, "synthetic-session", "Inspect synthetic fixture", RecorderOptions::default(),
).await?;
recorder.start_step(RecorderStep {
    summary: Some("Application-approved synthetic summary".into()),
    action: Some("Inspect fixture".into()),
    observation: None,
}).await?;
let call = recorder.record_tool_call("inspect", Default::default(), ToolCallOptions::default()).await;
match call {
    Ok(_) => {
        recorder.set_outcome("Inspection complete", true);
        recorder.finish().await?;
    }
    Err(operation_error) => {
        // Explicit safe summary, not operation_error.to_string().
        // Completion errors propagate too; applications may retain both errors separately.
        recorder.finish_failed("Inspection failed").await?;
        return Err(operation_error);
    }
}
# Ok(())
# }
```

Handle *every* application error path this way if failure completion is required, including
errors returned by `start_step` or `add_observation`. A bare `?`, drop, panic, task cancellation
or process crash does not complete the trace. This is the normal Rust lifetime difference from
Python's awaited async context-manager exit, not a promise of cancellation durability.

### Content, timing and completion

- `RecorderStep` accepts explicit safe `summary`, `action` and `observation`. Summary maps to
  the compatible `thought` property. These names do not validate approval or detect/redact
  private content. Applications own approval for all task/tool/outcome/callback content.
- Steps reuse `add_step` ordering. A call without an active step creates action `call:<tool_name>`;
  later calls reuse that step until `start_step` succeeds. An explicit duration wins; otherwise
  elapsed monotonic step time is recorded in whole milliseconds (saturating at `i64::MAX`).
  Unlike the reference wall clock, monotonic time is not affected by clock adjustments.
- `add_observation` overwrites the current persisted observation; without a step it rejects
  before writing. `set_step_observation` shares the existing store path with auto-observation.
- `RecorderOptions::trace` retains native start options and task embeddings defaulting to true.
  `generate_step_embeddings` defaults to false; true embeds on creation and requests the
  existing deferred batch helper on completion. Updating an already embedded observation
  does not regenerate that embedding, matching the source's pending-only completion behavior.
- `set_outcome` selects legacy summary/success; `set_structured_outcome` validates and selects
  T2 structured completion. Last valid selection wins. Normal `finish` defaults to success
  with no outcome. `finish_failed(safe_summary)` overrides either selection with legacy failure.
  No exception text is automatically persisted or embedded. All completion writes are awaited;
  errors propagate and preserve the existing partial-write order, not a new transaction.

### Observer boundary and ordering

Implement application-owned `ToolCallObserver` and register each with the consuming builder
`memory.on_tool_call_recorded(Arc::new(observer))` before sharing the service. Registration order
is execution order. The callback receives `HookContext::tool_call()` and `step_id()` only;
`add_touched_edge(&EntityRef).await` reuses `record_touched_entity` identity precedence.
The context exposes no store, provider stream or authentication authority.

Callbacks run only after successful call persistence, requested automatic observation and
all explicit touched-entity writes. Any failure in those writes propagates and stops callbacks.
A callback's returned error is suppressed and emits only the static warning
`tool-call observer failed; continuing`; subsequent callbacks still run. As in the source,
this includes a touched-edge write error returned *inside* a callback. Outside callbacks,
storage failures are never suppressed. Callback panics are not caught. Logs include no callback
name, arguments, result, error text or payload. No new synchronization is introduced.

Public integration tests in `application/tests/reasoning_recorder.rs` pin exact persisted and
embedded synthetic summaries, ordering, defaults, statuses, observations, failure completion,
partial writes, ordered callback suppression and payload-free warning output. They also preserve
legacy construction/tool defaults/lifecycle. They do not ingest a provider reasoning channel or
make model calls. T4 can drive these APIs; T5 owns live graph/Silver29/GM-08 validation. H2's
accepted A4 gap is unchanged, and real Huginn event wiring remains H7.

## T4 GM08 implementation candidate — partial, not live acceptance

The new GM08 fixture/capture/comparator and Dory runner are implementation candidates.
Live T5 execution completed after the human authorized a lockfile-fixed crate fetch: the
GM08 paired differential, the post-change Silver29 run and the native execution target all
passed on isolated fixtures. Deterministic coverage remains the regression base. H3
acceptance is still the human's decision; this document records evidence, not acceptance.

The three GM05 preservation failures are **pre-existing at base**, proven by one isolated
attribution run (see [Attribution](#gm05-preservation-attribution)): the same three tests fail
identically at `8554ccbe` and on this branch, with byte-identical `gm05_contract.py` and
`gm05_capture.py`. They are recorded baseline failures, not a regression of this candidate.
H2, its accepted A4 gap and every reference pin remain unchanged.

### Frozen scenario and comparison

`dev/muninn/fixtures/gm08-v1.json` fixes two tasks, three steps each, six explicit tool
calls with success/failure/timeout statuses, arguments/results/errors and durations
10/20/30 ms per task. The success trace uses legacy completion; the failed trace uses
structured timeout outcome, elapsed metric and an existing entity reference. Fixed
synthetic conversation/message/entity UUIDs seed MENTIONS and support INITIATED_BY,
TRIGGERED_BY and TOUCHED; graph captures also retain HAS_STEP/USES_TOOL/INSTANCE_OF.
No extractor, H4 deduplication, external model or provider-private reasoning is involved.

`gm08_capture.py` records all sixteen write results and their request arguments, then
independent trace reads, tool statistics, six list-control variants and four similar-trace
queries. It reconnects both database driver and reference client before follow-on reads.
The product reuses the existing HTTP bridge process lifecycle, without new endpoints.
`gm08_peer.py` retains native reference model fields rather than repairing bridge responses
from graph data. API/graph failed-call counters are never rewritten to match each other.

**SOURCE-CONFLICT retained:** native reference models expose inherited fields and similarity
metadata that the HTTP wire does not expose. Raw captures and strict field diffs retain
these absences; they are not authorized deviations or permission to expand the bridge.
The existing get_trace protocol/runtime disagreement remains unchanged; this scenario uses
get_trace_with_steps only. Seeded entity references avoid name-derived ID normalization.

`gm08_contract.py` accepts only zero numeric tolerance and validates each side independently
before comparing, so an equal-but-corrupt pair still fails. Per-side invariants cover unique
canonical random UUID4 bindings, known field sets, the two-task/three-step/one-call-per-step
fixture shape, declared statuses and durations, API↔graph trace/step/call identity, HAS_STEP
order against `step_number`, one parent per step or call, USES_TOOL and INSTANCE_OF targets,
INITIATED_BY/TRIGGERED_BY/TOUCHED cardinality, edge endpoint labels, orphan and duplicate
nodes/edges, generated capture-window bounds, supplied message timestamps, sequential and
ranked API ordering, and relative graph times (trace start → step order → call timestamp →
tool `last_used_at`, with tool `created_at` not after use). Tool statistics are checked against
recorded statuses and durations: API `failed_calls` must equal total minus successes, while the
persisted graph `failed_calls` counter must count only error/timeout. The reconnect document is
validated with the same invariants. Only graph collections are sorted. Missing and null differ;
metrics_json whitespace/object key order may normalize.
The comparison CLI is read-only and emits field paths, expected/actual values and item verdicts:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -B dev/muninn/gm08_contract.py \
  --expected /path/to/reference.json --actual /path/to/product.json
```

Exit 0 means this offline comparison passed; exit 2 means invalid input or differences.
This CLI cannot certify source provenance or substitute frozen bytes for a live oracle run.

### Prepared T5 command and prerequisites

After the remaining T4 checks below and parent-owned independent verification, choose a new
output directory under the exact task-owned evidence surface:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -B dev/muninn/gm08_runner.py \
  --sources dev/muninn/.cache/sources \
  --output .agents/rpi/plan/tasks/muninn-h3-execution-memory/evidence/t4-gm08/t5-live-01
```

Do not execute this command as a T4 documentation check. It starts real ephemeral resources.
It requires dory, the already-installed pinned Rust/Neo4j images, the installed
`muninn-gm05-oracle:20261104` image, read-only `muninn-cargo-registry` volume containing
all locked Linux dependencies, the unchanged H2 faithful-copy helper and verified reference
checkouts at 9d45af55/e60b410. Silver b6c7 remains separate and unchanged. No pull, fetch,
installation, external binary or archived expected-capture option is implemented.

The runner composes H2 faithful copying, GM05 source snapshotting, reference integrity and
materialization, and the existing bridge lifecycle. It builds current dirty/untracked Rust
inputs with `cargo build -p muninn --bin muninn --locked --offline` in an owned Linux builder.
Build/export/consumed binary hashes must agree. Source, helper, fixture, image, commands,
streams/exits, capture/diff and artifact hashes belong in the run's summary.json.
The fixture resource is separately harness-hashed; the build snapshot includes the native
Rust target but the product build does not compile or execute that target.

The unique exclusion-checked `.gm08-state-*` subtree inside that new evidence directory is the
only in-worktree transient bind state. General faithful-copy scratch is system temporary.
Two separate authenticated empty tmpfs databases run on an internal network, with no published
ports, nonroot users, resource caps and dropped capabilities. Credentials are synthetic, passed
through environment names and excluded from command artifacts. Exact owned IDs are cleaned,
absence checked and original/copied source hashes checked before final summary publication.
The lifecycle journal is covered by deterministic simulated tests (no Docker): removal and
absence re-verification apply only to identities this invocation created, cleanup is idempotent,
a failed stage keeps the journal intact for the `finally` cleanup, a non-hex identifier is never
adopted, and a credential echoed by a command aborts before any stream artifact is published.
Peer guards reject unknown capture sides, a consumed binary hash mismatch (removing the copy),
and a non-empty starting database. These simulate the production paths; the lifecycle has
**not yet been exercised live**.

The ignored native target exercises current recorder/observer/structured-completion APIs,
ordered persisted steps/tools, graph edges, counters, controlled 3D step ranking/filters and
new-driver reconnect. Three fixed entities make each TOUCHED writer identifiable by its target:
step 3 of the failed trace reaches the outcome entity only through structured completion, the
observer entity only through ordered callbacks, and the fixture entity only through the explicit
`touched_entities` list. Its counts are therefore attributions, not a merged total.
It intentionally has no runtime-skip-to-PASS path. It must receive a
separate **new authenticated empty synthetic** fixture, not either measured GM08 database:

```sh
# Set MUNINN_FIXTURE_URI and MUNINN_FIXTURE_PASSWORD privately for the owned fixture.
MUNINN_GM08_FRESH_SYNTHETIC=1 cargo test -p muninn-neo4j \
  --test reasoning_execution --locked -- --ignored --exact \
  recorder_outcomes_provenance_and_step_similarity_survive_reconnect
```

This native test uses explicit vectors, not Hash1536 or semantic-quality claims. Its caller
owns fixture teardown. Compile success and Cargo's ignored count mean native live **NOT_RUN**.
The sole added dependency is a local application dev-dependency; no external pin changed.

### T5 live status

**All three live checks passed** on 2026-11-04 with the pinned images, dory, and one
human-authorized lockfile-only crate fetch. No pin, `Cargo.lock`, oracle, contract, fixture,
normalization, tolerance or shared configuration changed.

#### Authorized offline build prerequisite

`evidence/t5-live/fetch-and-run.py` runs `cargo fetch --locked` with a private `CARGO_HOME`
inside one exclusion-checked `.gm08-cargo-*` directory (checked with `git check-ignore`
before creation, removed afterwards). Result: 700 crate archives, fetch exit 0,
`Cargo.lock` SHA-256 `261c85eb…` before and after (identical), host registry unchanged at 629
archives, shared `muninn-cargo-registry` volume mounted read-only in the builder. The offline
build then ran entirely with `--locked --offline`.

#### GM08 paired differential: PASS

`evidence/t5-live/gm08-live-07/` (authoritative; `gm08-live-01…06` retain earlier
fail-closed attempts and harness fixes):

- Two separate, freshly authenticated, empty, synthetic databases on one internal network
  with no published ports; both reported Neo4j `5.26.32` community and the same image digest
  `sha256:414adfe4…`.
- Reference capture ran the pinned `9d45af55` package inside the installed oracle image;
  the product capture ran the current binary's HTTP bridge. Reference native results are
  projected onto the exact `bridge/src/wire.rs` shape (`gm08_peer.project_wire`), because
  native-only fields are not part of the measured public surface. The measured graph is the
  execution-memory label allowlist, which excludes schema/ontology bookkeeping symmetrically.
- Controlled embeddings: reference `tck MockEmbedder (SHA256, 1536)`, product `hash1536`,
  no external model call.
- Verdict: `pass`, **0 field diffs**, `api`/`graph`/`reconnect` all `pass`, zero numeric
  tolerance, no authorized deviation. Both sides persisted 18 execution-memory nodes and 34
  directed edges; both returned 2 traces, identical list control results
  (ordered 2, completed 2, successful 1, failed 1, page 1, supplied_dates 2), similar-trace
  results (all 2, successful 1, limit 1, threshold 1.1 → 0), and identical tool statistics
  (6 calls, 2 successes, 4 reported failures, rate 1/3, average 20 ms).
- Raw `reference.json`/`product.json`, per-field diffs, per-item verdicts and 216 artifact
  hashes are retained; cleanup removed all 6 owned identities (builder, two databases, two
  peers, network) with absence checks, and workspace/snapshot hashes were unchanged.

#### Post-change Silver29: 29/29 PASS

`evidence/t5-live/silver29-02/` builds the same 744-file source inventory
(`source-manifest.json` SHA-256 `b591c502…`) to binary
`8b56846a3e718565bcb45f0c791663ea834ff4157a460ad8fe92310a82d8c35f`, identical to the GM08
build and to the consumed `/target/muninn`. The unchanged T1 in-container runner executed the
`silver-reasoning` profile on a fresh authenticated empty Neo4j: expected = collected =
executed = 29 exact IDs, `29 passed`, exit 0, every setup/call/teardown `passed`, zero skips,
zero xfail/xpass, `valid: true`.

#### Native execution target: PASS

Two live runs (initial and final state) on separate fresh authenticated empty fixtures:
`1 passed; 0 failed; 0 ignored` in ~5 s each, with structured completed/failed outcomes,
ordered steps and tool counters, step-similarity ranking/filters, reconnect reads, and
TOUCHED edges attributed to their distinct writers (explicit `touched_entities`, ordered
observer callbacks, structured-outcome link). Dory reports the port as `0.0.0.0` on the VM
side while the real macOS listener is loopback-only (`lsof`), consistent with the documented
dory behaviour.

#### Limits

The GM08 differential measures the exposed bridge surface plus the persisted graph, so
library-only step search/context remains covered by the native target instead of a new public
endpoint. It is one synthetic scenario, not semantic retrieval quality. Ignored integration
targets in the package suite remain NOT_RUN and were never counted as passes.

### GM05 preservation attribution

`evidence/t4-gm08/gm05-attribution.py` archives base `8554ccbe` and copies the current
`dev/muninn` into runtime-owned `mktemp` directories outside the worktree, then runs only the
three named preservation tests on each side and records raw stdout/stderr/exit plus module
hashes. Result: `pre_existing_at_base`. All three tests fail identically at base and on this
branch, and `gm05_contract.py`/`gm05_capture.py` are byte-identical, so they are known
baseline failures. The script is not a test, is not committed to CI, and no GM05 file was
edited. The earlier `gm05-preserve-c1` run reproduces 97 passes and the same three failures.

### Retained RED evidence

The correction batch's new rules were observed failing before implementation, and the raw
streams are retained as `red-c1-failclosed.stdout/stderr` (19 failures plus 7 errors: missing
equal-but-corrupt invariants and missing lifecycle/peer injection). Two test-authoring defects
were fixed after that RED run and are disclosed rather than hidden: the dangling-call case
dropped the wrong USES_TOOL endpoint, and the simulated absence probe reported a present
container. No implementation was written before its test existed.

### Self-review and remaining proof

- **S1:** live GM08 differential (API, graph, statistics, similarity, reconnect), the
  post-change Silver29 run and the native execution target have all passed on isolated
  synthetic fixtures. Deterministic tests remain the regression base; no scenario beyond this
  one fixed pair is claimed.
- **S2:** GM05 files, its contract, deviations, orchestration, accepted H2 evidence and every
  reference pin were not modified. The three GM05 failures are pre-existing at base.
- **S3:** zero tolerance stays frozen; no tolerance, deviation or deviation list was added, and
  no field is dropped. No measurement or milestone acceptance was claimed.
- **S4:** the only live fixture was the native target's own database; no provider or external
  model ran. Every stream and exit is retained under `evidence/t4-gm08/` and
  `evidence/t5-live/`. Live cleanup and absence were observed for the native fixture and for
  every GM08 attempt, including the exclusion-checked `.gm08-state-*` bind.
- **S5:** only this document and the developer README changed; the parent owns tracker, status,
  independent verification and human acceptance.

**Risk: item5 — local test dependency change.** The parent escalated Task Size item5 and
requires an independent verifier for this candidate; that is not the accepted H2 A4 verifier
gap. The authored unit exceeds the 400-line advisory because capture, strict comparator,
isolated lifecycle, tests and native assertions form one GM08 transaction; no code was minified
and no test was omitted to meet the advisory.

[summary]: ../../.agents/rpi/plan/tasks/muninn-h3-execution-memory/evidence/silver29-correction/summary.json
[inventory]: ../../.agents/rpi/plan/tasks/muninn-h3-execution-memory/evidence/silver29-correction/scenario-inventory.json
