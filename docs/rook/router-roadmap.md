# Rook Router Roadmap

Implementation plan for turning Rook into a small, intelligent AI router.

**Status:** approved specification, awaiting Linear import. **Scope:** Rook only. Huginn is not covered. **Reference basis:** an external AI gateway implementation, consulted only as a capability checklist — never as a template.

## Why this document exists

The Linear workspace has reached its free issue limit, so the 27 issues below could not be created in the tracker. They are specified here in Linear-ready form: each entry has the exact title, priority, labels, dependency edges, `path:line` anchors, scope, acceptance criteria and explicit out-of-scope. Creating them later is a copy operation, not an analysis.

## Decisions locked

1. **Delete the format-translation registry.** `FormatRegistry::format_for` has no production caller, both shipped translators are no-ops, and the real wire mapping already happens in DTO `From` impls plus each adapter. The domain model is the pivot. A pair registry needs N-squared translators; a domain pivot needs N. Keeping the layer would invite contributors to fill it with duplicate logic.
2. **The provider boundary is the protocol, not the vendor.** Four protocol adapters replace one crate per provider, and providers become data in a descriptor table.
3. **Selection is eligibility, then scoring, then explanation.** No weighting input may reintroduce a rejected candidate. Every rejection and every score component is named.
4. **Google parity starts with Gemini AI Studio** (official API key, `generateContent`). The Antigravity IDE is deferred to the imported-token tier and gated by DALLAY-692. Nothing is built on an assumption that consumer subscriptions are reusable gateway credentials.
5. **`tokens_per_minute` is removed.** It is persisted and documented but never enforced.

## Explicitly out of scope

Each of these is a product bet that makes the router larger without making it better at choosing a target.

Compression engines, fusion and pipeline combos, per-step prompts, shadow routing, adaptive or learned selection, model modalities beyond text and embeddings, web-cookie session authentication, IDE-token proxying, API key groups with wildcards, IP allowlists, access schedules, device binding, lease scopes, SSO-sourced keys, and any Electron shell.

## Parity snapshot

| Area                | Rook today                                                           | Reference                                          | Coverage |
|---------------------|----------------------------------------------------------------------|----------------------------------------------------|----------|
| API key governance  | 5 typed scopes, tiers, allowlists, USD budgets, rotation, revocation | + groups, IP allowlists, schedules, device binding | ~70%     |
| Routing selection   | 1 wired strategy (`priority`), no scoring                            | 21 strategies, 16-factor scorer                    | ~15%     |
| Format and response | Domain pivot, SSE for OpenAI and Anthropic                           | N-squared pair registry, compression engines       | ~20%     |
| Client endpoints    | 4 client-facing routes                                               | 40+ families                                       | ~10%     |
| Providers           | 8 kinds, 5 adapter crates, 2 auth mechanisms                         | ~380 provider ids, 8 auth mechanisms               | ~5%      |
| Combos              | Sound model, 1 strategy, no weights or nesting                       | Schema v2, nesting, wildcards, weights, fusion     | ~35%     |

Rook is already ahead on two axes and should not trade them away: key security (HMAC with a server secret, restrictions that fail closed, no plaintext at rest) and translation topology (domain pivot instead of an N-squared matrix).

## Roadmap at a glance

| Phase | Goal                                         | Issues | Gate                              |
|-------|----------------------------------------------|--------|-----------------------------------|
| 0     | Integrity: remove what lies, keep what works | 6      | Suite green, no dead declarations |
| 1     | Descriptor registry: providers become data   | 8      | Adding NaN costs zero Rust        |
| 2     | Selection engine: the intelligent part       | 6      | Every choice is explainable       |
| 3     | Explainability surface                       | 2      | Preview matches execution         |
| 4     | Client surface completion                    | 2      | Gemini and embeddings work        |
| 5     | Authentication breadth                       | 3      | Drivers, gated by DALLAY-692      |

Dependency edges are declared per issue below (`Depends on`). Phase 0 and Phase 1 can run concurrently; Phase 2 depends on Phase 1.

---

## Phase 0 — Integrity

Shrink before growing. Nothing here adds a feature; every issue removes a false claim or a duplicated mechanism so later phases have less to reason about.

### ROOK-P0-1 · \[ROOK\]\[P1\] Delete the dead format-translation layer

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| High     | `product/rook` `type/chore` `area/transport` | S    | —          |

#### Context

`FormatRegistry` is presented as Rook's provider format translation layer, but it has no production effect:

- `FormatRegistry::format_for` (`crates/infrastructure/transport-axum/src/format_registry.rs:167-175`) has no production caller.
- `IdentityTranslator` (`format_registry.rs:28-42`) and `DomainPivotTranslator` (`:50-62`) both return their input unchanged.
- The only cross-format registrations (`apps/rook/src/di.rs:339-350`, OpenAI to Anthropic) therefore translate nothing.
- Real wire mapping already happens elsewhere: each inbound route has a serde `From` impl into the domain model, and each protocol adapter builds its provider wire format from the domain model.
- `format_registry.rs:122-149` rewrites tools and `tool_choice` gated on `metadata.origin == "responses"`. That is a Responses concern living in the wrong layer.

#### Scope

- Delete the registry, `IdentityTranslator`, `DomainPivotTranslator`, the request and response translator getters, `translate_request`, `translate_response`, `format_for`, and the `missing_translator` error path.
- Delete the registrations at `apps/rook/src/di.rs:339-350`.
- Move the `origin == "responses"` tools and `tool_choice` rewriting into the Responses request conversion.
- Remove per-attempt translation call sites that only invoked the registry. Verify `route_request.rs:745-757`, `:1181`, `:1877` and `combo_stream.rs:317`.

#### Keep

- `ApiFormat` on the inbound route declaration: it still records which client dialect arrived.
- DTO to domain `From` impls.
- Adapter-owned domain to provider-wire mapping.

