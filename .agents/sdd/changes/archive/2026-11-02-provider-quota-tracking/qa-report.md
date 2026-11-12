# QA report — `provider-quota-tracking`

## 1. Identity and verdict

- **Change:** `provider-quota-tracking` (absorbs `per-provider-quota-integration`, U1–U3 as phase 5).
- **Mode:** `openspec`. **Phase:** `qa`. **Date:** 2026-11-02.
- **Execution:** `fallback`; there is no `.agents/sdd/quality-runner.json` and no QA runner with versioned envelopes in the repository.
- **QA verdict:** **PASS WITH WARNINGS** for the applicable acceptance scenarios.
- **Archive gate:** **AUTHORIZED WITH WARNINGS**. No acceptance-relevant scenario remains in `BLOCKED` or `NOT TESTED` state, and no CRITICAL/P0/P1 findings remain open.

The applicable acceptance scenarios have behavioral evidence re-run in this phase. The only accepted warning is the toolchain incompatibility of the local Semgrep gate, already resolved by re-running the scan with Python 3.12. This report does not declare operator acceptance.

## 2. Sources and technical handoff

The required artifacts were read:

- `proposal.md`
- `specs/provider-quota-capture/spec.md`, `specs/provider-usage-quota/spec.md`, `specs/providers-ui/spec.md`
- `design.md`
- `tasks.md`
- `verify-report.md`
- `state.yaml`
- `apply-progress.md`
- `.agents/sdd/config.yaml`
- `.agents/AGENTS.md`, `.agents/contexts/rook.md`, `.agents/contexts/dashboard.md`
- `docs/rook/architecture.md` (routing eligibility contract, to separate that surface from the dashboard warning)

**Verify handoff:** `verify-report.md` reports **PASS WITH WARNINGS**. V-01 (disk space failure) is resolved with `cargo clean` on regenerable artifacts; V-02 (Semgrep toolchain incompatibility) is resolved by re-running the gate with `--python 3.12`. No findings from that phase remain open.

QA did not repeat `fmt`, Clippy, `cargo doc`, coverage, trivy, gitleaks or full CI: those are technical controls delivered by Verify, not observable acceptance. QA did re-run the suites enumerated in the matrix.

## 3. Target, environment, permissions and limits

- **Target:** capture and presentation of upstream provider quotas, and the per-provider usage summary that exposes them.
- **Environment:** local Linux worktree, Cargo workspace, dashboard Vitest suite, Docker available for E2E. Providers and stores are controlled fixtures; there are no network calls to real providers.
- **Permissions and data:** no external credentials or persistent user data were used. There was no data reset. There was no commit, push or publication in this phase.
- **Observation limit:** capture is exercised with controlled provider responses. No real call to an external provider was made, because no authorized credentials exist in the environment; the absence of that verification does not alter the contract because capture depends on responses already observed by Cortex.

## 4. Capability inventory

| Capability                              | Status       | Selection and reason                                                                                                              |
|----------------------------------------|--------------|-----------------------------------------------------------------------------------------------------------------------------------|
| Upstream quota capture (U1–U3)         | Tested       | It is the origin of the feature data; without it the observations section would have no input.                                    |
| Per-provider usage summary (U5)        | Tested       | It exposes the separation between routed usage, routed 429s, local thresholds and upstream observations.                          |
| Quota dashboard (phase 3 and U6)       | Tested       | It is the visible surface where the user can confuse signals; it is where the feature is consumed.                                |
| Quota-based routing eligibility        | Out of scope | A different decision surface, with its own contract (`QuotaEligibilityPolicy`). Rule 9 of the design explicitly excludes it.      |

## 5. Acceptance scenario matrix

