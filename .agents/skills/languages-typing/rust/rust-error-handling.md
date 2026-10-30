# Skill: rust-error-handling

Rust error-handling patterns adapted for the Cortex monorepo.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — filtered and adapted.

## Purpose

Standardize error handling across the codebase while following the
domain-driven error pattern established with `CortexError`.

## Error Architecture in Cortex

  ```
  shared-kernel (sin deps)
    └── CortexError (error wrapper with Box<dyn Error>)
        ├── ProviderError
        ├── NotFoundError
        ├── RateLimitedError
        └── AllProvidersExhaustedError

  rook-core (domain)
    └── Defines ports with CortexResult<T>

  transport-axum (infrastructure)
    └── Adapts provider errors to CortexError

  apps/rook (application)
    └── Handles errors, logging, and HTTP responses
  ```

This pattern is BETTER than anyhow for library/domain code because:

1. Typed errors that callers can match
2. Domain-specific error types
3. Integration with the fallback system

## Included Rules

### 1. err-thiserror-lib

> Use thiserror for typed errors in library crates.

We already use it correctly in shared-kernel. Keep following this pattern.

 ```rust
#[derive(Debug, thiserror::Error)]
  #[error(transparent)]
  pub struct CortexError {
      #[from]
      source: Box<dyn std::error::Error + Send + Sync + 'static>,
  }
  ```

**Do NOT use anyhow in domain crates.** Anyhow is for applications/binaries.

### 2. err-from-impl

> Implement `From<E>` for automatic conversions.

  ```rust
  impl From<std::io::Error> for CortexError {
      fn from(err: std::io::Error) -> Self {
          Self::provider(err.to_string())
      }
  }
  ```

This enables the `?` operator automatically.

### 3. err-no-unwrap-prod ⚠️ CRITICAL

> Avoid `.unwrap()` in production code.

Found in provider implementations and adapters.

  ```rust
  // BAD
  fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
      Ok(self.client.post(&url).unwrap()) // Panic on error!
  }

  // GOOD
  fn complete(&self, req: &CompletionRequest) -> CortexResult<CompletionResponse> {
      self.client.post(&url)
          .await
          .map_err(|e| CortexError::provider(e.to_string()))
  }
  ```

Valid exceptions:

- Tests
- After an explicit check (e.g., `if let Some(x) = v { x.unwrap() }`)
- Const/static values with a compile-time guarantee

### 4. err-question-mark

> Use the `?` operator for clean error propagation.

 ```rust
 // BAD
 fn load_config(path: &str) -> Result<Config, Error> {
     let content = match std::fs::read_to_string(path) {
         Ok(c) => c,
         Err(e) => return Err(Error::Io(e)),
     };
     Ok(Config::parse(&content)?)
 }

 // GOOD
 fn load_config(path: &str) -> Result<Config, Error> {
     let content = std::fs::read_to_string(path)?;
     Ok(Config::parse(&content)?)
 }
 ```

Combine with context for better messages:

 ```rust
 use anyhow::{Context, Result};

 // Application code only, not domain code
 fn load_user(path: &Path) -> Result<User> {
     let content = std::fs::read_to_string(path)
         .with_context(|| format!("failed to read user from {}", path.display()))?;
     serde_json::from_str(&content)
         .context("failed to parse user JSON")
 }
 ```

### 5. err-result-over-panic

> Prefer `Result<T, E>` over panics for recoverable errors.

Decision guide:

| Situation                          | Use                                  |
 |------------------------------------|--------------------------------------|
| Provider unavailable               | `Result`                             |
| Rate limited                       | `Result` (CortexError::rate_limited) |
| Index out of bounds (user data)    | `Result`                             |
| Index out of bounds (internal bug) | `panic!` with expect                 |
| Invariant violated (program bug)   | `panic!`                             |
| Unreachable code                   | `unreachable!()`                     |

### 6. err-source-chain

> Preserve error chains with `#[source]`.

 ```rust
 #[derive(Debug, thiserror::Error)]
 enum ProviderError {
     #[error("network error contacting {provider}")]
     Network {
         provider: String,
         #[source]
         source: reqwest::Error,
     },

     #[error("provider {provider} returned error: {message}")]
     Api {
         provider: String,
         message: String,
     },
 }
 ```

**Important:** Using `#[source]` preserves the complete error chain,
which is useful for debugging and logging.

## Anti-Patterns to Avoid

 ```rust
 // ❌ DO NOT USE anyhow in the domain layer
 // (apps/rook may use anyhow; the domain layer may NOT)

 // ❌ DO NOT create errors as strings
 fn bad() -> Result<(), String> {
     Err("something went wrong".to_string())
 }

 // ❌ DO NOT use untyped Box<dyn Error>
 fn bad() -> Result<(), Box<dyn std::error::Error>> {
     // The caller cannot match a specific error
 }

// ✅ USE CortexError with domain-specific types
  fn good() -> CortexResult<Response> {
     // Callers can match on the error:
     // match err {
     //     e if e.is_rate_limited() => handle_rate_limit(e),
     //     _ => handle_generic_error(e),
     // }
 }
 ```

## Related Files

- `crates/domain/shared-kernel/src/error.rs` — CortexError and variants
- `crates/domain/rook-core/src/` — Ports using CortexResult
- `crates/infrastructure/providers-*/src/` — Provider implementations

## See Also

- rust-api-design — For API design
- rust-anti-patterns — Anti-patterns to avoid
