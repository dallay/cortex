# Skill: rust-naming

Rust naming conventions adapted for the Cortex monorepo.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — naming rules.

## Purpose

Standardize names across the codebase to improve readability and maintainability.
The Rust compiler enforces some naming conventions.

## Naming Rules

### 1. name-funcs-snake

> Use `snake_case` for functions, methods, and variables.

 ```rust
 // ✅ Correct
 fn calculate_total() -> f64 { ... }
 fn get_provider_id(&self) -> &ProviderId { ... }
 let max_retries = 3;

 // ❌ Incorrect
 fn calculateTotal() -> f64 { ... }  //warning
 fn GetProviderId(&self) -> &ProviderId { ... }  //warning
 ```

### 2. name-consts-screaming

> Use `SCREAMING_SNAKE_CASE` for constants and statics.

 ```rust
 // ✅ Correct
 const MAX_CONNECTIONS: u32 = 100;
 const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
 static CACHE: OnceLock<Cache> = OnceLock::new();

 // ❌ Incorrect
 const maxConnections: u32 = 100;  //warning
 ```

### 3. name-acronym-word

> Treat acronyms as words: `HttpServer`, not `HTTPServer`.

 ```rust
 // ✅ Correct
 struct HttpServer { ... }
 struct JsonParser { ... }
 fn parse_json() { ... }
 let tcp_connection = connect_tcp();

 // ❌ Incorrect
 struct HTTPServer { ... }  // HTTP is an acronym; do not capitalize the whole name
 struct JSONParser { ... }
 fn parseJSON() { ... }
 ```

### 4. name-into-ownership

> Use the `into_` prefix for conversions that consume ownership.

 ```rust
 impl Request {
     /// Consumes self and returns the inner value.
     pub fn into_completion_request(self) -> CompletionRequest {
         self.request
     }
 }

 // Usage:
 let request = wrapper.into_completion_request();  // wrapper is consumed
 ```

### 5. name-as-free

> Use the `as_` prefix for low-cost conversions (O(1), no allocation).

 ```rust
 impl ProviderConfig {
     /// Returns a reference to the timeout (free conversion).
     pub fn as_timeout(&self) -> &Duration {
         &self.timeout
     }
 }

 // Usage:
 let timeout_ref = config.as_timeout();  // No allocation
 ```

## Conversion Pattern Summary

| Prefix  | Cost      | Ownership         | Example                |
 |---------|-----------|-------------------|------------------------|
| `as_`   | Free      | `&T -> &U`        | `str::as_bytes()`      |
| `to_`   | Expensive | `&T -> U` (alloc) | `str::to_lowercase()`  |
| `into_` | Variable  | `T -> U`          | `String::into_bytes()` |

 ```rust
 // as_ — free borrow
 let bytes: &[u8] = string.as_bytes();

 // to_ — allocates new value
 let upper: String = text.to_uppercase();

 // into_ — consumes self
 let bytes: Vec<u8> = string.into_bytes();
 ```

## For New Crates

When creating a new public crate:

- Do **not** add the `-rs` or `-rust` suffix
- Use the name of the concept, not the language

 ```toml
 # ✅ Correct
 name = "provider-sqlite"

 # ❌ Incorrect
 name = "provider-sqlite-rs"
 name = "rust-provider-sqlite"
 ```

## Enforcement

Rust warns about many of these by default:

 ```bash
 cargo clippy
 # warning: function `calculateTotal` should have a snake case name
 ```

## Special Cases in Cortex

### Provider IDs and Model IDs

 ```rust
 // Our newtypes already use PascalCase internally
 #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
 pub struct ProviderId(pub u64);

 // Variables and functions use snake_case
 fn get_provider_id(provider: &ProviderId) -> u64 {
     provider.0
 }
 ```

### Async Handlers en Transport

 ```rust
 // HTTP handlers
 pub async fn handle_completion(
     Json(payload): Json<CompletionRequest>,
 ) -> Result<Json<CompletionResponse>, StatusCode> {
     // ...
 }
 ```

## Code Review Checklist

- [ ] Functions/methods use `snake_case`
- [ ] Constantes en `SCREAMING_SNAKE_CASE`
- [ ] Acronyms are treated as words (`HttpServer`, not `HTTPServer`)
- [ ] Use the correct conversion prefix (`as_`, `to_`, `into_`)
- [ ] Do not use the `-rs` suffix in crate names

## Related Files

- rust-api-design — For API design
- rust-linting — For lint enforcement

## See Also

- [Rust Naming Conventions](https://rust-lang.github.io/api-guidelines/naming.html)
- rust-anti-patterns — Naming anti-patterns