#### Acceptance criteria

- [ ] No symbol named `FormatRegistry`, `IdentityTranslator`, `DomainPivotTranslator` or `format_for` remains in the workspace.
- [ ] `cargo test -p transport-axum` and the gateway integration tests pass unchanged.
- [ ] An OpenAI-dialect request routed to an Anthropic connection still produces a valid Anthropic wire request and a valid OpenAI response, covered by a test.
- [ ] A `/v1/responses` request still reaches an Anthropic connection with correctly reshaped tools.
- [ ] No code path can return a "missing translator" error.

#### Out of scope

Adding new wire formats. Gemini ingress is ROOK-P4-2.

### ROOK-P0-2 · \[ROOK\]\[P1\] Make the declared routing surface truthful

| Priority | Labels                                     | Size | Depends on |
|----------|--------------------------------------------|------|------------|
| High     | `product/rook` `type/chore` `area/runtime` | S    | —          |

#### Context

`RoutingStrategy` (`crates/application/rook-usecases/src/router_impl.rs:28-34`) declares four variants; only `Priority` is ever constructed (`bootstrap_helpers.rs:65`, `apps/rook/src/di.rs:195-196`).

- `ModelBased` (`router_impl.rs:371-373` and `:412`) is a stub that logs a TODO and returns the first candidate.
- `RoundRobin` is implemented but unreachable from configuration.
- `WeightedRandom(Vec<f32>)` is implemented, but `StrategyConfig` (`apps/rook/src/config.rs:280-292`) has no way to supply weights, so it can never be selected.
- `docs/rook/configuration.md:88-91` documents `priority|round-robin|model-based` as if all three worked.

A declared but unreachable strategy is a defect, not a roadmap item.

#### Scope

- Delete `RoutingStrategy::ModelBased` and both stub arms, including the TODO comments at `router_impl.rs:372`.
- Keep `Priority`, `RoundRobin` and `WeightedRandom` as internal variants, and document that only `priority` is configurable today.
- Rewrite the routing-strategy section of `docs/rook/configuration.md` to state exactly what can be configured.

#### Acceptance criteria

- [ ] No stub strategy remains: every `RoutingStrategy` variant produces a distinct, implemented selection order.
- [ ] `docs/rook/configuration.md` lists exactly the strategy values a user can put in configuration, and nothing more.
- [ ] Configuration parsing rejects any unknown strategy value with a clear error.
- [ ] A test enumerates the configurable strategy set so documentation and code cannot drift silently.

#### Out of scope

The new policy set (priority, weighted, round-robin, least-latency, least-cost, auto). That is ROOK-P2-3.

### ROOK-P0-3 · \[ROOK\]\[P1\] Consolidate the three TokenBucket implementations

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| High     | `product/rook` `type/chore` `area/transport` | S    | —          |

#### Context

Three middleware modules each define a private `TokenBucket` with the same refill, consume and retry-after logic:

- `crates/infrastructure/transport-axum/src/middleware/api_key_rate_limiter.rs`
- `crates/infrastructure/transport-axum/src/middleware/ip_rate_limiter.rs`
- `crates/infrastructure/transport-axum/src/middleware/login_rate_limiter.rs`, whose variant hard-codes its constants

DALLAY-639 is marked Done, but the duplication persists across these three modules. An unused shared copy in a `provider-utils` crate was removed instead of being wired in.

#### Scope

- One `TokenBucket` type in `transport-axum`, parameterized by capacity and refill rate.
- All three limiters consume it.
- Preserve current limits, `Retry-After` values and the one-hour retry cap exactly.

#### Acceptance criteria

- [ ] Exactly one `TokenBucket` definition exists in the workspace.
- [ ] Unit tests cover refill, consume, retry-after computation and the one-hour cap.
- [ ] HTTP-level tests prove each limiter keeps its current observable behavior: status code and `Retry-After`.
- [ ] No rate-limit policy value changes.

#### Out of scope

Changing rate-limit policies or the admin rule store.

### ROOK-P0-4 · \[ROOK\]\[P1\] Remove the unenforced tokens_per_minute field

| Priority | Labels                                  | Size | Depends on |
|----------|-----------------------------------------|------|------------|
| High     | `product/rook` `type/chore` `area/auth` | S    | —          |

#### Context

`tokens_per_minute` is persisted on API keys, exposed through the management API and shown in the dashboard, but nothing enforces it. `api_key_rate_limiter.rs` limits by requests-per-minute per tier; the token dimension is labelled "reserved".

A field a user can set that has no effect is worse than an absent field: it creates false confidence that a key is token-bounded.

#### Scope

- Remove `tokens_per_minute` from the API key domain model (`crates/domain/rook-core/src/api_key.rs`), the SQLite mapping and migration, the management DTOs, and the dashboard.
- Update `docs/rook/api.md` so it documents only the limits that exist.

#### Acceptance criteria

- [ ] No reference to `tokens_per_minute` remains in code, migrations, dashboard or docs.
- [ ] Existing API keys load and authenticate unchanged.
- [ ] The management API no longer returns the field and rejects it on write with a clear error.
- [ ] Requests-per-minute limits are unaffected.

#### Out of scope

Implementing real per-key token limits. If that becomes a requirement it needs its own accounting design.

### ROOK-P0-5 · \[ROOK\]\[P1\] Fix per-key budgets rejecting every request

| Priority | Labels                                   | Size | Depends on |
|----------|------------------------------------------|------|------------|
| High     | `product/rook` `type/bug` `area/runtime` | S–M  | —          |

#### Context

`ProviderPort` exposes `strict_max_cost_micros_usd` and `strict_realized_cost_micros_usd` with default implementations returning `None` (`crates/domain/rook-core/src/ports.rs:35-92`). No adapter overrides them.

