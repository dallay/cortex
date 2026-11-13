# Apply Progress: Muninn Memory Evaluation and Benchmarking

## Status

**Connector implementation complete; synthetic paired E2E executed with a failed memory-available condition.** Async Muninn MCP handling, fixed-owner messages/preferences composition, app-boundary scope tests, and optional-service isolation tests are implemented. MCP startup now connects to Neo4j without applying schema setup or migrations; it performs only read-only vector-index dimension validation. The Huginn runtime approval flow is covered by focused tests and was exercised in the local E2E. The one synthetic comparison found successful message recall but no preference recall for the model-selected category. See the final section for the run report and evidence. This does not establish general memory quality or Agent Memory parity. No push or PR is authorized or intended. The >90% Agent Memory parity condition is a future PR gate; its measure is not defined and must not be invented.

## Phase 1 Evidence (Reconciled)

### Protocol compatibility

- `crates/huginn/runtime/Cargo.toml` pins `rmcp = 3.8.0`.
- Local installed `rmcp-3.8.0` source at
  `~/.cargo/registry/src/index.crates.io-*/rmcp-3.8.0/src/service/client.rs` implements
  `server/discover` and per-request protocol metadata (client source lines 707 onward).
- `crates/muninn/mcp/src/lib.rs` defines `PROTOCOL_VERSION = "2026-07-28"`, validates
  `server/discover`, `tools/list`, and `tools/call` request metadata, and advertises
  `2026-07-28`.
- Huginn's pinned `rmcp` 3.8.0 and Muninn MCP source both identify revision `2026-07-28`.
  The observed discovery/tool metadata contracts align. This establishes source compatibility,
  not runtime interoperability.

### Async execution blocker

- `crates/muninn/mcp/src/lib.rs` defines synchronous
  `ToolHandler::call(&Value) -> Result<Value, String>` and synchronous `Server::handle`.
- `muninn_mcp::serve` uses blocking `std::io::BufRead`/`Write` and processes requests
  sequentially. The README explicitly states that it has no async execution or cancellation.
- Muninn's memory ports are async (`crates/muninn/core/src/ports.rs`), and application services
  therefore cannot be called safely by the current synchronous handler without a blocking bridge.
  No such bridge or server-side runtime strategy exists in the inspected MCP boundary.
- The revised design selects an async handler/dispatch/stdio loop, sequential requests, bounded
  frames/results, and an operation timeout. `muninn-mcp` has no runtime dependency today, but
  workspace Tokio is available to compose async stdio I/O and timeout. No `block_on`, nested
  runtime, worker bridge, or unbounded concurrency is required by the selected approach.
- `serve` has no internal call sites outside the MCP binary and MCP tests in the inspected crate;
  converting its public signature to async is locally contained.

### Trusted scope and Huginn approval

- Huginn's `crates/huginn/runtime/src/mcp.rs` requests explicit approval before launching each
  configured server and supplies a separate `ApprovalRequest` for every discovered MCP tool call.
  Cancellation and timeout paths are present. This matches ADR-0006/ADR-0007's distinct startup
  and invocation approvals.
- The Muninn MCP server currently has no application handlers or memory-service composition:
  `crates/muninn/mcp/src/main.rs` serves an empty `Server::default()`, and
  `crates/muninn/mcp/README.md` documents that `tools/list` is empty and no Neo4j connection is
  wired.
- The existing `OwnerScope` type validates an owner namespace for short-term records. The long-term
  application exposes scoped preference search, but entity search and `get_context_scoped` include
  globally searched entities; long-term facts and reasoning also lack an equivalent owner filter in
  the inspected retrieval APIs. The existing `memory_search` catalog contract advertises all
  memory types and has no fixed-owner binding. Therefore it cannot be connected as-is without
  risking cross-owner disclosure.
- `apps/muninn` is the existing composition root. It creates Neo4j stores and a `MemoryClient`, but
  only for its `bridge` command; that path runs schema setup/validation and exposes a destructive
  clear capability only under an explicit option. `muninn-mcp` currently has no storage, provider,
  or configuration dependencies. Building an MCP composition in the app could reuse those
  adapters, but the scope-safe retrieval surface and whether to add a separate app command or wire
  the standalone binary are not established by current contracts.
