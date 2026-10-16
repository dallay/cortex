# Huginn provider OAuth support

## Decision and blocker

As of **October 16, 2026**, no eligible interactive OAuth flow for a local
third-party Huginn client has been established for either built-in inference
target. **OAuth is unsupported in Huginn**, not an alternative login method.
This delivery reports that limitation; it does not implement OAuth.

`/login` and `/providers` report unsupported OAuth separately from available
API-key/no-auth methods. An adapter declaring only `AuthenticationMethod::OAuth`
is present but has no supported login method: it must not be reported as an
absent adapter. The enum is descriptive, not authorization to start a flow.
There are no OAuth choices, token fields, browser/device actions, token exchange,
refresh, or OAuth credential persistence in this UI. Existing API-key and local
forms remain available. Connection readiness is local, not proof of remote access.

The blocker is missing official evidence authorizing an interactive local
third-party client for the exact inference API, including registration and scopes.
Do not substitute consumer tokens, CLI credential extraction, or a synthetic
authorization server for that evidence.

## Per-target support matrix

### `openai` — OpenAI-compatible inference

- **Current Huginn methods:** API key or explicitly unauthenticated endpoint,
  according to the existing adapter descriptor. Endpoint/model validation and
  loopback HTTP opt-in remain unchanged.
- **Official API relationship:** OpenAI's API overview accepts API keys **or
  workload identity federation (WIF) access tokens**. ChatGPT subscriptions and
  API billing are separate; subscription access does not establish API access.
- **Scopes and registration:** No scopes or public-client registration contract
  for interactive local user authorization was established by this research.
  WIF instead requires administrator-configured issuer/service-account mapping
  and an external workload identity; it does not supply a refresh token.
- **Local availability:** WIF is not evidence of a public interactive local user
  OAuth flow. Huginn does not implement WIF provisioning or token exchange.
  This is not a claim that OpenAI has no OAuth or token exchange whatsoever.
- **Compatible endpoints:** OpenAI-compatible wire format does not inherit
  OpenAI authorization rights. Every vendor requires its own official evidence
  for the exact endpoint, client type, scopes, and registration.
- **Support decision:** Interactive OAuth unsupported; retain existing API-key
  and no-auth forms, without promising access to any vendor's models.

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

## Official research evidence

The following official sources were retrieved on **October 16, 2026**. Research
evidence is separate from automated tests: UI doubles do not validate provider
authorization, real accounts, OS secret stores, or remote revocation.

- [OpenAI API overview](https://developers.openai.com/api/reference/overview):
  API keys and workload identity federation access tokens.
- [OpenAI workload identity federation](https://developers.openai.com/api/docs/guides/workload-identity-federation):
  administrator configuration, external identity exchange, and no refresh token.
- [ChatGPT subscription and API billing](https://help.openai.com/en/articles/8156019-how-can-i-move-my-chatgpt-subscription-to-the-api):
  separate products and billing.
- [Codex authentication](https://developers.openai.com/codex/auth):
  Codex sign-in is product-specific; general API calls use Platform credentials.
- [Ollama authentication](https://docs.ollama.com/api/authentication):
  unauthenticated local API, direct cloud API keys, and CLI sign-in.

## Revocation and future gates

`/logout` revokes local connection authorization and removes stored credentials.
It does not revoke a provider API key or establish remote token revocation.
Cancelling local work does not prove that remote execution or billing stopped.
Revoke remote credentials through the provider's documented controls.

Before enabling OAuth for any target, require:

1. Official permission for the exact inference API and local third-party client,
   including scopes, registration, billing relationship, and redirect/device rules.
2. A reviewed threat model and explicit token storage, expiry, refresh (if offered),
   cancellation, and local-versus-remote revocation semantics.
3. Regression coverage proving unsupported attempts cannot create ready
   connections or persist secrets, and preserving connection isolation and turn
   pinning through authorization changes.
4. Real-provider acceptance distinct from deterministic UI/contract fixtures.
   A test server alone cannot establish provider eligibility.