As a result, any request using an API key with a configured spending budget is rejected before it reaches a provider, because the gateway cannot prove a cost ceiling (`crates/application/rook-usecases/src/route_request.rs:445-456`, documented at `docs/rook/api.md:55-63`). The budget feature exists and is unusable.

#### Scope

- Derive a strict upper cost bound from model-catalog pricing metadata for models that declare pricing.
- Implement `strict_max_cost_micros_usd` in the protocol adapters where the catalog provides enough information.
- Keep failing closed when pricing is unknown, but report a distinct reason instead of a generic budget rejection.

#### Acceptance criteria

- [ ] A request from a budgeted key succeeds for a model whose pricing is declared in the catalog.
- [ ] A request for a model with unknown pricing still fails closed, with an error saying pricing is unavailable rather than that the budget is exhausted.
- [ ] Concurrent requests cannot overspend: the reserve-then-reconcile path is exercised by a test.
- [ ] Spending accounting is unchanged for keys without a budget.

#### Out of scope

Upstream billing reconciliation and provider-side cost verification.

### ROOK-P0-6 · \[ROOK\]\[P2\] Correct documentation drift in Rook docs

| Priority | Labels                      | Size | Depends on |
|----------|-----------------------------|------|------------|
| Medium   | `product/rook` `type/chore` | S    | —          |

#### Context

Observed divergence between documentation and code:

1. `docs/rook/architecture.md:220` says `routes.rs` exposes "four endpoints". It defines 16; the router mounts 60 in total.
2. `crates/infrastructure/models-catalog/src/models.json:12-20` uses `"ollamacloud"` while `ProviderKind::as_str()` returns `"ollama-cloud"` (`provider_connection.rs:97-110`). Only the serde alias makes it work; a new consumer reading the JSON directly will break.
3. `docs/rook/api.md:28` documents `DELETE /api/cache/signature/{sig}`, but the mounted route is `DELETE /api/cache/{signature}` (`routes.rs:1307-1320`). Both forms exist.
4. `docs/rook/architecture.md:225` lists adapter files but omits `responses_adapter.rs`, `responses_stream.rs` and the oauth, combo, alias, rate-limit and cache routers merged into `router()`.

#### Scope

- Correct each item above against the code.
- Make the catalog JSON key spelling match `as_str()` exactly, or document the alias as the intended contract.

#### Acceptance criteria

- [ ] Each corrected claim matches the code and is verifiable by reading one file.
- [ ] The route count stated in `architecture.md` matches the mounted router.
- [ ] `pnpm exec markdownlint-cli2 "docs/rook/*.md"` passes.

#### Out of scope

Rewriting the documentation. Only false claims are corrected.

---

## Phase 1 — Descriptor registry

The structural investment. After this phase, adding a provider is data.

### ROOK-P1-1 · \[ROOK\]\[P1\] Introduce ProviderDescriptor and the descriptor table

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| High     | `product/rook` `type/feature` `area/runtime` | M    | —          |

#### Context

Rook identifies a provider by the closed enum `ProviderKind` (`crates/domain/rook-core/src/provider_connection.rs:83-95`) and re-derives provider semantics in six separate places. There is no single declaration of what a provider is.

The model is wrong, not merely verbose: `ProviderKind::OpenAiCompatible` is missing from `all_provider_kinds` (`handlers/provider_quota.rs:600-609`), from the dashboard union type and from `format_for`. The fan-out sites have already drifted.

Two providers are descriptors in disguise: Azure OpenAI is `OpenAIProvider` with `base_url = "{endpoint}/openai/v1"` plus OAuth (`apps/rook/src/di.rs:1196-1223`), and OpenAI-compatible is the same type with an endpoint policy and a required base URL (`di.rs:1094-1116`).

#### Scope

Introduce a declarative descriptor:

```rust
pub struct ProviderDescriptor {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub protocol: Protocol,
    pub auth: AuthKind,
    pub default_base_url: &'static str,
    pub paths: WirePaths,
    pub canonical_base_urls: &'static [&'static str],
    pub capability_ceiling: Capabilities,
    pub models: ModelSource,
}
```

- `Protocol` = `OpenAiChat | AnthropicMessages | GeminiGenerateContent | OllamaNative`.
- `AuthKind` = `ApiKey | OAuth | ImportedToken`.
- `WirePaths` carries `chat`, `models` and `health` as data. This is what forces Groq's hardcoded paths out of code (`providers-groq/src/lib.rs:209,345,423`).
- `ModelSource` = `Static(&[..]) | Catalog | Dynamic`.

Seed 16 descriptors: the 8 existing kinds plus `xai`, `mistral`, `deepseek`, `together`, `fireworks`, `cerebras`, `openrouter`, `nan`.

#### Acceptance criteria

- [ ] A single descriptor table is the only place that declares provider identity, protocol, auth mechanism, base URL, wire paths, canonical hosts and capability ceiling.
- [ ] The table is queryable by id and by alias.
- [ ] A unit test iterates the table and asserts every descriptor has a protocol, an auth kind, a base URL and at least one model source.
- [ ] `nan` appears with base URL `https://api.nan.builders/v1` and its catalogued models.

#### Out of scope

Wiring descriptors into dependency injection (ROOK-P1-3) and deleting `ProviderKind` (ROOK-P1-4).

### ROOK-P1-2 · \[ROOK\]\[P1\] Collapse provider crates into four protocol adapters

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| High     | `product/rook` `type/feature` `area/runtime` | L    | ROOK-P1-1  |

#### Context

Today there is roughly one adapter crate per provider (`providers-openai`, `providers-anthropic`, `providers-ollama`, `providers-gemini`, `providers-groq`), each implementing the 10-method `ProviderPort` (`crates/domain/rook-core/src/ports.rs:35-92`). Several are the same protocol with a different host:

