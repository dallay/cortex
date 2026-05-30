# Skill: rust-testing

Patrones de testing para Rust, adaptados para el monorepo Cortex.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — testing rules.

## Propósito

Estandarizar cómo escribimos tests en todo el monorepo Cortex.
Tests bien escritos son la primera línea de defensa contra regresiones.

## Filosofia

> Tests son código también. Aplicamos las mismas reglas de calidad.

- Tests claros y legibles
- Un assertion por test cuando sea posible
- Nombres descriptivos que explican el comportamiento
- Arrange-Act-Assert pattern

## Reglas de Testing

### 1. test-descriptive-names

> Nombres de tests deben describir el comportamiento, no el método.

 ```rust
 // ❌ Incorrecto - describe implementación
 #[test]
 fn test_handle_request_valid_input() {
     // ...
 }

 // ✅ Correcto - describe comportamiento
 #[test]
 fn returns_error_when_provider_timeout_exceeds_limit() {
     // ...
 }

 #[test]
 fn fallback_to_next_provider_when_primary_fails() {
     // ...
 }
 ```

### 2. test-one-assertion

> Prebe un assertion principal por test. Múltiples assertions válidas.

 ```rust
 // ✅ Correcto - un assertion claro
 #[test]
 fn provider_config_validates_timeout_range() {
     let result = ProviderConfig::new(Duration::from_secs(u64::MAX));

     assert!(result.is_err());
 }

 // ✅ También válido - assertions relacionados
 #[test]
 fn parses_completion_response_with_usage() {
     let response = parse_response(JSON_RESPONSE);

     assert_eq!(response.content, "Hello world");
     assert_eq!(response.usage.tokens, 42);
     assert_eq!(response.usage.model, "gpt-4");
 }
 ```

### 3. test-doc-tests

> Incluye doctests para funcionalidad pública.

 ```rust
 /// Creates a new provider configuration.
 ///
 /// # Examples
 ///
 /// ```
 /// use rook_core::{ProviderConfig, Duration};
 ///
 /// let config = ProviderConfig::new(Duration::from_secs(30));
 /// assert!(config.is_ok());
 /// ```
 pub fn new(timeout: Duration) -> Result<Self, ConfigError> {
     // ...
 }
 ```

### 4. test-setup-teardown

> Usa setup y teardown cuando múltiples tests comparten estado.

 ```rust
 // ✅ Correcto - setup en el test mismo
 #[cfg(test)]
 mod tests {
     use super::*;

     fn create_test_provider() -> TestProvider {
         TestProvider::builder()
             .timeout(Duration::from_secs(1))
             .retries(1)
             .build()
     }

     #[test]
     fn handles_timeout_correctly() {
         let provider = create_test_provider();
         // test logic
     }

     #[test]
     fn retries_on_transient_error() {
         let provider = create_test_provider();
         // test logic
     }
 }
 ```

### 5. test-error-paths

> Siempre testa los paths de error, no solo el happy path.

 ```rust
 // ✅ Correcto - testa ambos paths
 #[test]
 fn validate_provider_id_rejects_empty() {
     let result = ProviderId::from_str("");
     assert!(result.is_err());
 }

 #[test]
 fn validate_provider_id_accepts_valid_id() {
     let result = ProviderId::from_str("provider-123");
     assert!(result.is_ok());
 }
 ```

### 6. test-mocking

> Usa traits para hacer mock de dependencias externas.

 ```rust
 // ✅ Correcto - trait permite mock
 #[async_trait]
 pub trait ProviderPort: Send + Sync {
     async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError>;
 }

 // Mock implementation para tests
 #[cfg(test)]
 struct MockProvider {
     response: Result<CompletionResponse, ProviderError>,
 }

 #[cfg(test)]
 #[async_trait]
 impl ProviderPort for MockProvider {
     async fn complete(&self, _: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
         self.response.clone()
     }
 }

 #[tokio::test]
 async fn routes_to_fallback_when_primary_fails() {
     let mock_primary = MockProvider { response: Err(ProviderError::Timeout) };
     let mock_fallback = MockProvider { response: Ok(response()) };

     let router = Router::new(mock_primary, mock_fallback);
     let result = router.route(request()).await;

     assert!(result.is_ok());
 }
 ```

### 7. test-property-based

> Considera property-based testing para funciones con invariantes.

 ```rust
 use proptest::prelude::*;

 proptest! {
     #[test]
     fn provider_id_roundtrips_through_serialization(id: u64) {
         let original = ProviderId(id);
         let serialized = original.to_string();
         let deserialized: ProviderId = serialized.parse().unwrap();

         prop_assert_eq!(original, deserialized);
     }
 }
 ```

### 8. test-benchmarks

> Incluye benchmarks para código crítico.

 ```rust
 #![feature(test)]
 extern crate test;

 #[cfg(test)]
 mod benches {
     use super::*;

     #[bench]
     fn bench_provider_routing(b: &mut test::Bencher) {
         b.iter(|| {
             let router = test_router();
             block_on(router.route(test_request()))
         });
     }
 }
 ```

## Integration Tests vs Unit Tests

### Unit Tests

- En archivos `mod tests` o `tests/` al lado del código
- Testean una sola unit (función, método, struct)
- Rápidos, sin I/O
- Mocks de dependencias

### Integration Tests

- En `tests/` directory del crate
- Testean interacción entre múltiples components
- Pueden usar I/O real (o mocks completos)
- Testean ports y adapters

 ```rust
 // tests/integration/router.rs
 #[tokio::test]
 async fn full_routing_flow_with_cache() {
     // Setup real cache
     let cache = InMemoryCache::default();
     let router = Router::with_cache(providers(), cache);

     // First call - misses cache
     let result = router.route(request()).await;
     assert!(result.is_ok());

     // Second call - hits cache
     let cached = router.route(request()).await;
     assert!(cached.is_ok());
 }
 ```

## Running Tests

 ```bash
 # Unit tests
 cargo test -p rook-core

 # Integration tests
 cargo test --test '*'

 # With output
 cargo test -p rook-usecases -- --nocapture

 # Benchmarks
 cargo bench -p rook-core

 # With --test-threads=1 para tests que comparten estado
 cargo test -- --test-threads=1
 ```

## Coverage

 ```bash
 # Instalar tarpaulin
 cargo install cargo-tarpaulin

 # Run coverage
 cargo tarpaulin --out Xml --output-dir coverage/
 ```

## Archivos Relacionados

- rust-api-design — Para diseñar APIs testables
- rust-async-patterns — Para tests async
- rust-error-handling — Para tests de error paths

## See Also

- [Testing Rust](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [proptest book](https://altsysrq.github.io/proptest-book/)
- rstest — Para parametric tests