- Huginn serializes MCP response data as bounded JSON text (`bounded_text`) and returns it as tool
  output. ADR-0006 states MCP output is untrusted. This establishes no authorization and cannot
  replace tool-level output sanitization.

## Previous Stop and Resolution

Phase 1 tasks 1.1–1.3 are complete and marked in `tasks.md`. The revised design resolves the prior
async-strategy concern: async conversion is locally contained, sequential, and compatible with the
workspace runtime. The user resolved the retrieval-contract question by approving messages and
preferences only, both owner-scoped, composed through an MCP command in `apps/muninn`. The approved
worktree is `/Users/acosta/.local/share/opencode/worktree/a863a9/muninn-memory-evaluation-benchmarking-impl`,
on `feature/muninn-memory-evaluation-benchmarking` at `09bb21d5`; the original H5 worktree remains
untouched. The earlier implementation block is therefore cleared for connector work. The previous
concern remains valid: `MemoryClient::get_context_scoped` includes globally searched entities, and
`memory_search` promises broader retrieval. Neither path may be used as-is for the approved tool.

Resolved decision: expose read-only messages and preferences only through an MCP command in
`apps/muninn`; apply the same immutable host-configured owner scope to both paths. Exclude entities
and reasoning traces. Do not use standalone `muninn-mcp` as the composition root, direct bridge/API
fallback, or write/update/delete operations.

No evaluation, inference, database reset, credential inspection, external/cloud access, download,
or production access was performed. The standalone MCP server executable was changed to use the
async stdio API; no Huginn product contract was changed.

## Commands and Checks

- RED: `cargo test -p muninn-mcp --test async_dispatch` failed because handler/dispatch were
  synchronous and async dependencies were absent.
- GREEN: `cargo test -p muninn-mcp --test async_dispatch` passed (1 passed, 0 failed) after async
  handler/dispatch conversion.
- `cargo test -p muninn-mcp` initially failed after the API conversion because the existing test
  call sites remained synchronous. After migrating all test handlers and calls, the command passed:
  16 tests passed, 0 failed (plus 0 unit/doc tests).
- `cargo test -p muninn` passed: 19 tests passed, 0 failed. A prior run of the targeted owner-scope
  test failed at compile time because the test preceded the `HostOwnerScope` implementation; after
  implementation, the full app test target passed.
- Added `Catalog::ScopedRetrieval` with exactly two tools and no scope selector in either schema.
  Its test first failed because the catalog variant was absent; after implementation, it passed.
- Async handler/dispatch test RED: `cargo test -p muninn-mcp --test async_dispatch` failed against
  the previous synchronous handler/dispatch API. GREEN passed after async conversion. Note: the
  result-size test was added after the 1 MiB bound already existed, so it is characterization, not
  RED evidence.