- `openai-chat` covers OpenAI, OpenAI-compatible, Azure OpenAI and Groq. `providers-groq` differs only in hardcoded `/chat/completions` (`:209,423`) and a `/openai/v1/models` health probe (`:345`).
- `ollama-native` covers Ollama and Ollama Cloud.

#### Scope

- Reduce to four adapters: `openai-chat`, `anthropic-messages`, `gemini-generateContent`, `ollama-native`.
- Move every provider-specific path and host into `ProviderDescriptor.paths` and `default_base_url`; no path literal remains in an adapter.
- Delete the redundant crates once their behavior is covered.

#### Acceptance criteria

- [ ] Exactly four provider adapter implementations exist.
- [ ] No adapter contains a provider-specific URL path or host literal.
- [ ] Groq, Azure OpenAI, OpenAI-compatible and Ollama Cloud keep their current observable behavior: request shape, health probe, `api_format()`.
- [ ] Existing adapter tests move with the code and pass.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes.

#### Out of scope

Adding new protocols (Bedrock, Vertex). Those are separate later issues.

### ROOK-P1-3 · \[ROOK\]\[P1\] Build adapters from descriptors instead of a match

| Priority | Labels                                       | Size | Depends on           |
|----------|----------------------------------------------|------|----------------------|
| High     | `product/rook` `type/feature` `area/runtime` | M    | ROOK-P1-1, ROOK-P1-2 |

#### Context

`apps/rook/src/di.rs:1049-1225` is a single `match kind` with eight arms, each constructing a provider-specific `*ProviderConfig` literal. Around it, three more matches re-derive provider behavior:

- `is_canonical_base_url` (`di.rs:1001-1017`)
- model sourcing (`di.rs:1347-1352`)
- `adapter_capability_ceiling` (`di.rs:1398-1421`)

Adding a provider means editing all four plus the wrapper at `di.rs:1336-1385`.

#### Scope

- Construct an adapter from `(descriptor, connection)` with no provider-identity branching: the protocol selects the adapter type, the descriptor supplies URL, paths and capabilities.
- Replace the three auxiliary matches with descriptor lookups.
- Keep `DynamicProviderBuilder::build` behavior (hot reload) unchanged.

#### Acceptance criteria

- [ ] `di.rs` contains no `match` or `if` on provider identity.
- [ ] Selecting an adapter is a function of `descriptor.protocol` alone.
- [ ] All 8 migrated providers construct successfully and route.
- [ ] Hot reload of a connection still takes effect on the next request without a restart.
- [ ] A test registers a descriptor for a provider with no dedicated crate and observes a working adapter.

#### Out of scope

Removing `ProviderKind` from the domain (ROOK-P1-4).

### ROOK-P1-4 · \[ROOK\]\[P1\] Replace ProviderKind with a descriptor reference

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| High     | `product/rook` `type/feature` `area/storage` | M    | ROOK-P1-1  |

#### Context

`ProviderKind` is the persisted identity of a provider connection (`provider_connection.rs:83-95`). Its serde spelling and `as_str()` disagree: `#[serde(rename_all="lowercase")]` yields `"ollamacloud"` and `"azureopenai"`, while `as_str()` yields `"ollama-cloud"` and `"azure-openai"`. Once the descriptor table exists, the enum is a second source of truth.

#### Scope

- Represent a connection's provider as a descriptor id, validated against the table at load and at write time.
- Add a SQLite migration mapping stored values to canonical ids: `"ollamacloud"` to `"ollama-cloud"`, `"azureopenai"` to `"azure-openai"`, others unchanged.
- Keep reading legacy values through an explicit alias map so pre-migration rows never fail.

#### Acceptance criteria

- [ ] An existing database with all 8 legacy kinds migrates and every connection loads, authenticates and routes unchanged.
- [ ] A connection referencing an unknown provider id is rejected with a clear validation error, not a panic.
- [ ] `ProviderKind` no longer appears in persisted or serialized data.
- [ ] The migration is covered by an automated test that starts from legacy rows.

#### Out of scope

Renaming or removing provider ids that are already public.

### ROOK-P1-5 · \[ROOK\]\[P1\] Retrofit the drift sites onto the descriptor table

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| High     | `product/rook` `type/feature` `area/runtime` | M    | ROOK-P1-4  |

#### Context

Provider semantics are re-derived in the following locations, each of which must consult the descriptor table instead:

- `crates/application/rook-usecases/src/manage_connections_validation.rs:163-170` — `normalize_api_format`
- `crates/application/rook-usecases/src/manage_connections.rs:1579-1594` — OAuth scopes; `:1610-1645` and `:1680-1700` — authorize and token endpoints
- `crates/application/rook-usecases/src/oauth_tokens.rs:193-230`
- `crates/infrastructure/transport-axum/src/provider_dto.rs:419-424` — default `api_format`
- `crates/infrastructure/transport-axum/src/handlers/provider_quota.rs:600-609` — `all_provider_kinds` is a fixed `[ProviderKind; 7]` array that already omits `OpenAiCompatible`
- `crates/infrastructure/models-catalog/src/lib.rs` — catalog keys derived from kinds

Equality-only checks that encode provider semantics also belong here: `manage_connections.rs:180,256,265,494,767,831-834,985-988,1018,1650,1708` and `handlers/models.rs:36-44`.

#### Scope

- Replace each site with a descriptor lookup.
- Fix `all_provider_kinds` by iterating the table.

#### Acceptance criteria

- [ ] No provider-identity match or equality check remains outside the descriptor table.
- [ ] `all_provider_kinds` returns every descriptor, including OpenAI-compatible.
- [ ] OAuth scopes and authorize and token endpoints come from the descriptor.
- [ ] `/v1/models` and the quota view list the same provider set as the table.

#### Out of scope

Changing the OAuth flow itself (ROOK-P5-2).

