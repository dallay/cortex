# Huginn inference adapters and local verification

## Boundary and transport

`BuiltinInferencePlugin` registers `openai` and `ollama` in the existing provider
catalog. It publishes no service: `ProvidersPlugin` remains the single
`agent:model@1` publisher. Compose the core with `ConnectionAccess` credentials,
activate the adapter plugin, then open `ConnectionManager`. The adapter receives
only the pinned `ConnectionMaterial`; it never reads environment credentials.

Every endpoint requires validated HTTPS, except when that connection explicitly
sets `allow_loopback_http = true` for a literal loopback IP. Neither DNS names
(including `localhost`) nor private/LAN addresses qualify. Local HTTP bypasses
environment proxies. URL credentials, query strings, fragments, zero timeouts,
and remote HTTP are rejected. Certificate validation is never disabled.
Redirects, including same-origin and HTTPS-to-HTTP redirects, are rejected.

The OpenAI-compatible endpoint includes its API prefix, for example
`https://api.example.com/v1`. Local Ollama uses a root endpoint, for example
`http://127.0.0.1:11434`, with explicit HTTP opt-in. The latter is deliberately
local-only, not an Ollama cloud adapter.

## Discovery, capabilities, and failures

OpenAI-compatible discovery uses `GET /models`; Ollama discovery uses
`GET /api/tags`, then `POST /api/show` for the selected model. Static descriptor
model lists are empty for these connection-scoped adapters. A stored selection
is only a candidate: turn pinning must validate the exact model against the
connection's live catalog. It never selects the first model or another endpoint.
An explicit router selection can choose another catalog model on the same
connection; the stored connection model is a default, not a replacement for the
turn's pinned choice.

`ModelCatalog` explicitly distinguishes `Available` (including an empty list),
`Unsupported`, `Unavailable`, and `Invalid`. No absent or failed catalog permits
inference. Discovery is bounded by a total timeout, 1 MiB, and 1,024 models.
Errors do not include remote bodies, URLs, credentials, or remote finish reasons.
Successful model text/tool arguments are content, not sanitized diagnostic data;
an untrusted model can still generate sensitive content supplied in its prompt.

An OpenAI-format model listing does not establish tool support: discovery reports
`ToolSupport::Unknown` and the remote model may explicitly reject a tool request.
For Ollama, `/api/show` must advertise `completion`; missing capability metadata
fails pinning. Absence of `tools` causes a tool-bearing request to fail before
inference, with no fallback. Advertised support is not proof of model quality:
the real-server probe below tests usable calls and result continuation.

Ollama uses the documented OpenAI-compatible `/v1/chat/completions` SSE transport,
not a guessed native NDJSON translation. Both adapters share Huginn's fragmented
text/tool parser, finish validation, tool-result serialization, and sanitized
transport errors. Reasoning fields are currently ignored. A model can consume its
output budget reasoning without producing usable text, and a `length` finish
fails explicitly rather than returning a successful partial result.

The router keeps connection/model/generation fixed across tool continuation and
compaction. Selection changes affect subsequent turns only. Manager revocation
invalidates later inference within an already pinned turn. Dropping/cancelling
an in-flight stream is best effort and local only: it does **not** confirm remote
cancellation, credential revocation, or that billing stopped.

## Reproducible real Ollama checks

Verified locally on October 14, 2026, using Ollama client/server **0.40.3**, macOS,
the existing installed GGUF models, and loopback HTTP without credentials. No
model downloads or paid credentials were used. These observations do not certify
other versions, quantizations, templates, or remote servers.

Start a local server and inspect its actual inventory:

```bash
OLLAMA_HOST=127.0.0.1:11434 ollama serve
curl http://127.0.0.1:11434/api/version
curl http://127.0.0.1:11434/api/tags
cargo run -p huginn-runtime --example verify_ollama -- \
  qcwind/qwen3-8b-instruct-Q4-K-M:latest
```

The probe invokes the actual Huginn adapter, validates the installed model and
capabilities, then checks streamed text/completion, a streamed `list_files` call
with parsed JSON arguments, an actual read-only listing of a temporary workspace,
and continuation that names `verified_fixture.rs` from the tool result. It does
not execute generated shell commands or write into the user's repository.

### Verified coding-flow model

- `qcwind/qwen3-8b-instruct-Q4-K-M:latest`, 8.2B, Q4_K_M.
- Installed digest:
  `500a1f067a9f782620b40bee6f7b0c89e17ae61f686b92c24933e4ca4b2b8b41`.
- `/api/show`: `completion`, `tools`, `thinking`.
- Probe: **PASS** streamed text/completion, tool call/JSON, local tool execution,
  and tool-result continuation.
- Limitation: a preliminary 64-token request without `/no_think` exhausted its
  budget in reasoning (`finish_reason: length`) with no usable text. The passing
  probe uses `/no_think` and 1,024/2,048-token budgets. Those hints are probe
  prompts, not adapter-injected behavior or a guarantee for arbitrary prompts.

### Model not verified for the coding flow

- `qwen2.5-coder:14b`, 14.8B, Q4_K_M.
- Installed digest:
  `9ec8897f747e246e970bc5cfdda85d22f1123dc2e3d34978a010a75968716849`.
- Inventory advertises `completion`, `tools`, `insert`.
- Same probe: streamed text/completion **PASS**; tool stage **FAIL** with
  `probe expected one valid list_files tool call`. Continuation was not run.
- This combination is **not certified** for Huginn's coding flow despite its
  advertised capabilities. The adapter does not silently substitute a model.

## Deterministic evidence and remaining integration

Run independently of Ollama or paid credentials:

```bash
cargo test -p huginn-runtime
cargo test -p huginn-application
```

Controlled fixtures cover fragmented SSE/tools, catalog failure/absence, missing
models, unsupported tools, bounded waits/cancellation, real router pinning and
connection-manager revocation, cross-origin redirects, authorization capture,
untrusted TLS, and trusted-fixture HTTPS downgrade rejection. Fixtures are not
evidence of compatibility with any actual Ollama model.

The headless registration API is available; the existing CLI factory remains a
legacy compatibility path rather than a second model publisher. Automated full
Providers UI/CLI composition and recovery belongs to P0.5. OAuth, ChatGPT
credentials, Rook, dynamic loading, and the separate onboarding QA issues are
outside this delivery. This document does not close those QA gates.

Protocol references:

- [Ollama OpenAI compatibility](https://docs.ollama.com/api/openai-compatibility)
- [Ollama tool calling](https://docs.ollama.com/capabilities/tool-calling)
- [Ollama model information](https://docs.ollama.com/api/show)
