> **`provider_id` spelling contract.** The bundled catalog keys declarations by
> canonical provider descriptor id (`openai`, `ollama-cloud`, `azure-openai`).
> The legacy serde spellings (`ollamacloud`, `azureopenai`) are still accepted
> on read through the descriptor alias map, but new entries must use the
> canonical id, and unknown ids are rejected at load time.

# Model catalog and eligibility

Rook separates three facts: the presence of an identifier, the model's
declared capabilities, and the capabilities retained by the adapter. The presence
of a model does not prove it supports tools, images, or streaming.

## Declarations

The catalog included in `models-catalog/src/models.json` retains Tier-1 models
and their documented capabilities (context window, vision, tools, reasoning,
and streaming). Any capability not documented stays `unknown`.

Administrative declarations in `rook.toml` allow registering verified
capabilities or overriding catalogued ones:

```toml
[[model_catalog]]
provider_id = "openai"
model_id = "verified-deployment"

[model_catalog.capabilities]
tools = "supported"
streaming = "supported"
vision = "unsupported"
reasoning = "unknown"
context_tokens = 8192

[[model_catalog]]
provider_id = "openai"
model_id = "verified-deployment"
connection_id = "c56a4180-65aa-42ec-a945-5fd21dec0538"

[model_catalog.capabilities]
tools = "unknown"
```

The identifiers and the limit in the example are illustrative, not guarantees for
a real provider. The administrator must verify the model and the endpoint before
declaring `supported`. Declarations do not contain credentials.

The model catalog pre-merges all sources once at construction into a single
indexed view (`StaticModelCatalog`). Resolution applies each field with the
following priority order evaluated at construction:

1. Declaration of the exact connection and model (`connection_configuration`).
2. Declaration of the provider type and model (`provider_configuration`).
3. Declaration included in the catalog (`bundled`).
4. `unknown`.

An omitted field inherits the previous value. An explicit `unknown` state
removes a lower-priority support claim. `context_tokens` is
optional and must be positive; its absence means the limit is not known.
Duplicate declarations for the same connection, provider, and model are
invalid. Changes require restarting Rook.

Additional models from a provider declaration are added to that provider's
catalog. Models declared for a connection are not published to
other connections. Azure retains only its configured deployment;
OpenAI-compatible retains only `supported_models`. Metadata does not
extend those two lists.

## Provenance and discovery

Each known capability and limit retains `source`: `bundled`,
`provider_configuration`, `connection_configuration`, `adapter`, or `unknown`.
`ModelCatalogPort::resolve` also accepts identifiers obtained through
discovery verified by the caller. It combines that identifier with the
applicable declarations, but does not infer capabilities from name or presence.
Health checks do not become capability discovery.
This implementation does not add automatic queries to providers.

Resolved metadata remains associated with the same adapter instance
across registry updates and crosses the concurrency boundary.

## Conservative policy

A required capability needs positive support from both the model **and** the
adapter. Rook excludes candidates with state `unsupported` or `unknown` before
making a call to the provider. Exclusions do not consume call attempts
or open circuits. If no eligible candidate remains, the provider-exhaustion
error is retained; combos include distinct exclusion reasons
for incompatibility and lack of evidence.

- Tool declarations, `tool_choice` other than `none`, and
  messages with tool history require `tools`.
- A direct streaming request requires `streaming`.
- Ordinary text does not require unused optional capabilities.
- Combos run `complete` and synthesize SSE. They do not need native streaming.
- The alias's permissions and those of its physical steps are checked before
  capability filtering. An unauthorized later step still rejects the combo.
- Requests requiring tools do not read or write the response cache. A
  response saved before changing the declaration does not allow
  bypassing the current policy.

Current adapters do not retain input images. OpenAI rejects
non-text content when deserializing the request, with HTTP 422, before
converting it to the domain or making external calls. Anthropic also rejects
non-representable image blocks. Declaring `vision = "supported"` does not enable
that functionality. Support for explicit reasoning controls is also not
enabled by a declaration.

The OpenAI, Azure OpenAI, OpenAI-compatible, and Anthropic adapters can
retain tool requests; Gemini, Groq, and Ollama currently do not retain
all the required fields and cannot be enabled by configuration.
Gemini does not implement native streaming. These adapter restrictions do not
constitute evidence about the remote model's capabilities.

## `GET /v1/models` response

`id`, `object`, `created`, and `owned_by` are retained. The additive `rook` extension
includes capabilities, presence provenance, and unknown-value policy:

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

The list respects `allowed_models` and `allowed_providers` of the authenticated context.
A `rook/*` alias is only visible if its physical steps are also authorized.
Aliases retain their identifiers and publish `unknown` capabilities:
Rook does not attribute support of one of its steps to the whole combo.

## Migration

**Before upgrading a deployment that uses tools or direct streaming,
declare the verified capabilities of its models.** This also applies to
identifiers included in the catalog: their previous capabilities were not
verified and are not automatically converted to `supported`.

Do not use a global provider declaration for custom endpoints that do not
share its guarantees. Prefer `connection_id` for those cases. A
text request without tools and without streaming keeps its previous eligibility.

To test without real providers:

```sh
cargo test -p models-catalog
cargo test -p rook-usecases --test capability_eligibility
cargo test -p rook-usecases --lib route_request::tests::capabilities
cargo test -p transport-axum --test capability_contract --test gateway_e2e
cargo test -p rook --test config_tests --test di_tests
```