### ROOK-P1-6 · \[ROOK\]\[P1\] Unify the model catalog and declare Tier-1 models

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| High     | `product/rook` `type/feature` `area/runtime` | M    | ROOK-P1-1  |

#### Context

`crates/infrastructure/models-catalog/src/lib.rs:19-70` holds bundled `models.json` and TOML-configured declarations in two separate vectors. `list()` chains them and dedupes by `(provider_kind, model_id)` (`:78-101`); `resolve()` re-applies precedence per lookup (`:103-136`). The two sources are never merged, so every read re-implements precedence.

#### Scope

- Merge bundled and configured declarations once at construction into a single indexed view.
- Preserve the documented precedence: bundled, then configured global, then configured connection-scoped.
- Add model entries for the new Tier-1 providers, including NaN's catalogue: DeepSeek V4-Flash, GLM 5.3, GLM 5.3-Flash, Qwen 3.8-Flash, Qwen 3.6, MiMo v2.6-Flash and Gemma 4.
- Record capabilities that providers document: context window, vision, tool calling, reasoning.

#### Acceptance criteria

- [ ] One merged catalog backs `list()`, `list_for_connection()` and `resolve()`.
- [ ] Precedence is applied once, at construction, and covered by tests.
- [ ] Connection-scoped overrides still win for their connection and do not leak to others.
- [ ] Unknown capabilities stay unknown rather than defaulting to a claim.
- [ ] `docs/rook/model-catalog.md` matches the merged behavior.

#### Out of scope

Fetching model lists dynamically from providers.

### ROOK-P1-7 · \[ROOK\]\[P1\] Drive the dashboard provider catalog from the backend

| Priority | Labels                                         | Size | Depends on |
|----------|------------------------------------------------|------|------------|
| High     | `product/rook` `type/feature` `area/dashboard` | M    | ROOK-P1-1  |

#### Context

The dashboard duplicates provider knowledge in frontend code: `providerCatalog.ts:39-45`, a `PROVIDER_KINDS` union, `ProviderIcon.vue:56`, `AddProviderDialog.vue:151-156,666,772-781`, and i18n strings. This is why `OpenAiCompatible` is missing from the frontend union: the same drift as the backend.

#### Scope

- Expose the descriptor set through an admin endpoint: id, label, protocol, auth kinds, capability ceiling, OAuth support, icon key.
- Drive the add-provider dialog, the provider list and icon selection from that response.
- Keep icons as a frontend asset keyed by descriptor id, with a fallback.

#### Acceptance criteria

- [ ] Adding a descriptor requires no change to any TypeScript union type or dialog option list.
- [ ] An unknown descriptor id renders with a fallback icon instead of failing.
- [ ] Existing provider screens behave identically for the 8 current providers.
- [ ] Dashboard tests pass and the new endpoint is covered.

#### Out of scope

Redesigning the dashboard.

### ROOK-P1-8 · \[ROOK\]\[P1\] Prove plug and play: NaN is a table entry only

| Priority | Labels                                    | Size | Depends on           |
|----------|-------------------------------------------|------|----------------------|
| High     | `product/rook` `type/test` `area/runtime` | S    | ROOK-P1-3, ROOK-P1-6 |

#### Context

The point of the descriptor refactor is that adding a provider becomes data. The best available litmus test is NaN (nan.builders), an OpenAI-compatible community inference cluster:

- Base URL: `https://api.nan.builders/v1`
- Auth: bearer API key (`sk-...`), no OAuth, no device flow
- Models: DeepSeek V4-Flash, GLM 5.3 and 5.3-Flash, Qwen 3.8-Flash and 3.6, MiMo v2.6-Flash, Gemma 4, plus embedding, rerank, TTS/STT and image models
- Documented as drop-in compatible: change the base URL and the key, nothing else

If adding NaN requires any Rust change in an adapter crate or any edit to `di.rs`, the refactor is not finished.

#### Scope

- Register NaN through the descriptor table and the catalog only.
- Add an integration test proving a request routes to NaN and that `/v1/models` lists its models.

#### Acceptance criteria

- [ ] The NaN commit touches only the descriptor table, the catalog data and tests.
- [ ] `git diff --stat` for that commit contains no file under any provider adapter crate and no `di.rs`.
- [ ] An integration test routes a chat completion through the descriptor path, using recorded fixtures or an `NAN_API_KEY`-gated live test.
- [ ] `/v1/models` lists NaN models with declared capabilities.
- [ ] This issue is the acceptance gate for the whole descriptor phase.

#### Out of scope

Live network calls in CI.

---

## Phase 2 — Selection engine

Where "intelligent" actually lives. Depends on Phase 1 for a clean candidate set.

### ROOK-P2-1 · \[ROOK\]\[P1\] Add the eligibility stage with reason codes

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| High     | `product/rook` `type/feature` `area/runtime` | M    | ROOK-P1-3  |

#### Context

Candidate filtering exists but is implicit and unnamed: `available_providers` (`router_impl.rs:224-240`) chains guards over registry iteration order, and each guard returns a boolean with no explanation.

#### Scope

Introduce an explicit first stage that produces both the eligible set and the rejected set:

```rust
struct Rejection {
    target: TargetRef,
    reason: RejectionReason,
}

enum RejectionReason {
    AuthMissing,
    AuthExpired,
    ModelUnsupported,
    CapabilityMissing,
    CircuitOpen,
    QuotaExhausted,
    KeyModelNotAllowed,
    KeyProviderNotAllowed,
    ConnectionDisabled,
}
```

Hard filters: authentication validity, model support, capability ceiling, circuit state, quota exhaustion, key allowlists, connection enabled.

#### Acceptance criteria

- [ ] Every candidate that is not selected is accompanied by exactly one rejection reason.
- [ ] No scoring or weighting input can reintroduce a rejected candidate.
- [ ] Reason codes are a closed enum, serializable and stable for consumers.
- [ ] Tests cover each rejection reason at least once.

