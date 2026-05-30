# Skill: rust-documentation

Patrones de documentación para Rust, adaptados para el monorepo Cortex.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — documentation rules.

## Propósito

Estandarizar cómo documentamos el código en todo el monorepo Cortex.
Una buena documentación reduce la carga de soporte y mejora la adopción.

## Reglas Incluidas

### 1. doc-all-public

> Documenta todos los items públicos con `///`.

Cada item público debe tener documentación que explique:

- Qué es
- Cómo usarlo
- Errores posibles

 ```rust
 /// Configuration for establishing a connection to the provider.
 ///
 /// # Examples
 ///
 /// ```
 /// let config = ProviderConfig {
 ///     timeout: Duration::from_secs(30),
 ///     retries: 3,
 /// };
 /// ```
 pub struct ProviderConfig {
     /// Maximum time to wait for a response.
     pub timeout: Duration,
     /// Number of retry attempts for failed requests.
     pub retries: u32,
 }
 ```

### 2. doc-module-inner

> Usa `//!` para documentación de módulo.

 ```rust
 //! # Provider Module
 //!
 //! This module implements the provider abstraction layer.
 //!
 //! ## Providers
 //!
 //! - [`OpenAiProvider`]
 //! - [`AnthropicProvider`]
 //!
 //! ## Usage
 //!
 //! ```rust,ignore
//! let provider = OpenAiProvider::new(config);
  //! ```

### 3. doc-examples-section
 > Incluye `# Examples` con código ejecutable.

 ```rust
 /// Parses a completion request.
 ///
 /// # Examples
 ///
 /// ```
/// use rook_core::CompletionRequest;
  /// fn parse_request(model: &str, prompt: &str) -> Result<CompletionRequest, ParseError> {
  ///     Ok(CompletionRequest::new(model, prompt)?)
  /// }
  /// ```
 pub fn new(model: &str, prompt: &str) -> Result<Self, ParseError> { ... }
 ```

**Usa `?` en ejemplos, no `.unwrap()`:**

 ```rust
 /// # Examples
 ///
 /// ```
 /// # use my_crate::{Config, Error};
 /// # fn main() -> Result<(), Error> {
 /// let config = Config::load("config.toml")?;
 /// # Ok(())
 /// # }
 /// ```
 ```

### 4. doc-errors-section

> Incluye `# Errors` para funciones que pueden fallar.

 ```rust
 /// Loads a provider configuration.
 ///
 /// # Errors
 ///
 /// Returns an error if:
 /// - The file does not exist
 /// - The TOML is invalid
 /// - Required fields are missing
 pub fn load_config(path: &Path) -> Result<Config, ConfigError> { ... }
 ```

### 5. doc-panics-section

> Incluye `# Panics` para funciones que pueden panic.

 ```rust
 /// Returns the element at the given index.
 ///
 /// # Panics
 ///
 /// Panics if `index >= self.len()`.
 pub fn get(&self, index: usize) -> &T { &self.data[index] }
 ```

### 6. doc-safety-section

> Incluye `# Safety` para funciones unsafe.

 ```rust
 /// Reads a value from a pointer.
 ///
 /// # Safety
 ///
 /// Caller must guarantee:
 /// - `ptr` is valid for reads
 /// - `ptr` is properly aligned
 /// - No mutable references exist
 pub unsafe fn read_ptr<T>(ptr: *const T) -> T { ptr.read() }
 ```

### 7. doc-intra-links

> Usa intra-doc links `[Type]` para referenciar tipos.

 ```rust
 /// See [`ProviderPort`] for the provider interface.
 /// See [`CompletionRequest`] for request types.
 ///
 /// [`ProviderPort`]: crate::ProviderPort
 pub trait RouterPort { ... }
 ```

### 8. doc-hidden-setup

> Usa `# ` prefix para ocultar código de setup en ejemplos.

 ```rust
 /// # Examples
 ///
 /// ```
 /// # use my_crate::{Processor, Config};
 /// # let config = Config::default();
 /// # let processor = Processor::new(config);
 /// let result = processor.process()?;
 /// # Ok::<(), Error>(())
 /// ```
 pub fn process(&self) -> Result<Value, Error> { ... }
 ```

## Enforced con Lints

 ```toml
 [lints.rust]
 missing_docs = "warn"
 ```

 ```rust
 // Excepciones actuales en Cortex:
 #![allow(missing_errors_doc)]
 #![allow(missing_panics_doc)]
 ```

Estos están allow porque el codebase aún no tiene toda la documentación,
pero NUEVO código debe seguirlos.

## Checklist para Nuevas Funciones

Para cualquier función pública nueva:

- [ ] `///` doc comment
- [ ] `# Examples` con código ejecutable
- [ ] `# Errors` si returns `Result`
- [ ] `# Panics` si puede panic
- [ ] `# Safety` si es `unsafe`
- [ ] Intra-doc links a tipos relacionados

## Archivos Relacionados

- `crates/domain/rook-core/src/` — Domain types
- `crates/infrastructure/providers-*/src/` — Provider implementations
- `rust-linting.md` — Lint configuration

## See Also

- rust-linting — Para enforcement de documentación
- rust-api-design — Para diseño de APIs públicas