- An oversized-frame test initially failed because Tokio `BufReader` may read ahead by its fixed
  8 KiB buffer. It now asserts the bounded maximum plus that fixed buffer allowance; the subsequent
  complete MCP package suite passed.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy -p muninn-mcp --all-targets -- -D warnings`: passed.
- `cargo clippy -p muninn --bin muninn -- -D warnings`: passed. An earlier app `--all-targets`
  invocation failed on one pre-existing `bool_assert_comparison` warning at
  `apps/muninn/src/main.rs:881`; that unrelated line was not changed.
- `git diff --check`: passed. `rg` found no `block_on`, `block_in_place`, or worker bridge in the
  Muninn MCP source or the new MCP composition module. Existing `std::thread::spawn` calls elsewhere
  in `apps/muninn/src/main.rs` belong to pre-existing commands and are outside this MCP path.
- Implemented app composition validates a host-supplied owner scope before connecting to Neo4j,
  rejects request keys outside the declared query/limit/category surface (including
  `user_identifier`), injects the fixed owner into both owner-scoped application retrieval APIs,
  and bounds each query to 20 results. `user_identifier` remains a selector, not authentication.
- App-boundary fake-retriever tests now prove both handlers pass only the immutable host scope,
  return only the value for that scope, and reject `user_identifier` without dispatching either
  retrieval. Separate tests reject missing/blank host scope and assert the catalog contains exactly
  the messages and preferences tools. These tests characterize behavior that was already wired;
  the first test invocation failed to compile because the injected `ScopedRetriever` seam and
  `test_server` helper had not yet been added, then passed after adding that seam. This compile
  failure is not claimed as a behavioral RED failure.
- Timeout coverage uses Tokio's paused clock and verifies a pending handler becomes a sanitized
  tool error after the configured 30-second deadline. This is characterization of timeout logic
  added in the previous apply attempt, not a new RED/GREEN cycle. Added stdio coverage verifies a
  serialized response whose envelope exceeds the frame limit fails before writing output.
- MCP startup avoids extraction, enrichment, and geocoding setup. A test passes deliberately
  invalid optional bridge configurations with the MCP command and verifies `optional_stages` returns
  no optional services. Owner validation occurs before embedding or Neo4j connection in `main`.
- Earlier Huginn integration assessment (reconciled): `McpClients::connect` in
  `crates/huginn/runtime/src/mcp.rs` launches an external child process, and the production
  `apps/muninn mcp` command connects to Neo4j before serving. That makes the production command
  unsuitable for this no-database test. It does **not** make all protocol interoperability tests
  database-dependent: the app's `test_server()` seam and generic async `muninn_mcp::serve` allow
  in-process verification with a fake retriever. The current rmcp client test now provides that
  evidence. It does not exercise Huginn's `McpClients`, startup approval, or per-invocation
  `HostPolicy` approval, so the explicitly scoped Huginn runtime task remains open.
- Current-turn command evidence:
  - RED: `cargo test -p muninn --bin muninn mcp_command::tests::both_retrieval_handlers_use_only_the_fixed_host_owner` failed to compile because the planned test seam (`ScopedRetriever`, `test_server`) was not implemented yet.
  - GREEN: same filtered command passed after adding the seam (4 MCP composition tests passed).
  - RED: `cargo test -p muninn --bin muninn mcp_command_does_not_initialize_optional_bridge_services` failed to compile because `optional_stages` was not implemented yet.
  - GREEN: same filtered command passed after adding command-sensitive optional-stage setup.
  - Characterization: `cargo test -p muninn-mcp --test async_dispatch dispatch_times_out` passed (1 test); `cargo test -p muninn-mcp --test stdio response_envelope_over_limit` passed (1 test).
  - `cargo test -p muninn-mcp`: passed, 18 tests, 0 failed; no unit/doc tests.
  - `cargo test -p muninn`: passed, 22 tests, 0 failed.
  - `cargo test -p muninn-application owner`: passed, 6 tests, 0 failed; `cargo test -p muninn-application scoped`: passed, 1 test, 0 failed; `cargo test -p muninn-application long_term::tests::preference_scope_separates_same_category_and_vectors_and_preserves_global_reads`: passed, 1 test, 0 failed.
  - One malformed invocation, `cargo test -p muninn-application short_term::tests::scoped_message_search_rejects_foreign_owner long_term::tests::scoped_preference_search`, failed because Cargo accepts only one test-name filter. The valid filter invocations above then passed.
  - `cargo fmt --all -- --check`, `cargo clippy -p muninn-mcp --all-targets -- -D warnings`, `cargo clippy -p muninn --bin muninn -- -D warnings`, and `git diff --check`: passed.
- No Huginn runtime source or core/application contract changed. The actual protocol-integration gate remains open; do not claim runtime compatibility. Phase 2.4–2.6 and Phase 3.1 are marked complete. Phase 3.3–3.5 remain incomplete.
- No E2E inference, local model readiness inspection, evaluation service startup, credentials,
  external/cloud resources, downloads, persistent data, or production resources were accessed.

## Final Protocol Verification Attempt

- Added an exact `rmcp = "=3.8.0"` dev-dependency to `apps/muninn` with `client` and
  `transport-async-rw` features. The test uses rmcp's default `.serve(stream)` lifecycle, a
  `tokio::io::duplex` pair, the app's fake-retriever `test_server()`, and the real generic async
  `muninn_mcp::serve` path. It does not launch an executable or connect to Neo4j.
- TDD evidence: the new protocol test first failed during rmcp initialization with
  `Invalid request metadata`; the server did not yet implement the legacy `initialize` request
  required by rmcp 3.8.0's default `.serve()` lifecycle. After implementing validated standard
  initialization and a valid response, the focused test passed. The test then confirms `tools/list`
  returns exactly the two scoped retrieval tools, both calls return the fixed host owner, both
  model-supplied `user_identifier` attempts return tool errors, and neither denied call reaches
  the fake retriever.
- The discovery response now includes `ttlMs: 0` and `cacheScope: "private"` as required by rmcp's
  discovery result contract. The integration test follows rmcp's default legacy initialize path;
  its `tools/list` request is the tool discovery check. It does not claim to exercise modern
  `server/discover` startup or Huginn's approval layer.
- Startup inspection: `main` parses and validates MCP owner scope before embedder setup or
  `connect()`; `optional_stages()` returns no extraction, enrichment, or geocoding services for
  MCP. The new `--embedding-provider none` app test verifies no embedding identity is required and
  no embedder is constructed. These tests do not start Neo4j.
- Final checks: `cargo test -p muninn-mcp` passed (18 integration tests; 0 failures),
  `cargo test -p muninn` passed (24 tests; 0 failures), `cargo fmt --all -- --check` passed,
  `cargo clippy -p muninn-mcp --all-targets -- -D warnings` passed,
  `cargo clippy -p muninn --bin muninn -- -D warnings` passed, and `git diff --check` passed.
- Phase 3.3 remains unchecked because the test is located in `apps/muninn` and does not execute
  Huginn runtime `McpClients` or its startup/per-call approval flow. Phase 3.4 and 3.5 also remain
  unchecked; no Huginn approval or untrusted-output changes were made or claimed. Do not infer full
  Huginn runtime integration from this successful Muninn server boundary test.

## Read-Only Startup and Bounded Result Serialization Fixes

- Schema-policy TDD RED: `cargo test -p muninn --bin muninn schema_policy_keeps_mcp_read_only_and_setup_for_existing_commands`
  failed because MCP's policy plan still included `SetupAll`. GREEN passed after splitting common
  Neo4j connection from schema actions and selecting read-only validation for MCP. Bridge and
  `schema` retain `SetupAll` followed by vector-dimension validation. The deterministic policy
  test inspects those action plans without connecting to Neo4j.
- Owner scope validation remains before embedding/service setup and common Neo4j connection in
  `main`. No database was started or contacted.
- Bounded serialization TDD RED: `cargo test -p muninn-mcp --lib
  result_buffer_caps_serialized_bytes_at_limit_plus_one` failed because the writer retained all
  1,048,704 input bytes instead of capping at 1,048,577. GREEN passed after implementing the capped
  writer and using `serde_json::to_writer` into that buffer. The existing
  `oversized_results_fail_without_returning_the_payload` test passes and confirms the response
  emits only `Tool result exceeds size limit`, not result content.
- The serialization cap applies to the encoded output buffer: at most
  `MAX_RESULT_BYTES + 1` bytes are retained to detect overflow. The handler's `serde_json::Value`
  is already materialized and can itself contain a large object/string; this change does not claim
  a strict bound on total process memory or on allocations made while constructing that value.
- Final checks after both fixes: `cargo test -p muninn` passed (25 tests, 0 failures);
  `cargo test -p muninn-mcp` passed (19 total tests, 0 failures, including 18 integration tests);
  `cargo fmt --all -- --check`, `cargo clippy -p muninn-mcp --all-targets -- -D warnings`,
  `cargo clippy -p muninn --bin muninn -- -D warnings`, and `git diff --check` passed.
- At this checkpoint, Phase 3.3–3.5 remained unchecked because no Huginn runtime integration test
  existed. The later section below records the continuation that resolved this gap.

## Current continuation attempt

- Re-read `.agents/contexts/huginn.md`, Huginn ADRs 0006 and 0007, the runtime MCP integration
  tests and fixture, `McpClients`, and the Muninn `Catalog::ScopedRetrieval`/`Server` API before
  considering a cross-package fixture.
- Architecture decision check: `just huginn-architecture` passed, including its negative control.
  `.github/scripts/check-huginn-architecture.py` enforces dependency boundaries for
  `huginn-core` and `huginn-application`, not `huginn-runtime`; therefore a runtime-only dev
  dependency on `muninn-mcp` would not require broadening this script's allowlist. No manifest
  changes were made in this continuation attempt.
- `cargo test -p huginn-runtime --test mcp_lifecycle` passed (2 tests). An initial filtered
  invocation used a test name not present in that test target and ran 0 tests; it is not counted as
  verification evidence.
- `cargo test -p muninn-mcp` passed (24 tests total across unit/integration targets; 0 failed).
- `cargo test -p muninn` passed (25 tests; 0 failed).
- No test-only stdio fixture or Huginn `McpClients` integration test was added. These checks do
  not meet the Phase 3.3–3.5 acceptance points: real Huginn runtime client against the
  `muninn_mcp::Server`, startup approval/denial, approved and denied per-call behavior, and output
  handling. Leave tasks 3.3–3.5 unchecked. No production app, Neo4j, model, network, credentials,
  or service was launched or accessed.

## Huginn Runtime MCP Integration (2026-11-10)

- RED: Added an inline runtime integration test that calls the proposed private `McpClients::connect_with`; `cargo test -p huginn-runtime --lib actual_muninn_server_uses_huginn_startup_and_invocation_approvals` first failed because `connect_with` was missing. The initial test also exposed a fixture call-order type error; corrected the fixture before implementation.
- GREEN: Added private generic `connect_with` and shared `initialize_with`. Public `McpClients::connect` now uses that production helper with the existing sanitized environment, `ProcessGroup`, `KillOnDrop`, and `TokioChildProcess` setup. Existing plugin discovery retains its child-process setup and uses the shared initialization sequence.
- The runtime test uses Tokio duplex IO, an rmcp 3.8.0 client, `muninn_mcp::serve`, `Catalog::ScopedRetrieval`, asynchronous fake handlers, and `HostPolicy` with a deterministic recording prompt. It checks denied startup avoids connector invocation, approved startup discovers/registers two tools, approved retrieval returns the fixed owner result, invocation denial does not dispatch, and supplied `user_identifier` is rejected without dispatch. The client is shut down; the server task is aborted and awaited.
- Verification: focused runtime integration test passed (1); `cargo test -p huginn-runtime --test mcp_lifecycle` passed (2); `cargo test -p muninn` passed (25); `cargo test -p muninn-mcp` passed (19 total); `just huginn-architecture` passed including negative control; `cargo clippy -p huginn-runtime --all-targets -- -D warnings` passed; `cargo fmt --all -- --check` and `git diff --check` passed.
- An initial clippy pass found test placement after a test module and a held mutex guard; moved the module to file end and explicitly dropped the guard. Final clippy passed.
- No Huginn core/application contracts changed. No DB, app process, model, network, credentials, E2E, external service, or persistent data was used. Tasks 3.3–3.5 are checked. The separately gated synthetic E2E work remains unauthorized and unperformed.

## Orchestrator Verification (2026-11-10)

- Fresh checks passed after the runtime integration change:
  - `cargo test -p huginn-runtime --lib actual_muninn_server_uses_huginn_startup_and_invocation_approvals`: 1 passed.
  - `cargo test -p huginn-runtime --test mcp_lifecycle`: 2 passed.
  - `cargo test -p huginn-runtime --test transports mcp_denied_launch_and_approved_discovery_call_and_shutdown`: 1 passed.
  - `cargo test -p huginn-runtime --test transports mcp_crash_and_timeout_are_reported_and_shutdown_is_bounded`: 1 passed.
  - `cargo test -p muninn`: 25 passed; `cargo test -p muninn-mcp`: 19 total passed.
  - `cargo test -p muninn-application owner`: 6 matching tests passed across targets; `cargo test -p muninn-application scoped`: 1 passed.
  - `just huginn-architecture`, `cargo fmt --all -- --check`, Clippy with `-D warnings` for `huginn-runtime`, `muninn-mcp`, and `muninn`, and `git diff --check`: passed.
- The integration test exercises the production-shared `McpClients::connect_with` discovery/registration path and real `HostPolicy`, with actual rmcp 3.8.0 and `muninn_mcp::serve` over duplex IO. It does not start `apps/muninn`, Neo4j, or a model. Existing Huginn transport tests independently exercise the child-process stdio path, denial/approval, crash, timeout, and shutdown.
- The fake server does not exercise Neo4j adapter wiring. The `apps/muninn` composition test and existing owner-scope application tests cover fixed owner propagation/isolation at their respective boundaries. A live database retrieval check remains intentionally unperformed.
- Encoded MCP output is capped, and Huginn applies its existing `bounded_text` output cap. The handler's already-materialized result `Value` is not subject to a strict total-memory bound.
- Phase 3.3–3.5 are complete based on the combined runtime integration, transport, application-scope, and fresh focused verification above. Phase 4 remains blocked on explicit scenario/assertion/baseline/claim approval and local readiness gates. No inference or E2E run occurred.

## Scenario Approval (2026-11-10)

- The user approved `synthetic-e2e-proposal.md` exactly as written. Task 4.1 is complete.
- The user selected “Indicar ambos” for generation and embedding provider/model identifiers, but did not provide the identifiers yet. Task 4.2 remains blocked until the user names both already-ready local choices.
- No readiness check, service/database startup, or inference was performed after approval. The scenario approval alone does not authorize those actions.

## Synthetic E2E Execution (2026-11-10)

- The user selected the already-ready local generation model `qcwind/qwen3-8b-instruct-Q4-K-M:latest` and local MiniLM embeddings. Both were verified without inspecting credentials or downloading artifacts. Huginn used the same `32768` context budget in both conditions; Ollama reported a `40960` model context.
- A unique disposable Neo4j instance used the pinned image, a 2 GiB memory limit, tmpfs-only database/log state, no persistent mounts, and Bolt bound to host loopback. Docker Desktop could not expose Bolt from an internal network, so the user approved a normal bridge network; no external requests or downloads were made.
- The Ratatui TUI failed in the host terminal with a cursor-position timeout. The documented `--line-mode` recovery was used. Huginn retained normal startup, per-call, and file-write `HostPolicy` approvals. The user later authorized the scoped test execution so routine approvals did not require additional chat prompts; no policy bypass or `--allow` grant was used.
- The exact context guard rejected two pre-inference attempts at `8192` and `16384`. Those attempts used no model inference. The paired run used fresh session databases and workspace copies at `32768`, with identical prompts/model/context/tools.
- Baseline: both retrieval calls returned no records under the fixed empty scope. Huginn wrote the required three-line `summary.md` with all fields `UNKNOWN`; baseline assertions passed.
- Memory condition: message retrieval returned the seeded message `The release workspace identifier is Vesper-Delta-704.`. Preference searches returned no records. The fixture preference category is `campaign_status_report`, but the model supplied `preferences`; the connector forwards that category as a retrieval filter. The resulting `summary.md` had the identifier and `UNKNOWN` for title and columns. The memory-available condition failed; no retry with a corrected category was made.
- The MCP message response included the matching synthetic message's embedding vector. This was not needed for the task. The persisted evidence omits vectors and credentials; review response projection/data minimization before broader use.
- The model first returned the requested output in chat without creating a file. Both conditions received the same follow-up prompt to write that exact response to `summary.md`; both file writes passed through Huginn's normal approval.
- Evidence: `.agents/rpi/plan/tasks/muninn-memory-evaluation-benchmarking/evidence/synthetic-memory-recall-v1-20261110T101934Z-f8c62b5e/` (`run-manifest.json`, `seed-report.json`, `session-tool-evidence.json`, both output files, and `report.md`). The report records deviations and the limited conclusion. No general benchmark, parity percentage, or global parity claim is supported.
- The one paired comparison is complete and failed the memory-available acceptance criteria. Do not repeat it under this approved protocol. Next work requires a separate decision about category discoverability/normalization and minimizing returned message fields; any implementation change must follow TDD and receive its own focused verification.