| ID    | Scenario                                                                        | Observable criterion                                                                                             | Evidence                                                                                                                                          | Status |
|-------|---------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------|--------|
| QA-01 | Capture does not alter the local usage history                                  | Writing an observation does not modify `usage_history`                                                       | `cargo test -p audit-sqlite` — `quota_observation_round_trips_values_nulls_and_runtime_identity` and the `latest_observations` tests per series      | PASS   |
| QA-02 | The observation is not derived from local usage                                 | There is no derivation from tokens, cost or 429 frequency                                                       | `cargo test -p rook-core --test quota_tests` — `provider_scoped_observation_does_not_prove_connection_exhaustion` (10 tests)                       | PASS   |
| QA-03 | The most recent series wins and expired evidence does not prove exhaustion      | `latest_observations` keeps the most recent observation per series and `stale_evidence_is_unknown_not_exhausted` | `cargo test -p audit-sqlite` (21 tests) and `cargo test -p rook-core --test quota_tests`                                                             | PASS   |
| QA-04 | Provider headers are captured with provenance                                   | OpenAI and Anthropic expose their quota headers to the adapter                                                   | `cargo test -p providers-openai --test quota_headers` (1) and `-p providers-anthropic --test quota_headers` (3)                                      | PASS   |
| QA-05 | The summary exposes Cortex-only coverage                                        | The response declares `coverage: cortex_routed` and the source                                              | `cargo test -p transport-axum --test provider_quota_routes` — `get_provider_quota_returns_json_with_explicit_usage_coverage_and_source` (13 tests) | PASS   |
| QA-06 | Upstream observations are exposed separately from usage, 429s and thresholds    | `upstreamObservations` field with provenance, scope, window, age and freshness                                  | `provider_quota_routes` — `get_provider_quota_exposes_upstream_observations_separately_with_provenance_and_age`                                    | PASS   |
| QA-07 | Absence of observations is not presented as health or absence of a limit        | Explicit `not_reported`                                                                                         | `provider_quota_routes` — `get_provider_quota_marks_absent_upstream_observations_not_reported`                                                     | PASS   |
| QA-08 | A failure reading observations is marked `unavailable` without losing the rest | Isolated failure of one query                                                                                    | `provider_quota_routes` — `get_provider_quota_marks_failed_upstream_observation_read_unavailable`                                                  | PASS   |
| QA-09 | Partial or unknown cost is not serialized as zero                               | `costUsd: null` with `costAvailability`                                                                           | `provider_quota_routes` — `get_provider_quota_reports_partial_and_unknown_cost_without_losing_provenance`                                          | PASS   |
| QA-10 | Local Ollama has no account quota                                               | `not_applicable` and no fabricated values                                                                        | `provider_quota_routes` — `get_provider_quota_marks_ollama_account_quota_not_applicable`                                                           | PASS   |
| QA-11 | The view renders the observations section with provenance                       | Dedicated section with metric, scope, window, limit, remaining, source, freshness, age and reset                 | `vitest src/views/ProvidersQuotaView.spec.ts` — `renders upstream observations with provenance when available`                                     | PASS   |
| QA-12 | With no captured observations an honest empty state is shown                    | Explicit empty state, without quota values                                                                     | `vitest` — `renders not_reported state when no upstream observations exist`                                                                        | PASS   |
| QA-13 | With `unavailable` availability the observations section is not rendered        | The section does not appear and absence of signal is not claimed                                                 | `vitest` — `does not render the upstream section when unavailable`                                                                                 | PASS   |
| QA-14 | The local warning remains about routed traffic                                  | Explicit local warning label about routed traffic                                                   | `vitest` — label assertion in the status cell and `providers.quota.localWarningLabel` in `en.json`/`es.json`                                 | PASS   |
| QA-15 | Partial cost is labeled as unknown without a numeric subtotal                   | No `$1.25` in the cost cell                                                                                 | `vitest` — `labels partial cost as unknown without showing a numeric subtotal`                                                                     | PASS   |
| QA-16 | Trends show seven days                                                          | Seven trend points with estimated cost                                                                     | `vitest` — `renders seven days of trend usage with cost when available`                                                                            | PASS   |
| QA-17 | A failed refresh keeps the last result and marks a degraded state               | Retained data and visible `stale` state                                                                         | `vitest src/composables/useProvidersQuota.spec.ts` — `keeps the last good result and exposes stale state after refresh failure`                    | PASS   |
| QA-18 | Recovery clears the degraded state                                              | After a successful refresh no stale data marker remains                                                         | `vitest` — `clears stale state after a successful recovery`                                                                                        | PASS   |

Suite re-run in this phase: `rook-core` 10 tests, `transport-axum` 13 tests, `audit-sqlite` 21 tests, provider headers 4 tests, dashboard 9 tests (5 for the view and 4 for the composable). Total 57 tests, all green.

## 6. Findings

| ID      | Severity | Description                                                                                                                                                                                                                                                                                                             | Status                                                                              |
|---------|-----------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------|
| QA-F-01 | INFO      | `just ci-semgrep` fails locally because `uv run --no-project` selects Python 3.14 and the `protobuf` pinned by `semgrep==1.126.0` fails to import. CI is not affected (it installs with `pip` on the runner Python). The scan passes when pinning `--python 3.12`. It is not a product or feature finding. | Accepted as a non-blocking warning; the repository recipe was not modified   |
| QA-F-02 | INFO      | No real call to an external provider was executed to observe quota headers. Capture was verified with controlled responses.                                                                                                                                                                              | Accepted as a non-blocking warning; no authorized credentials in the environment |
| QA-F-03 | INFO      | SonarCloud and the push-only build were not executed: they require `SONAR_TOKEN` and a remote service, or a push trigger.                                                                                                                                                                         | Outside the local set by design; they are not reported as approved                  |

No CRITICAL, P0 or P1 findings.

## 7. Traceability with the specs

| Requirement (spec)                                                                                                                            | QA scenarios                                          | Status |
|-----------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------|--------|
| `provider-quota-capture`: separation of observations and local usage                                                                          | QA-01, QA-02, QA-03, QA-04                             | PASS   |
| `provider-usage-quota`: coverage, periods, unknown cost, upstream signals, local limit                                                          | QA-05, QA-06, QA-07, QA-08, QA-09, QA-10               | PASS   |
| `providers-ui`: real dashboard, coverage, trends, distinct thresholds, captured signals, degraded refresh, Ollama without account quota        | QA-11, QA-12, QA-13, QA-14, QA-15, QA-16, QA-17, QA-18 | PASS   |

## 8. Conclusion

The three capabilities of the feature have behavioral evidence re-run and every applicable acceptance scenario is PASS. No acceptance-relevant scenario remains in `BLOCKED` or `NOT TESTED`. The archive gate is authorized with warnings, conditional on `verify-report.md` and this `qa-report.md` existing and on syncing the delta specs to the main specs.
