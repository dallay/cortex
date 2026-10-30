# Skill: rust-api-design

Rust API design patterns adapted for the Cortex monorepo.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — filtered and adapted.

## Purpose

Standardize how we design public APIs in Cortex crates while respecting
hexagonal architecture and established patterns.

## Included Rules

### 1. api-sealed-trait

> Use sealed traits to control external implementations.

In Cortex, `ProviderPort` already follows this pattern. This is the correct approach.

 ```rust
 mod private {
     pub trait Sealed {}
 }

pub trait ProviderPort: private::Sealed {
      async fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse>;
  }

 // Only internal providers can implement this
 impl private::Sealed for OpenAiProvider {}
 impl ProviderPort for OpenAiProvider { ... }
 ```

**Apply to:** New public traits that should not be implementable externally.

### 2. api-newtype-safety

> Use newtypes to prevent mixing semantically different values.

We already follow this pattern in shared-kernel:

- `ProviderId`, `ModelId`, and `RequestId` — all newtypes over `u64`/`String`

 ```rust
 // GOOD — Established pattern
 #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
 pub struct ProviderId(pub u64);

 #[derive(Debug, Clone, PartialEq, Eq)]
 pub struct ModelId(pub String);
 ```

**Apply to:** New types that could be confused with one another.

### 3. api-must-use

> Mark types and functions with `#[must_use]` when ignoring their results is a bug.

Crucial for builders and `Result` types.

 ```rust
 #[must_use = "builders do nothing unless consumed"]
 pub struct RequestBuilder { ... }

 impl RequestBuilder {
     #[must_use = "builder methods return modified builder"]
     pub fn with_timeout(mut self, t: Duration) -> Self { ... }
 }
 ```

In Cortex: Apply this to builders in `transport-axum` and any `Result` that must not be ignored.

### 4. api-common-traits

> Implement standard traits (`Debug`, `Clone`, `PartialEq`, etc.) for public types.

Derivation guide:

| Trait       | When to derive                     |
 |-------------|------------------------------------|
| `Debug`     | Always for public types            |
| `Clone`     | The type can be duplicated         |
| `Copy`      | Small, simple types                |
| `PartialEq` | Equality comparison is meaningful  |
| `Eq`        | Total equality, without floats     |
| `Hash`      | For `HashMap` keys                 |

 ```rust
 // Minimum for domain types
 #[derive(Debug, Clone, PartialEq)]
 pub struct CompletionRequest { ... }
 ```

### 5. api-default-impl

> Implement `Default` for types with sensible default values.

Useful for configuration and state.

 ```rust
 impl Default for HealthStatus {
     fn default() -> Self {
         Self {
             available: false,
             latency_ms: None,
             last_check: None,
         }
     }
 }
 ```

### 6. api-non-exhaustive

> Use `#[non_exhaustive]` on public enums and structs for forward compatibility.

 ```rust
 #[non_exhaustive]
 pub enum ProviderKind {
     OpenAi,
     Anthropic,
     Ollama,
     // Future providers can be added without a breaking change
 }

 // Consumers MUST use a wildcard
 match provider.kind() {
     ProviderKind::OpenAi => { ... },
     ProviderKind::Anthropic => { ... },
     _ => { ... }, // Required
 }
 ```

**Apply to:** Public enums that may evolve (`ProviderKind`, etc.).

### 7. api-parse-dont-validate

> Parse into validated types at boundaries.

The correct pattern is to validate once at the entry point and use typed values internally.

 ```rust
 // BAD — Scattered validation
 fn handle_request(email: &str) {
     if !is_valid_email(email) { return Err(...); }
     store_email(email); // Did someone else validate this?
 }

 // GOOD — Parse at the boundary
 fn handle_request(raw: &str) -> Result<()> {
     let email = Email::parse(raw)?; // Validated ONCE
     store_email(&email); // Type-safe here
 }
 ```

In Cortex: Apply this when parsing requests in `transport-axum` adapters.

## Related Files

- `crates/domain/shared-kernel/src/id.rs` — Newtypes existentes
- `crates/domain/rook-core/src/ports.rs` — Domain traits
- `crates/infrastructure/transport-axum/src/` — Adapters

## See Also

- rust-anti-patterns — Anti-patterns to avoid
- rust-async-patterns — Async patterns