#### Out of scope

Scoring and ranking (ROOK-P2-2).

### ROOK-P2-2 · \[ROOK\]\[P1\] Add the scoring stage with Signals and SelectionReason

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| High     | `product/rook` `type/feature` `area/runtime` | M    | ROOK-P2-1  |

#### Context

There is no scoring anywhere. Ordering is provider registration order plus `priority: u8`; `Priority` is literally `candidates.first()` (`router_impl.rs:361,402`), and `select_weighted` (`:242-258`) silently falls back to the first candidate when weights do not match the candidate count (`:248-250`).

The signals needed for scoring already exist in separate systems: telemetry p50/p95, `QuotaObservation`, circuit-breaker state, catalog cost and connection priority.

#### Scope

- One `Signals` struct gathered once per selection from existing sources.
- One score function producing bounded, comparable components per candidate.
- Deterministic tie-breakers (stable target id), so identical inputs always produce the same order.
- A `SelectionReason` value carrying the winning target plus its score components.

#### Acceptance criteria

- [ ] Selection is deterministic and reproducible for identical inputs.
- [ ] A weight and candidate-count mismatch can no longer silently degrade to "first candidate"; it is a hard error.
- [ ] Score components are inspectable per candidate.
- [ ] Unknown signal values are represented as unknown, never as zero.
- [ ] Tests pin the ranking for a fixed signal set.

#### Out of scope

Scoring quality tuning and machine-learned ranking.

### ROOK-P2-3 · \[ROOK\]\[P1\] Replace RoutingStrategy with six policies

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| High     | `product/rook` `type/feature` `area/runtime` | M    | ROOK-P2-2  |

#### Context

`RoutingStrategy` (`router_impl.rs:28-34`) is an enum matched at `:359-373` and `:400-413`, with weight extraction at `:218-222`. It is not configurable beyond `priority` and carries no notion of a policy object.

#### Scope

- Replace the enum with a policy set: `priority`, `weighted`, `round-robin`, `least-latency`, `least-cost`, `auto`.
- Each policy supplies weights or a comparator over the scored candidates from the previous issue; none of them re-implements eligibility.
- `auto` composes the others from request context and current signals. No learning state, no adaptation tables, no shadow routing.
- Make every policy configurable and document each one.

#### Acceptance criteria

- [ ] All six policies are selectable from configuration and covered by tests.
- [ ] `auto` resolves to a concrete policy per request, and the resolved policy appears in `SelectionReason`.
- [ ] Documentation describes each policy, its required signals and its tie-breaker.
- [ ] Existing `priority` behavior is preserved exactly.

#### Out of scope

Route preview (ROOK-P3-1) and any adaptive or learned selection.

### ROOK-P2-4 · \[ROOK\]\[P2\] Make combo strategy real

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| Medium   | `product/rook` `type/feature` `area/runtime` | S–M  | ROOK-P2-3  |

#### Context

`ComboStrategy` (`crates/domain/rook-core/src/model.rs:621-627`) has a single variant, `Priority`. The `strategy` field is persisted (`combo-sqlite/src/repository.rs:537-552`), exposed in DTOs (`combo_dto.rs:140-156`) and validated at config parse (`apps/rook/src/config.rs:258-266`; `di.rs:713-727` rejects anything but `"priority"`), but it is never read during execution. `route_request.rs:1726-1727` and `combo_stream.rs:251-252` call `sorted_steps()`, a single `sort_by_key(priority)` (`model.rs:813-817`).

A combo's declared strategy therefore has no effect on which step runs first.

#### Scope

- Delete `sorted_steps()` as the ordering authority.
- Route combo step ordering through the same selection engine as provider routing, so a combo strategy and a routing policy are the same concept.
- Expand the accepted strategy values to the policy set.

#### Acceptance criteria

- [ ] Changing a combo's strategy changes its execution order, provable by test.
- [ ] `priority` combos behave exactly as today.
- [ ] An unsupported strategy is rejected at write time with a clear error, not at request time.
- [ ] Streaming uses the same ordering as non-streaming.

#### Out of scope

New step fields (ROOK-P2-5).

### ROOK-P2-5 · \[ROOK\]\[P2\] Extend ComboStep with weight and a target union

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| Medium   | `product/rook` `type/feature` `area/storage` | M    | ROOK-P2-4  |

#### Context

`ComboStep` (`model.rs:630-643`) is `{ provider_id, model, connection_id: Option, priority: u8 }`. It can only express one model on one provider, so a combo cannot express "any connection of provider X", cannot weight steps, and cannot reference another combo.

#### Scope

- Add `weight: Option<u8>` (0-100), ignored by policies that do not use weights.
- Replace the flat fields with `target: ComboTarget`, where `ComboTarget` is `Model { provider_id, model, connection_id? } | Combo { combo_id } | ProviderWildcard { provider_id, model }`.
- Persist, validate, expose in DTOs and accept in configuration.
- Keep legacy rows loading as `Model` targets.

#### Acceptance criteria

- [ ] Existing combos load and execute unchanged.
- [ ] A weighted combo produces a weighted distribution across steps, covered by a statistical test with a fixed seed.
- [ ] A provider wildcard step resolves to an eligible connection at selection time, not at write time.
- [ ] Validation rejects a step referencing a nonexistent combo or provider.

#### Out of scope

Executing nested combos (ROOK-P2-6). This issue only permits referencing them.

### ROOK-P2-6 · \[ROOK\]\[P2\] Support nested combos with bounded depth

| Priority | Labels                                       | Size | Depends on |
|----------|----------------------------------------------|------|------------|
| Medium   | `product/rook` `type/feature` `area/runtime` | S–M  | ROOK-P2-5  |

#### Context

