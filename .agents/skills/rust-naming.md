# Skill: rust-naming

Convenciones de nomenclatura para Rust, adaptadas para el monorepo Cortex.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — naming rules.

## Propósito

Estandarizar nombres en todo el codebase para que sea más legible y mantenible.
Las convenciones de Rust son enforced por el compilador en algunos casos.

## Reglas de Nomenclatura

### 1. name-funcs-snake

> Usa `snake_case` para funciones, métodos y variables.

 ```rust
 // ✅ Correcto
 fn calculate_total() -> f64 { ... }
 fn get_provider_id(&self) -> &ProviderId { ... }
 let max_retries = 3;

 // ❌ Incorrecto
 fn calculateTotal() -> f64 { ... }  //warning
 fn GetProviderId(&self) -> &ProviderId { ... }  //warning
 ```

### 2. name-consts-screaming

> Usa `SCREAMING_SNAKE_CASE` para constantes y statics.

 ```rust
 // ✅ Correcto
 const MAX_CONNECTIONS: u32 = 100;
 const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
 static CACHE: OnceLock<Cache> = OnceLock::new();

 // ❌ Incorrecto
 const maxConnections: u32 = 100;  //warning
 ```

### 3. name-acronym-word

> Trata los acrónimos como palabras: `HttpServer`, no `HTTPServer`.

 ```rust
 // ✅ Correcto
 struct HttpServer { ... }
 struct JsonParser { ... }
 fn parse_json() { ... }
 let tcp_connection = connect_tcp();

 // ❌ Incorrecto
 struct HTTPServer { ... }  // HTTP es un acrónimo, no todo mayúsculas
 struct JSONParser { ... }
 fn parseJSON() { ... }
 ```

### 4. name-into-ownership

> Usa `into_` prefix para conversiones que consumen ownership.

 ```rust
 impl Request {
     /// Consumes self and returns the inner value.
     pub fn into_completion_request(self) -> CompletionRequest {
         self.request
     }
 }

 // Uso:
 let request = wrapper.into_completion_request();  // wrapper consumido
 ```

### 5. name-as-free

> Usa `as_` prefix para conversiones libres (O(1), no alloc).

 ```rust
 impl ProviderConfig {
     /// Returns a reference to the timeout (free conversion).
     pub fn as_timeout(&self) -> &Duration {
         &self.timeout
     }
 }

 // Uso:
 let timeout_ref = config.as_timeout();  // No allocation
 ```

## Patrones de Conversion Summary

| Prefix  | Costo     | Ownership         | Ejemplo                |
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

## Para Nuevos Créditos

Cuando crees un nuevo crate público:

- **No** suffixe con `-rs` o `-rust`
- Usa el nombre del concepto, no del lenguaje

 ```toml
 # ✅ Correcto
 name = "provider-sqlite"

 # ❌ Incorrecto
 name = "provider-sqlite-rs"
 name = "rust-provider-sqlite"
 ```

## Enforzamiento

Rust warn en muchos de estos por defecto:

 ```bash
 cargo clippy
 # warning: function `calculateTotal` should have a snake case name
 ```

## Casos Especiales en Cortex

### Provider IDs y Model IDs

 ```rust
 // Nuestros newtypes ya usan PascalCase internamente
 #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
 pub struct ProviderId(pub u64);

 // En variables/funciones: snake_case
 fn get_provider_id(provider: &ProviderId) -> u64 {
     provider.0
 }
 ```

### Async Handlers en Transport

 ```rust
 // handlers HTTP
 pub async fn handle_completion(
     Json(payload): Json<CompletionRequest>,
 ) -> Result<Json<CompletionResponse>, StatusCode> {
     // ...
 }
 ```

## Checklist para Code Review

- [ ] Funciones/métodos en `snake_case`
- [ ] Constantes en `SCREAMING_SNAKE_CASE`
- [ ] Acrónimos como palabras (`HttpServer`, no `HTTPServer`)
- [ ] Conversiones con prefix correcto (`as_`, `to_`, `into_`)
- [ ] No sufijos `-rs` en nombres de crates

## Archivos Relacionados

- rust-api-design — Para diseño de APIs
- rust-linting — Para enforcement

## See Also

- [Rust Naming Conventions](https://rust-lang.github.io/api-guidelines/naming.html)
- rust-anti-patterns — Anti-patterns de naming
