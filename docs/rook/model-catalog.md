# Model Catalog and Eligibility

Rook separates three facts: the presence of an identifier, the declared
capabilities of the model, and the capabilities preserved by the adapter. The presence
of a model does not prove that it supports tools, vision, or streaming.

## Declarations

The catalog bundled in `models-catalog/src/models.json` preserves previous
identifiers. It contains no capability guarantees: its initial states
are `unknown`. Administrative declarations in `rook.toml` allow registering verified
capabilities:

```toml
[[model_catalog]]
provider_kind = "openai"
model_id = "verified-deployment"

[model_catalog.capabilities]
tools = "supported"
streaming = "supported"
vision = "unsupported"
reasoning = "unknown"
context_tokens = 8192

[[model_catalog]]
provider_kind = "openai"
model_id = "verified-deployment"
connection_id = "c56a4180-65aa-42ec-a945-5fd21dec0538"

[model_catalog.capabilities]
tools = "unknown"
```

The identifiers and limit in the example are illustrative, not guarantees from
a real provider. The operator must verify the model and endpoint before
declaring `supported`. Declarations do not contain credentials.

Resolution applies each field in this order of precedence:

1. Exact connection and model declaration.
2. Provider kind and model declaration.
3. Declaration bundled in the catalog.
4. `unknown`.

An omitted field inherits the prior value. An explicit `unknown` state
overrides a lower-priority support claim. `context_tokens` is
optional and must be positive; its absence means the limit is unknown.
Duplicate declarations for the same connection, provider, and model are
invalid. Changes require restarting Rook.

Additional models from a provider declaration are added to the catalog
for that provider. Models declared for a specific connection are not published to
other connections. Azure retains only its configured deployment;
OpenAI-compatible retains only `supported_models`. Metadata does not
expand those two lists.

## Provenance and Discovery

Each known capability and limit preserves `source`: `bundled`,
`provider_configuration`, `connection_configuration`, `adapter`, or `unknown`.
`ModelCatalogPort::resolve` also accepts identifiers obtained via
caller-verified discovery. It merges that identifier with applicable
declarations, but does not infer capabilities by name or presence.
Health checks do not turn into capability discovery.
This implementation adds no automated provider querying.

Resolved metadata remains attached to the same adapter instance
during registry updates and crosses concurrency boundaries.

## Conservative Policy

A required capability demands positive support from both the model **and** the
adapter. Rook excludes candidates with status `unsupported` or `unknown` before
making a call to the provider. Rejections do not consume call attempts
or open circuit breakers. If no eligible candidate remains, the provider
exhaustion error is preserved; combos include distinct discard reasons
for incompatibility and lack of evidence.

- Tool declarations, `tool_choice` other than `none`, and messages with tool history require `tools`.
- A direct streaming request requires `streaming`.
- Plain text does not require unused optional capabilities.
- Non-streaming combo requests use `complete`. Streaming combo requests require each eligible step to support native `streaming` and skip completion-only steps.
- Permissions for the alias and its physical steps are checked before filtering
  by capabilities. An unauthorized downstream step still rejects the combo.
- Requests requiring tools do not read or write the response cache.
  A response cached before changing a declaration does not allow
  bypassing current policy.

Current adapters do not preserve input images. OpenAI rejects non-text
content when deserializing the request, returning HTTP 422, before
converting to domain models or making external calls. Anthropic also rejects
non-representable image blocks. Declaring `vision = "supported"` does not enable
that functionality. Support for explicit reasoning controls is likewise
not enabled by a declaration.

OpenAI, Azure OpenAI, OpenAI-compatible, and Anthropic adapters can
preserve tool requests; Gemini, Groq, and Ollama do not currently preserve
all necessary fields and cannot be enabled via configuration.
Gemini does not implement native streaming. These adapter constraints do not
constitute evidence about remote model capabilities.

## `GET /v1/models` Response

Fields `id`, `object`, `created`, and `owned_by` are preserved. The additive `rook`
extension includes capabilities, presence provenance, and unknown policy:

```json
{
  "id": "connection-id/verified-deployment",
  "object": "model",
  "created": 0,
  "owned_by": "connection-id",
  "rook": {
    "capabilities": {
      "tools": { "state": "supported", "source": "provider_configuration" },
      "vision": { "state": "unsupported", "source": "provider_configuration" },
      "reasoning": { "state": "unknown", "source": "provider_configuration" },
      "streaming": { "state": "supported", "source": "provider_configuration" },
      "context_tokens": { "value": 8192, "source": "provider_configuration" }
    },
    "presence_source": "runtime_registry",
    "unknown_policy": "reject_required"
  }
}
```

The list honors `allowed_models` and `allowed_providers` from the authenticated context.
A `rook/*` alias is only visible if its physical steps are also authorized.
Aliases preserve their identifiers and publish `unknown` capabilities:
Rook does not attribute support of one step to the entire combo.

## Migration

**Before updating a deployment using tools or direct streaming,
declare verified capabilities for your models.** This also applies to
identifiers included in the catalog: their previous capabilities were unverified
and do not automatically become `supported`.

Do not use a global provider declaration for custom endpoints that do not
share its guarantees. Prefer `connection_id` for those cases. A text-only
request without tools or streaming preserves its previous eligibility.

To test without real providers:

```sh
cargo test -p models-catalog
cargo test -p rook-usecases --test capability_eligibility
cargo test -p rook-usecases --lib route_request::tests::capabilities
cargo test -p transport-axum --test capability_contract --test gateway_e2e
cargo test -p rook --test config_tests --test di_tests
```