Nested combos let an operator compose premium, cheap and free chains from smaller chains instead of duplicating steps. They are only safe with an explicit depth bound and cycle detection.

#### Scope

- Resolve `ComboTarget::Combo` references at selection time.
- Bound recursion depth and detect cycles.
- Preserve the existing per-attempt (10 s) and total (60 s) combo budgets across the whole expanded chain, not per level.

#### Acceptance criteria

- [ ] A cycle is rejected at write time with a clear error.
- [ ] A depth beyond the configured maximum is rejected at write time.
- [ ] A nested chain respects the total time budget.
- [ ] Streaming commits to exactly one target and never switches after the first content chunk, as today.

#### Out of scope

Fan-out or parallel execution of combo targets.

---

## Phase 3 — Explainability surface

### ROOK-P3-1 · \[ROOK\]\[P2\] Expose POST /api/route/preview

| Priority | Labels                                         | Size | Depends on |
|----------|------------------------------------------------|------|------------|
| Medium   | `product/rook` `type/feature` `area/transport` | S–M  | ROOK-P2-3  |

#### Context

Route selection is opaque from outside. The signals needed to explain it (health, telemetry, quota, cost, eligibility) all exist once the eligibility and scoring stages land.

#### Scope

- `POST /api/route/preview`, admin-authorized.
- Accept a representative request, a requested model or combo alias, and selection context. Perform no inference.
- Return ordered candidates, effective connection ids, eligibility decisions, score components and bounded reason codes.
- Use consistent snapshots of health, quota and metric input, and mark unknown signals as unknown.

#### Acceptance criteria

- [ ] The endpoint never calls an upstream provider and never persists usage, cost or audit outcomes.
- [ ] Output uses the same selection semantics as execution, proven by a test that previews and then executes the same request and compares the chosen target.
- [ ] Unknown signals are reported as unknown, not as neutral values.
- [ ] Requires an admin scope; a non-admin key receives 403.
- [ ] The response is deterministic for identical inputs.

#### Out of scope

Simulating an upstream response body.

### ROOK-P3-2 · \[ROOK\]\[P2\] Record bounded, redacted selection reasons in telemetry

| Priority | Labels                                             | Size | Depends on |
|----------|----------------------------------------------------|------|------------|
| Medium   | `product/rook` `type/feature` `area/observability` | S    | ROOK-P2-2  |

#### Context

Once selection produces a `SelectionReason`, it should be observable. Today telemetry records latency and usage per provider but nothing about why a target was chosen.

#### Scope

- Record the resolved policy, the winner, and the top alternatives with their rejection or score reason.
- Bound cardinality and size; never include prompts, message content or credentials.

#### Acceptance criteria

- [ ] A completed request can be traced to the policy, the chosen target and the top rejected alternatives.
- [ ] The payload is size-bounded and contains no request content.
- [ ] Cardinality is bounded: reason codes are enum values, never free text.
- [ ] Existing telemetry endpoints keep their current shape.

#### Out of scope

An analytics UI for selection reasons.

---

## Phase 4 — Client surface completion

### ROOK-P4-1 · \[ROOK\]\[P2\] Support /v1/embeddings end to end

| Priority | Labels                                         | Size | Depends on |
|----------|------------------------------------------------|------|------------|
| Medium   | `product/rook` `type/feature` `area/transport` | S–M  | ROOK-P1-2  |

#### Context

Rook's client surface registers chat completions, messages, responses and models only. Embedding requests cannot be served, which excludes an entire workload class from the gateway.

#### Scope

- OpenAI-compatible `POST /v1/embeddings` for supported text input.
- Route only to models declared embedding-capable in the catalog.
- Enforce key model and provider allowlists and existing scopes.
- Return stable vectors, input index order, model id and usage.
- Attribute usage and failures to the effective provider, connection and client key.

#### Acceptance criteria

- [ ] A valid request returns vectors in input order with correct usage.
- [ ] A non-embedding model is rejected with a clear error before any upstream call.
- [ ] Key allowlists and scopes apply exactly as for chat.
- [ ] A provider that does not support embeddings is skipped by eligibility, not failed at runtime.
- [ ] NaN's `qwen3-embedding` is usable as a real integration target.

#### Out of scope

Rerank API, vector storage and RAG orchestration.

### ROOK-P4-2 · \[ROOK\]\[P2\] Add Gemini-compatible inbound ingress

| Priority | Labels                                         | Size | Depends on           |
|----------|------------------------------------------------|------|----------------------|
| Medium   | `product/rook` `type/feature` `area/transport` | M    | ROOK-P0-1, ROOK-P1-2 |

#### Context

Rook exposes OpenAI and Anthropic inbound dialects but no Gemini dialect. A Google SDK, or any tool configured for `generativelanguage`, cannot reach Rook at all.

#### Scope

- `POST /v1beta/models/{model}:generateContent` and `:streamGenerateContent`.
- One request DTO to domain conversion and one response conversion, following the pattern used for Anthropic (`anthropic_adapter.rs`).
- Apply existing auth, scopes, allowlists, combos and telemetry without special cases.

#### Acceptance criteria

- [ ] A Gemini-format request reaches an OpenAI-dialect provider and returns a valid Gemini-format response.
- [ ] Streaming emits valid Gemini SSE events, including terminal and error events.
- [ ] Tool calls survive the round trip with their identities intact.
- [ ] Unsupported Gemini features are rejected explicitly rather than silently dropped.
- [ ] No format-registry layer is reintroduced: conversion is the DTO pair plus the adapter.

#### Out of scope

Vertex AI protocol and service-account authentication.

---

## Phase 5 — Authentication breadth

### ROOK-P5-1 · \[ROOK\]\[P2\] Extract an AuthDriver boundary

