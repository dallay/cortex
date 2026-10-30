# Skill: rust-documentation

Rust documentation patterns adapted for the Cortex monorepo.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — documentation rules.

## Purpose

Standardize how we document code across the Cortex monorepo.
Good documentation reduces support burden and improves adoption.

## Included Rules

### 1. doc-all-public

> Document all public items with `///`.

Every public item must have documentation that explains:

- What it is
- How to use it
- Possible errors

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

> Use `//!` for module documentation.

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
 > Include `# Examples` with executable code.

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

**Use `?` in examples, not `.unwrap()`:**

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

> Include `# Errors` for functions that can fail.

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

> Include `# Panics` for functions that can panic.

 ```rust
 /// Returns the element at the given index.
 ///
 /// # Panics
 ///
 /// Panics if `index >= self.len()`.
 pub fn get(&self, index: usize) -> &T { &self.data[index] }
 ```

### 6. doc-safety-section

> Include `# Safety` for unsafe functions.

 ```rust
 /// Reads a value from a pointer.
 ///
 /// # Safety
 ///
 ///  /// Caller must guarantee:
 /// - `ptr` is valid for reads
 /// - `ptr` is properly aligned
 /// - No mutable references exist
 pub unsafe fn read_ptr<T>(ptr: *const T) -> T { ptr.read() }
 ```

### 7. doc-intra-links

> Use intra-doc links `[Type]` to reference types.

 ```rust
 /// See [`ProviderPort`] for the provider interface.
 /// See [`CompletionRequest`] for request types.
 ///
 /// [`ProviderPort`]: crate::ProviderPort
 pub trait RouterPort { ... }
 ```

### 8. doc-hidden-setup

> Prefix setup lines with `#` and a space to hide them in examples.

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

## Enforced by Lints

 ```toml
 [lints.rust]
 missing_docs = "warn"
 ```

 ```rust
 // Current exceptions in Cortex:
 #![allow(missing_errors_doc)]
 #![allow(missing_panics_doc)]
 ```

These are allowed because the codebase is not yet fully documented,
but all NEW code must follow them.

## Checklist for New Functions

For every new public function:

- [ ] `///` doc comment
- [ ] `# Examples` with executable code
- [ ] `# Errors` si returns `Result`
- [ ] `# Panics` if it can panic
- [ ] `# Safety` if it is `unsafe`
- [ ] Intra-doc links to related types

## Related Files

- `crates/domain/rook-core/src/` — Domain types
- `crates/infrastructure/providers-*/src/` — Provider implementations
- `rust-linting.md` — Lint configuration

## See Also

- rust-linting — For documentation lint enforcement
- rust-api-design — For public API design
