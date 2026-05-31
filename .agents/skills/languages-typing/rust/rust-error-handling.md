# Skill: rust-error-handling

Patrones de error handling para Rust, adaptados para el monorepo Cortex.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — filtrada y adaptada.

## Propósito

Estandarizar el manejo de errores en todo el codebase, respetando el patrón
de domain-driven errors establecido con CortexError.

## Arquitectura de Errores en Cortex

  ```
  shared-kernel (sin deps)
    └── CortexError (error wrapper con Box<dyn Error>)
        ├── ProviderError
        ├── NotFoundError
        ├── RateLimitedError
        └── AllProvidersExhaustedError

  rook-core (domain)
    └── Define ports con CortexResult<T>

  transport-axum (infrastructure)
    └── Adapta errores de providers a CortexError

  apps/rook (application)
    └── Maneja errores, logging, respuesta HTTP
  ```

Este patrón es MEJOR que anyhow para código de biblioteca/domain porque:

1. Errores tipados que callers pueden matchear
2. Domain-specific error types
3. Integración con el sistema de fallback

## Reglas Incluidas

### 1. err-thiserror-lib

> Usa thiserror para errores tipados en crates de biblioteca.

Ya lo usamos bien en shared-kernel. Mantener este padrão.

 ```rust
#[derive(Debug, thiserror::Error)]
  #[error(transparent)]
  pub struct CortexError {
      #[from]
      source: Box<dyn std::error::Error + Send + Sync + 'static>,
  }
  ```

**NO usar anyhow en crates de dominio.** Anyhow es para aplicación/binaries.

### 2. err-from-impl

> Implementa From<E> para conversiones automáticas.

  ```rust
  impl From<std::io::Error> for CortexError {
      fn from(err: std::io::Error) -> Self {
          Self::provider(err.to_string())
      }
  }
  ```

Esto permite `?` operator automáticamente.

### 3. err-no-unwrap-prod ⚠️ CRÍTICO

> Evita unwrap() en código de producción.

Encontrado en provider implementations y adapters.

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

Excepciones válidas:

- Tests
- After explicit check (e.g., `if let Some(x) = v { x.unwrap() }`)
- Const/static con compile-time guarantee

### 4. err-question-mark

> Usa `?` operator para clean propagation.

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

Combinar con context para mejores mensajes:

 ```rust
 use anyhow::{Context, Result};

 // Solo en application code, no en domain
 fn load_user(path: &Path) -> Result<User> {
     let content = std::fs::read_to_string(path)
         .with_context(|| format!("failed to read user from {}", path.display()))?;
     serde_json::from_str(&content)
         .context("failed to parse user JSON")
 }
 ```

### 5. err-result-over-panic

> Prefiere Result<T, E> sobre panic para errores recuperables.

Decision guide:

| Situación                          | Use                                |
 |------------------------------------|------------------------------------|
| Provider no disponible             | `Result`                           |
| Rate limited                       | `Result` (CortexError::rate_limited) |
| Index out of bounds (user data)    | `Result`                           |
| Index out of bounds (internal bug) | `panic!` con expect                |
| Invariant violated (program bug)   | `panic!`                           |
| Unreachable code                   | `unreachable!()`                   |

### 6. err-source-chain

> Preserva error chains con #[source].

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

**Importante:** El uso de #[source] preserva el error chain completo,
útil para debugging y logging.

## Anti-Patterns a Evitar

 ```rust
 // ❌ NO USAR anyhow en domain layer
 // (apps/rook puede usar anyhow, domain NO)

 // ❌ NO crear errores como Strings
 fn bad() -> Result<(), String> {
     Err("something went wrong".to_string())
 }

 // ❌ NO usar Box<dyn Error> sin tipo
 fn bad() -> Result<(), Box<dyn std::error::Error>> {
     // Caller no puede matchear específicamente
 }

// ✅ USAR CortexError con tipos domain-specific
  fn good() -> CortexResult<Response> {
     // Callers pueden hacer:
     // match err {
     //     e if e.is_rate_limited() => handle_rate_limit(e),
     //     _ => handle_generic_error(e),
     // }
 }
 ```

## Archivos Relacionados

- `crates/domain/shared-kernel/src/error.rs` — CortexError y variants
- `crates/domain/rook-core/src/` — Ports con CortexResult
- `crates/infrastructure/providers-*/src/` — Provider implementations

## See Also

- rust-api-design — Para diseño de APIs
- rust-anti-patterns — Anti-patterns a evitar
