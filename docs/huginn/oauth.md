# Huginn provider OAuth support

## Decision and blocker

As of **November 26, 2026**, OpenAI documents Sign in with ChatGPT (SIWC) as an
optional way for open-source and locally hosted third-party apps to request
permission for eligible Responses API requests to use the user's ChatGPT plan.
This establishes a documented flow for that app category; it does not confirm
Huginn specifically, a particular account or workspace, or successful operation.

Huginn appears to fit the documented category because it is open-source and
locally executed. That is an inference, not an OpenAI eligibility decision.
As of **November 27, 2026**, Huginn implements the dedicated local-only SIWC
vertical slice (`chatgpt-siwc`) behind explicit user consent in `/login`,
separate from the OpenAI Platform API-key / Chat Completions route and from
Ollama:

- **Implemented first slice (`chatgpt-siwc`):** single local account, browser
  PKCE (`S256`) + `127.0.0.1` loopback callback, `client_id=dynamic_agent_client`
  registration and issued `client_id` reuse, required scope validation, strict
  `OsSecrets` persistence (no silent ephemeral downgrade), single-flight refresh
  rotation, OIDC-discovered remote revocation reporting on `/logout`, model
  discovery via `GET /v1/models` (`visibility == "list"`, `slug`), and text-only
  streaming inference via `POST /v1/responses` (`store: false`, `stream: true`).
- **Still unsupported:** OAuth on OpenAI-compatible Chat Completions routes,
  Ollama, multi-account SIWC, and tools/images over SIWC Responses API.
- **Verification boundary:** deterministic local mock-server tests verify PKCE,
  token rotation, `/v1/models`, `/v1/responses` SSE, and OIDC revocation; real
  OpenAI account/workspace eligibility still requires separate manual acceptance
  with a live ChatGPT account.

`/login` and `/providers` continue to report unsupported OAuth separately on
adapters that do not implement SIWC (such as `openai`, `ollama`, and
`ollama-cloud`). An adapter declaring only `AuthenticationMethod::OAuth` other
than `chatgpt-siwc` has no supported login method and must not be reported as an
absent adapter. Existing API-key and local forms remain available. Connection
readiness is local, not proof of remote access.

The remaining validation gate is real-provider manual acceptance with a live
ChatGPT account, plus the lack of OpenAI confirmation for Huginn specifically
or any particular account/workspace. The documented flow is for Responses API;
it does not authorize adding OAuth to Huginn's Chat Completions-compatible
route. Do not substitute consumer tokens, CLI credential extraction, or a
synthetic authorization server.

## Per-target support matrix

### `openai` — OpenAI-compatible inference

- **Current Huginn methods:** API key or explicitly unauthenticated endpoint,
  according to the existing adapter descriptor. Endpoint/model validation and
  loopback HTTP opt-in remain unchanged.
