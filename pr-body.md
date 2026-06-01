## What

Implements **FormatRegistry Phase 2** — explicit, registry-driven multi-format routing. This closes the functional routing gap described in #63.

### Core changes

| Layer | Change |
|---|---|
| **Domain** | `ApiFormat` moved to `rook-core` to eliminate cross-layer cycles |
| **Domain** | `FormatTranslatorPort` introduced as a domain-level abstraction |
| **Ports** | `ProviderPort::api_format()` added to every provider |
| **Registry** | `FormatRegistry` now: registers pairs, returns request/response translators per direction, ships `IdentityTranslator` for same-format, ships `DomainPivotTranslator` for OpenAI to Anthropic |
| **Routing** | `RouteRequest::execute_with_format()` and `execute_stream_with_format()` select provider by `api_format()` and apply translator pairs |
| **Transport** | `routes.rs` detects format from endpoint (`/v1/chat/completions` to OpenAI, `/v1/messages` to Anthropic) |
| **DI** | Rook DI registers bidirectional OpenAI Anthropic pairs at boot |
| **Tests** | Integration tests cover Anthropic to OpenAI and OpenAI to Anthropic via registry |

### Architecture

Client format (ApiFormat) flows through RouteRequest, which calls `provider.api_format()` on the selected ProviderPort, then looks up translators via `FormatRegistry::get_request_translator(client_format, provider_format)` and `get_response_translator(provider_format, client_format)`. Before this change, routing was implicit with no registry, no format tracking, and no extensibility. Now routing is explicit and adding a new translator pair requires only calling `FormatRegistry::register()` with two translators.

## Why

Before this change, routing was implicit — the transport layer called adapters directly with no registry, no format tracking, and no extensibility for new provider formats. Now routing is explicit: every call goes through the registry, format detection is in one place, and adding a new translator pair requires only calling `FormatRegistry::register()` with two translators.

## Testing

- cargo test -p transport-axum --test format_translation_integration
- cargo test -p transport-axum format_registry
- cargo test -p rook-usecases route_request
- cargo test --workspace
- cargo clippy --all-targets --all-features -- -D warnings

## Next steps (out of scope for this PR)

- Register gemini, groq, ollama format pairs (they map to OpenAI-compatible today)
- Add a registry endpoint for dynamic registration at runtime
- Document format pairs in docs/providers.md

---

**Files**: 14 changed, +595 lines