| Priority | Labels                                    | Size | Depends on |
|----------|-------------------------------------------|------|------------|
| Medium   | `product/rook` `type/feature` `area/auth` | M    | ROOK-P1-1  |

#### Context

Authentication today is `AuthType::{ApiKey, OAuth}` (`provider_connection.rs:5-9`), with OAuth logic spread across `manage_connections.rs:1579-1700` and `oauth_tokens.rs:193-230`, and each of those files re-derives provider specifics through `ProviderKind` matches.

#### Scope

- An `AuthDriver` trait: obtain credentials, refresh when applicable, report expiry, expose the auth headers a protocol adapter needs.
- Implementations: `ApiKeyDriver`, `OAuthDriver` (initially wrapping existing code unchanged).
- Reserve `ImportedTokenDriver` as a declared but unimplemented variant, gated by DALLAY-692.

#### Acceptance criteria

- [ ] Adapters receive credentials from a driver and contain no auth-mechanism branching.
- [ ] Adding an auth mechanism requires no adapter change.
- [ ] Existing API key and OAuth connections behave identically.
- [ ] Token material never appears in logs, telemetry or error messages.

#### Out of scope

New auth mechanisms.

### ROOK-P5-2 · \[ROOK\]\[P2\] Consolidate OAuth into one driver

| Priority | Labels                                    | Size | Depends on |
|----------|-------------------------------------------|------|------------|
| Medium   | `product/rook` `type/feature` `area/auth` | M    | ROOK-P5-1  |

#### Context

OAuth handling is duplicated per provider: scopes at `manage_connections.rs:1579-1594`, authorize and token endpoints at `:1610-1645`, authorize URL at `:1680-1700`, token lifecycle at `oauth_tokens.rs:193-230`. None of these consult a single declaration.

#### Scope

- One OAuth driver parameterized by descriptor-supplied scopes, endpoints and PKCE requirements.
- Preserve Azure's stricter requirements: OAuth mandatory, HTTPS endpoint, deployment name (`provider_connection.rs:213-219`).

#### Acceptance criteria

- [ ] One OAuth code path serves every OAuth-capable provider.
- [ ] Azure OpenAI keeps its mandatory-OAuth and HTTPS-endpoint validation.
- [ ] Refresh behavior and token caching are unchanged for existing connections.
- [ ] Existing OAuth tests pass without rewrite.

#### Out of scope

New OAuth providers.

### ROOK-P5-3 · \[ROOK\]\[P3\] Imported local-token driver

| Priority | Labels                                    | Size | Depends on            |
|----------|-------------------------------------------|------|-----------------------|
| Low      | `product/rook` `type/feature` `area/auth` | M    | ROOK-P5-1, DALLAY-692 |

#### Context

Some providers authenticate with credentials that already exist on the developer's machine, from a CLI login or an IDE session. The descriptor table already reserves `AuthKind::ImportedToken`, but building it before the contract question is settled would mean assuming consumer subscriptions are reusable as gateway credentials.

DALLAY-692 is the spike that settles this. It is explicitly not permission to scrape browser cookies or copy CLI tokens.

#### Scope

Only after DALLAY-692 returns a permitted contract for a specific provider:

- An `AuthDriver` that reads a documented local credential location for that provider.
- Redaction guarantees and a clear "not configured" failure mode.

#### Acceptance criteria

- [ ] Blocked until DALLAY-692 records a permitted contract for the provider in question.
- [ ] Credentials are never logged, copied into Rook's database, or exported through the API.
- [ ] A missing or expired local credential produces an actionable error, not a crash.
- [ ] Each provider requires its own issue; this one does not generalise across vendors.

#### Out of scope

Browser cookie scraping, CLI token extraction, and anything DALLAY-692 does not explicitly permit.

---

## Existing issues affected

| Existing issue                                                               | Disposition                                                                                              |
|------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------|
| DALLAY-712 — Consolidate the three TokenBucket implementations               | Absorbed by ROOK-P0-3; close as superseded                                                               |
| DALLAY-684 — Explainable cost, latency and weighted combo selection policies | Absorbed by ROOK-P2-2 and ROOK-P2-3; close as superseded                                                 |
| DALLAY-688 — Deterministic route preview                                     | Absorbed by ROOK-P3-1; close as superseded                                                               |
| DALLAY-689 — Authorized embeddings requests                                  | Absorbed by ROOK-P4-1; close as superseded                                                               |
| DALLAY-690 — Native AWS Bedrock with SigV4                                   | Kept open. Becomes a Phase 6 issue: one adapter plus one descriptor, landing after Phase 1 and ROOK-P5-1 |
| DALLAY-691 — Google Vertex AI with service authentication                    | Kept open, same disposition as DALLAY-690                                                                |
| DALLAY-692 — Validate authorized OAuth and subscription paths                | Kept open. Gates ROOK-P5-3 and any imported-token work                                                   |
| DALLAY-392 — Kiro provider integration                                       | Kept open. Blocked on its upstream contract question, unrelated to this roadmap                          |

## Import procedure

The Linear workspace is at its free issue limit, so creation failed with `400 invalid_request` before any issue was written. Once capacity exists:

1. Create every entry above under team `DALLAY`, project `cortex`.
2. Use the heading suffix as the title, for example `[ROOK][P1] Delete the dead format-translation layer`.
3. Map priority: P1 to High, P2 to Medium, P3 to Low.
4. Apply the labels listed in each issue's metadata table, and add `product/rook`.
5. Recreate the `Depends on` edges as `blockedBy` relations.
6. After creation, close DALLAY-712, 684, 688 and 689 as superseded, each pointing at its replacement.
7. Leave the deprecation comments on DALLAY-690, 691, 692 and 392 noting their new phase.

Cross-reference ids are stable: `ROOK-P<phase>-<n>` appears in the body text of dependent issues, so a search for `ROOK-P1-3` finds both the issue and everything that waits on it.
