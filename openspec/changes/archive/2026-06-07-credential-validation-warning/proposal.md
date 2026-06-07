# Change Proposal: Provider Credential Validation Warning

## Why

A user with a **valid** Ollama Cloud API key currently hitting the weekly quota (HTTP 429 from `/api/chat`) cannot add their provider connection: the "Test connection" button reports `Unhealthy` and Save stays disabled. The credentials are fine — only the quota is exhausted. Today's `health_check()` conflates three distinct failure modes into one bucket: **auth rejected** (401/403), **quota exhausted** (429), and **server unreachable** (5xx / network). All three return `HealthStatus::Unhealthy`, which the dashboard renders as red, blocks Save, and gives the user no signal that the key itself is valid.

We adopt a cortex-flavored version of OmniRoute's 3-layer probe pattern (`tmp/OmniRoute/src/lib/providers/validation.ts`): probe the provider's cheapest read endpoint, classify the response into one of `ok | warning | unhealthy | unknown | expired`, and surface a **warning** state for 429 and no-token-but-reachable so a valid key with a transient quota hit is still addable.

## What Changes

### Behavioral changes

- `POST /api/providers/test-credentials` returns a new wire shape. `ok: Option<bool>` is replaced by `valid: bool`; two new optional fields are added: `warning: Option<String>`, `method: Option<String>`. The `status` field is constrained to a string enum: `"ok" | "warning" | "unhealthy" | "unknown" | "expired"`.
- **HTTP 429 from a credential probe** → `valid: true, status: "warning"`, `warning: "Rate limited, but credentials are valid"`. Save enabled; yellow alert.
- **HTTP 401/403** → `valid: false, status: "unhealthy"`, `error: "auth rejected: HTTP 401 — ..."`. Save disabled; red alert.
- **HTTP 5xx or network error** → `valid: false, status: "unhealthy"`, `error: "Cannot reach server: ..."`. Save disabled; red alert.
- **No API key configured + provider reachable** → `valid: true, status: "warning"`, `warning: "No API key configured. You can add one later via Edit."`. Save enabled; yellow alert.
- **Unknown (no probe implemented, e.g. Anthropic)** → `valid: true, status: "unknown"`, no error, no warning. Save enabled; no alert.
- **Expired (OAuth)** — UNCHANGED — `valid: false, status: "expired"`. Save disabled. The pre-existing short-circuit in `manage_connections.rs:163-179` is preserved verbatim.
- **Save button in the dashboard** is enabled iff `valid === true`, regardless of `status` or `warning`. The previous rule `ok === true` (which also blocked `ok === null` / Unknown) is replaced.

### Per-provider probe plan

| Provider     | Probe                                                            | Differentiates 401/403 | Differentiates 429 | No-token warning | No probe |
|--------------|------------------------------------------------------------------|------------------------|--------------------|------------------|----------|
| openai       | `GET {base_url}/v1/models` (Bearer)                              | ✓                      | ✓                  | ✓                |          |
| anthropic    | (none — returns `Unknown`)                                       | n/a                    | n/a                | n/a              | ✓        |
| gemini       | `GET {base_url}/v1beta/models` (Bearer or `?key=`)               | ✓                      | ✓                  | ✓                |          |
| groq         | `GET {base_url}/openai/v1/models` (OpenAI-compatible)            | ✓                      | ✓                  | ✓                |          |
| ollama       | 2-step: `GET /api/tags` (reachability) + `POST /api/chat` (auth) | ✓                      | ✓                  | ✓                |          |
| ollama-cloud | (likely same as ollama; confirm in `sdd-design` via `di.rs:810`) | ✓                      | ✓                  | ✓                |          |

## Scope

### In scope

- All 6 provider kinds: `openai`, `anthropic`, `gemini`, `groq`, `ollama`, `ollama-cloud`.
- **Domain** (`rook-core`): add a 4th variant `Warning { provider, latency_ms, reason }` to `HealthStatus`. Keep `ProviderKind` unchanged.
- **Application** (`rook-usecases`): extend `TestConnectionResult` (add `valid: bool`, `warning: Option<String>`, `method: Option<String>`). Update `from_health()` and `test_status_from_health()`. The `expired` short-circuit at `manage_connections.rs:163-179` is preserved.
- **Transport** (`transport-axum`): extend `TestConnectionResponse` DTO; rebuild the response from `TestConnectionResult`. No route signature change.
- **Per-provider providers**: refactor `health_check()` to differentiate 401/403, 429, and 5xx/network. Add a lightweight probe to `providers-gemini` and `providers-groq`. Extend `providers-ollama` (and `providers-ollama-cloud` if separate) to bucket 429 as warning.
- **Dashboard** (`apps/rook/dashboard`): update `TestConnectionResponse` TS interface in `lib/api.ts`; add a yellow `AlertTriangle` state to `AddProviderDialog.vue`; change `canSave` from `ok === true` to `valid === true` (or equivalently `valid !== false`).
- **Tests**: wiremock unit tests in each provider crate; integration tests in `manage_connections_test_credentials.rs` and `provider_routes.rs`; Vitest cases in `AddProviderDialog.spec.ts`; Playwright cases in `providers.spec.ts`.
- **Specs**: deltas on `provider-connections/spec.md` (§7.3, §10 AC), `provider-connections-transport/spec.md`, `providers-ui/spec.md` (Save-button rule + warning scenario).
- **Audit logging**: record `warning` distinctly from `error` in the audit log; include warning text for operator visibility.

### Out of scope (deferred)

