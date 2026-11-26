# Huginn provider OAuth support

## Decision and blocker

As of **November 26, 2026**, OpenAI documents Sign in with ChatGPT (SIWC) as an
optional way for open-source and locally hosted third-party apps to request
permission for eligible Responses API requests to use the user's ChatGPT plan.
This establishes a documented flow for that app category; it does not confirm
Huginn specifically, a particular account or workspace, or successful operation.

Huginn appears to fit the documented category because it is open-source and
locally executed. That is an inference, not an OpenAI eligibility decision.
Huginn does not implement SIWC. **OAuth remains unsupported in Huginn**; this
document does not enable it.

`/login` and `/providers` report unsupported OAuth separately from available
API-key/no-auth methods. An adapter declaring only `AuthenticationMethod::OAuth`
is present but has no supported login method: it must not be reported as an
absent adapter. The enum is descriptive, not authorization to start a flow.
There are no OAuth choices, token fields, browser/device actions, token exchange,
refresh, or OAuth credential persistence in this UI. Existing API-key and local
forms remain available. Connection readiness is local, not proof of remote access.

The remaining blockers are product implementation and validation, plus the lack
of confirmation for Huginn or any specific account/workspace. The documented
flow is for Responses API; it does not authorize adding OAuth to Huginn's current
Chat Completions-compatible route. Do not substitute consumer tokens, CLI
credential extraction, or a synthetic authorization server.

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
  an account. No SIWC registration, token exchange, or inference is implemented.
- **Compatible endpoints:** OpenAI-compatible wire format does not inherit
  OpenAI authorization rights. Every vendor requires its own official evidence
  for the exact endpoint, client type, scopes, and registration.
- **Support decision:** SIWC is documented for the open-source/local app
  category, but OAuth is unsupported in Huginn until a separate implementation
  and authorized validation are completed. Retain the existing API-key and
  no-auth forms without promising access to any vendor's models.

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
  API key, is not implemented in Huginn, and does not authorize use of the
  current Chat Completions route.
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

## Separate first implementation slice proposal

If the project later authorizes implementation, start with a dedicated,
local-only SIWC vertical slice for one account and text-only streaming inference.
Keep it separate from the API-key/Chat Completions route. The slice would cover
browser registration and PKCE loopback callback, validation of identity and
granted scopes, safe persistence of registration identifiers and rotating
tokens, refresh and remote revocation, and one Responses API request with
`store: false` and `stream: true`. Do not silently fall back to API-key billing.
Exclude hosted/commercial distribution, multiple accounts, images, tools, and
Codex session reuse from this first slice.

Decisions required before implementation:

1. Confirm the intended distribution and whether the app remains within the
   documented open-source/local category; hosted or paid distribution needs a
   separate OpenAI eligibility decision.
2. Decide the user-facing name, explicit opt-in/consent flow, and whether the
   first slice supports one account only.
3. Decide the secret-store contract and backend for the stable host ID,
   per-account issued client ID, and rotating tokens.
4. Decide model selection behavior, including account-specific discovery,
   unavailable/empty catalogs, and whether manual model entry is allowed.
5. Define Responses API mapping, streaming, cancellation, usage/errors, and
   workspace/credit-limit feedback for the first text-only scope.
6. Authorize a real-provider acceptance test and define its account/workspace
   controls; deterministic fixtures alone cannot verify eligibility.

## Revocation and future gates

`/logout` removes local connection authorization and stored credentials. It does
not revoke a provider API key or prove remote token revocation. A future SIWC
implementation must call the currently discovered remote revocation endpoint
and report when remote revocation was not confirmed. Cancelling local work does
not prove that remote execution or billing stopped.

Before enabling OAuth for any target, require:

1. A reviewed threat model and explicit token storage, expiry, refresh,
   cancellation, and local-versus-remote revocation semantics.
2. Regression coverage proving unsupported attempts cannot create ready
   connections or persist secrets, and preserving connection isolation and turn
   pinning through authorization changes.
3. Real-provider acceptance distinct from deterministic UI/contract fixtures.
   A test server alone cannot establish provider eligibility.