- **Separate API authentication:** [OpenAI's API overview](https://developers.openai.com/api/reference/overview)
  describes Platform API keys and workload identity federation (WIF), which are
  distinct from SIWC. The [WIF guide](https://developers.openai.com/api/docs/guides/workload-identity-federation)
  requires administrator-configured issuer/service-account mapping and an
  external workload identity; it is not a local end-user sign-in flow. ChatGPT
  plan usage does not establish API-key access, and ChatGPT and API billing are
  separate. These API/WIF references were last reviewed on October 16, 2026.
- **Documented SIWC registration:** The official guide describes a public-client
  flow for open-source/local apps using browser authorization, PKCE, and an HTTP
  loopback callback. It defines identity scopes (`openid profile email`) and
  plan-use scopes (`offline_access resource.invoke
  chatgpt.tokens.use.direct`). A new registration starts with
  `client_id=dynamic_agent_client`; the client saves and reuses the issued
  `client_id` for that account. The guide does not require a client secret.
- **SIWC inference boundary:** The guide uses `GET /v1/models` for model choices
  and `POST /v1/responses` for eligible inference. It is not a general OAuth
  grant for OpenAI-compatible Chat Completions endpoints.
- **Local availability:** The documented category includes open-source and
  locally hosted apps. Huginn's category fit is inferred from its MIT license
  and local execution; Huginn has not been specifically approved or tested with
  a live account. SIWC registration, token exchange, and text-only Responses API
  inference are implemented separately in the dedicated `chatgpt-siwc` adapter,
  not on the `openai` Chat Completions adapter.
- **Compatible endpoints:** OpenAI-compatible wire format does not inherit
  OpenAI authorization rights. Every vendor requires its own official evidence
  for the exact endpoint, client type, scopes, and registration.
- **Support decision:** Keep OAuth unsupported on the `openai` Chat Completions
  adapter and retain the existing API-key and no-auth forms without promising
  access to any vendor's models. Use the dedicated `chatgpt-siwc` adapter for
  SIWC over Responses API, with zero silent fallback to API-key billing.

### `ollama` — local inference only

- **Current Huginn methods:** The descriptor offers no authentication and an
  API-key form; official local Ollama requires no authentication. The built-in
  adapter is local-only, not an Ollama cloud adapter.
- **Official API relationship:** Ollama documents local API use without
  authentication and API keys for direct cloud API calls.
- **Scopes and registration:** No third-party interactive local OAuth scopes or
  client registration contract was established. `ollama signin` belongs to the
  Ollama CLI; it is not a third-party authorization contract for Huginn.
- **Local availability:** Use the existing local endpoint/model form, with
  explicit opt-in for HTTP on literal loopback. No sign-in is required or offered.
- **Support decision:** Interactive OAuth unsupported; retain local inference.
  Cloud authorization and CLI credential reuse are outside this target.

## Sign in with ChatGPT evidence

The following OpenAI SIWC sources were reviewed on **November 26, 2026**. The
pages did not show publication or update dates.

- [SIWC overview for open-source and locally hosted apps](https://developers.openai.com/siwc/token-sharing-open-source): OpenAI says an open-source app can request permission to use the user's ChatGPT plan for eligible Responses API requests. The same page directs paid or remotely hosted apps to an interest form. This supports the category-level conclusion, not an approval of Huginn.
- [SIWC registration and sign-in](https://developers.openai.com/siwc/token-sharing-open-source/sign-in): documents `client_id=dynamic_agent_client` for initial registration, an app-name hint, a persistent host identifier, an issued `client_id` to save and reuse, authorization code with PKCE, and a loopback callback on `127.0.0.1`. It lists the scopes and documents the authorize and token endpoints below. It does not require a client secret.
- [SIWC models and inference](https://developers.openai.com/siwc/token-sharing-open-source/models-and-inference): documents `GET https://api.openai.com/v1/models`, filtering displayable entries by `visibility == "list"` and passing the selected `slug` as the model. Inference uses `POST https://api.openai.com/v1/responses` with `store: false` and `stream: true`. It says not to use ChatGPT `backend-api` endpoints. A model listing is not proof of account entitlement.
- [SIWC token reference](https://developers.openai.com/siwc/token-sharing-open-source/token-reference): access tokens last one hour (`expires_in: 3600`); refresh tokens last 30 days and rotate on successful refresh. The documentation states there is no fixed limit on successive replacements while each token remains valid.
- [SIWC accounts and sessions](https://developers.openai.com/siwc/token-sharing-open-source/profiles-and-sessions): says to discover the revocation endpoint from OIDC metadata. Revocation ends the renewable session, not the registered client; if remote revocation cannot be confirmed, local token deletion must not be described as confirmed remote revocation.
- [OpenAI OIDC discovery metadata](https://auth.openai.com/.well-known/openid-configuration): the `revocation_endpoint` returned on November 26, 2026 was `https://auth.openai.com/api/accounts/oauth/revoke`. Discover this value at runtime instead of treating the observed endpoint as an immutable contract.
- [OpenAI Help: using a ChatGPT plan in other apps and sites](https://help.openai.com/en/articles/20001542-using-your-chatgpt-plan-in-other-apps-and-sites): eligible requests may use the plan's included allowance or applicable credits, subject to plan and workspace controls. The app does not receive an API key or access to ChatGPT conversations or memories.
- [OpenAI Help: ChatGPT and API billing](https://help.openai.com/en/articles/9039756-managing-billing-for-chatgpt-and-the-api-platform): ChatGPT and API usage have separate billing systems.

The registration guide specifies these SIWC endpoints and scopes:

| Item | Documented value |
|---|---|
| Authorization | `https://auth.openai.com/api/accounts/authorize` |
| Token exchange and refresh | `https://auth.openai.com/api/accounts/oauth/token` |
| Identity scopes | `openid profile email` |
| Plan-use scopes | `offline_access resource.invoke chatgpt.tokens.use.direct` |

## Authentication boundaries

- **OpenAI API key:** Huginn's existing OpenAI-compatible provider sends the
  configured API key as a bearer credential and uses its current
  Chat Completions-compatible request path. API-key usage is billed through the
  OpenAI Platform, separately from a ChatGPT plan.
- **Sign in with ChatGPT:** a separate, optional consent flow for eligible
  Responses API requests using the user's ChatGPT plan. It does not provide an
  API key, is implemented in Huginn as the dedicated `chatgpt-siwc` vertical
  slice (text-only Responses API), and does not authorize use of the Chat
  Completions route.
- **Codex authentication:** [OpenAI's Codex authentication guide](https://developers.openai.com/codex/auth)
  describes sign-in for Codex clients and separate API-key authentication. A
  Codex login or session does not establish Huginn's authorization to use SIWC.
- **Rook Codex accounts:** Rook's implementation is product-specific and uses
  `https://auth.openai.com/oauth/authorize` and
  `https://auth.openai.com/oauth/token` in
  `crates/infrastructure/providers-openai/src/codex/oauth.rs`; its account
  surface is `crates/domain/rook-core/src/codex_accounts.rs`. These paths differ
  from SIWC's `/api/accounts/...` endpoints. Rook does not implement or certify
  SIWC for Huginn. Do not extract or reuse another client's sessions or tokens.
- **Ollama:** the [Ollama authentication guide](https://docs.ollama.com/api/authentication)
  describes unauthenticated local API access and separate cloud API-key/CLI
  authentication. Huginn's built-in adapter remains local-only.

## Evidence classification

- **Documented:** OpenAI publishes SIWC for open-source and locally hosted
  third-party apps, with registration, consent, scopes, token lifecycle,
  revocation, model discovery, Responses API, and plan-use rules.
- **Inferred:** Huginn appears to fit the open-source/local category based on
  its MIT license (`LICENSE`) and local execution. This does not establish
  OpenAI's specific approval or the eligibility of any account or workspace.
- **Not verified:** no Huginn login, account/workspace eligibility check, model
  entitlement check, or real-provider inference was performed. No credentials
  were used. The existing UI/contract tests do not certify external OAuth or
  native secret-store behavior.

## Implemented first vertical slice (`chatgpt-siwc`)

Huginn implements the dedicated, local-only SIWC vertical slice for a single
account and text-only streaming inference, kept strictly separate from the
API-key / Chat Completions route:

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:4bc58fa546a11531ea1c6fcdb8182ed3f44d837840571c0fcc2eef9ce50ebab9 -->

- **Authorization & registration:** each sign-in uses browser PKCE (`S256`), a
  literal `127.0.0.1` ephemeral loopback callback, the persistent host ID, and
  `client_id=dynamic_agent_client` registration. The issued `client_id` is used
  for code exchange and token refresh, and sent for remote revocation; it is not
  reused for later sign-ins. Huginn validates `state` and required scopes before
  marking the connection ready.
- **Secret storage:** the issued `client_id`, access token, rotating refresh
  token, and expiry are stored strictly in `OsSecrets` (macOS Keychain / Linux
  Secret Service), with no silent downgrade to ephemeral or plaintext storage.
  The host ID is included in the credential record and is also written as
  plaintext to the production `siwc_host_id` file beside the database.

<!-- /coderabbit-ai-generated:v1 -->
- **Model discovery & inference:** `GET https://api.openai.com/v1/models`
  filtered by `visibility == "list"` using `slug`, and `POST https://api.openai.com/v1/responses`
  with `store: false` and `stream: true`, turn pinning, usage accounting, local
  cancellation, and zero silent fallback to API-key billing.
- **Deferred follow-ups:** multiple simultaneous SIWC accounts, tool calling,
  multimodal/image inputs, reasoning effort over Responses API, hosted or
  commercial distribution, and Codex session reuse remain out of scope.

## Real-provider manual acceptance checklist

Deterministic local mock-server tests verify PKCE, token exchange/rotation,
`/v1/models`, `/v1/responses` SSE, and OIDC revocation, but cannot certify live
OpenAI account or workspace eligibility. Report real-provider runs separately
from deterministic fixture tests, recording OS, terminal, credential backend,
and date:

- [ ] **Sign-in (`/login` → `chatgpt-siwc`):** confirm consent screen, complete
  browser PKCE loopback callback on `127.0.0.1`, verify ready connection without
  affecting existing API-key or Ollama connections, and verify credentials are
  stored in `OsSecrets` (never in `sessions.db`, logs, or terminal scrollback).
- [ ] **Model discovery (`/model`):** list eligible models (`visibility == "list"`,
  using `slug`) and pin a model for the next turn.
- [ ] **Streamed text turn:** send a text prompt, verify streamed output via
  `POST /v1/responses` (`store: false`, `stream: true`), verify reported usage
  accounting, and verify cancellation and error handling without fallback to an
  OpenAI API key.
- [ ] **Token refresh rotation:** verify an expiring access token refreshes via
  `/api/accounts/oauth/token`, rotates the refresh token in `OsSecrets`, and
  fails closed if refresh is rejected.
- [ ] **Revocation (`/logout`):** confirm `/logout` calls the OIDC-discovered
  `revocation_endpoint`, clears local credentials, and explicitly reports
  whether remote revocation was confirmed or only local cleanup occurred.

## Revocation and future gates

`/logout` removes local connection authorization and stored credentials. For
API-key connections, it does not revoke a provider API key or prove remote
token revocation. For `chatgpt-siwc`, `/logout` calls the OIDC-discovered
`revocation_endpoint`, clears local credentials, and explicitly reports when
remote revocation was not confirmed. Cancelling local work does not prove that
remote execution or billing stopped.

Before enabling OAuth for any additional target, require:

1. A reviewed threat model and explicit token storage, expiry, refresh,
   cancellation, and local-versus-remote revocation semantics.
2. Regression coverage proving unsupported attempts cannot create ready
   connections or persist secrets, and preserving connection isolation and turn
   pinning through authorization changes.
3. Real-provider acceptance distinct from deterministic UI/contract fixtures.
   A test server alone cannot establish provider eligibility.