- **Server-side rate limiting** (`rate-limiting/` spec) — different concern; not test-connection UX.
- **Provider fallback chains on 429** (`combo-execution/` spec) — server-side retry, separate.
- **Per-client rate limiting on test endpoints** — separate change.
- **A new `/api/providers/validate` endpoint** — OmniRoute splits this; we collapse into the existing `test-credentials` endpoint.
- **Persisting `warning` in `TestStatus`** — only `Active | Unhealthy | Unknown | Expired | NeverTested` is persisted. Warnings are wire-only.
- **Adding auth introspection to Anthropic** — Anthropic stays at `Unknown` (no probe). Future work.

## Impact

### Compatibility

- **Wire-protocol breaking change**: `TestConnectionResponse.ok: Option<bool>` → `valid: bool` + `warning` + `method`. Acceptable — no released versions yet.
- **Dashboard breaking change**: TS `TestConnectionResponse` interface. `api.ts` and `AddProviderDialog.vue` must change in lockstep.
- **Provider trait**: `HealthStatus` enum gains a 4th variant. All `ProviderPort` impls in tests must be updated.
- **`TestStatus` (persistence)**: UNCHANGED. The 5-variant enum keeps the same meaning; the warning is wire-only.

### Affected components

5+ Rust crates + 1 dashboard module, plus 3 spec files. Full file map in `findings.md` §7. Summary:

| Layer            | Files                                                                                                                     |
|------------------|---------------------------------------------------------------------------------------------------------------------------|
| Domain           | `crates/domain/rook-core/src/model.rs`, `crates/domain/rook-core/src/provider_connection.rs` (no change)                  |
| Application      | `crates/application/rook-usecases/src/manage_connections.rs`                                                              |
| Transport (wire) | `crates/infrastructure/transport-axum/src/provider_dto.rs`, `crates/infrastructure/transport-axum/src/provider_routes.rs` |
| Providers        | `providers-openai`, `providers-ollama`, `providers-ollama-cloud?`, `providers-gemini`, `providers-groq`                   |
| Dashboard        | `apps/rook/dashboard/src/lib/api.ts`, `apps/rook/dashboard/src/components/AddProviderDialog.vue`, `.spec.ts`              |
| Specs (delta)    | `provider-connections/spec.md`, `provider-connections-transport/spec.md`, `providers-ui/spec.md`                          |

## Capabilities

> Contract with `sdd-spec`. Each entry below is a delta target. No new capabilities.

### New Capabilities

- None. The change extends existing behavior; no new capability is introduced.

### Modified Capabilities

- `provider-connections`: extend `TestConnectionResult` shape (add `valid`, `warning`, `method`); add `Warning` variant to `HealthStatus`; codify 429-as-warning and no-token-as-warning rules in §7.3 and §10 AC.
- `provider-connections-transport`: rename wire field `ok` → `valid`; add `warning` and `method` fields; document new response examples (warning, no-token, unknown, expired).
- `providers-ui`: relax the "MUST require a successful credential test before Save" requirement to "Save enabled when `valid === true` regardless of warning"; add a third visual state (yellow) to the test-result block.

## Open Questions

> Resolved in this proposal where the answer is clear. Remaining ones are routed to `sdd-design`.

1. **Does OllamaCloud share Ollama's `ProviderPort` impl?** Routed to design. Check `apps/rook/src/di.rs:810-820` and the relevant `OllamaCloud` crate. If it shares, the 2-step probe covers both. If separate, the new logic must be added to that impl in parallel.
2. **Which step's `latency_ms` does the warning case report in the ollama 2-step probe?** Routed to design. Recommendation: report step 2's (the chat probe) — it's the more meaningful measurement when the chat endpoint is the one returning 429.
3. **Should the wire `method` field be a free-form string or an enum?** **Resolved here: free-form `Option<String>`.** Allows future probe types without schema churn; the small set of values (`"models_list" | "chat_probe" | "tags_reachability" | "not_supported" | "oauth_expired"`) is documented in the spec.
4. **Where does the dashboard's yellow styling live?** Routed to design. Recommendation: `AlertTriangle` from `lucide-vue-next` + Tailwind `text-yellow-600` (already in the icon import set).
5. **Should the audit log include the warning text?** **Resolved here: yes.** Operators need visibility into "test-credentials succeeded with warning" events.

## Non-Goals

- We are NOT introducing a new HTTP endpoint.
- We are NOT changing the persistence model (`TestStatus` stays 5-variant).
- We are NOT changing the OAuth expiry path (`expired` remains a hard failure).
- We are NOT introducing a separate `/validate` endpoint (OmniRoute does this; we collapse into the existing `/test-credentials`).
- We are NOT trying to detect free-tier vs paid-tier — we just respect the 429 we receive.

## Rollback Plan

- All changes are additive within existing modules; `git revert` of the change commits restores prior behavior cleanly.
- The wire-shape change is breaking but no released versions exist, so no migration is required.
- If a regression surfaces in production, the simplest mitigation is to keep the new `Warning` variant in the domain layer but have `from_health()` collapse it back to `Unhealthy` in the application layer — a one-line gate that preserves the new probe logic for follow-up fixes.

## Success Criteria

- [ ] A user with a valid Ollama Cloud key at weekly quota can click "Test connection", see a yellow warning, and Save the connection successfully.
- [ ] HTTP 401/403 still blocks Save with a red alert and the same error message format as before.
- [ ] HTTP 5xx / network errors still block Save with a red alert and a "Cannot reach server" message.
- [ ] OAuth-expired connections still return `status: "expired"` and block Save — no regression.
- [ ] `just ci-local` passes (clippy, fmt, Rust tests, Vitest, cargo doc, cargo audit).
- [ ] All three spec files (`provider-connections`, `provider-connections-transport`, `providers-ui`) carry a delta that captures the new behavior.
