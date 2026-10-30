# Skill: rust-testing

Rust testing patterns adapted for the Cortex monorepo.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — testing rules.

## Purpose

Standardize how we write tests across the Cortex monorepo.
Well-written tests are the first line of defense against regressions.

## Philosophy

> Tests are code too. Apply the same quality standards.

- Clear, readable tests
- One assertion per test when possible
- Descriptive names that explain the behavior
- Arrange-Act-Assert pattern

## Testing Rules

### 1. test-descriptive-names

> Test names should describe behavior, not the method.

 ```rust
 // ❌ Incorrect - describes implementation
 #[test]
 fn test_handle_request_valid_input() {
     // ...
 }

 // ✅ Correct - describes behavior
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

> Prefer one primary assertion per test. Multiple related assertions are valid.

 ```rust
 // ✅ Correct - one clear assertion
 #[test]
 fn provider_config_validates_timeout_range() {
     let result = ProviderConfig::new(Duration::from_secs(u64::MAX));

     assert!(result.is_err());
 }

 // ✅ Also valid - related assertions
 #[test]
 fn parses_completion_response_with_usage() {
     let response = parse_response(JSON_RESPONSE);

     assert_eq!(response.content, "Hello world");
     assert_eq!(response.usage.tokens, 42);
     assert_eq!(response.usage.model, "gpt-4");
 }
 ```

### 3. test-doc-tests

> Include doctests for public functionality.

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

> Use setup and teardown when multiple tests share state.

 ```rust
 // ✅ Correct - setup in the test itself
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

> Always test error paths, not only the happy path.

 ```rust
 // ✅ Correct - test both paths
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

> Use traits to mock external dependencies.

 ```rust
 // ✅ Correct - trait allows mocking
 #[async_trait]
 pub trait ProviderPort: Send + Sync {
     async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, ProviderError>;
 }

 // Mock implementation for tests
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

> Consider property-based testing for functions with invariants.

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

> Include benchmarks for critical code.

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

## Integration Tests vs. Unit Tests

### Unit Tests

- In `mod tests` or `tests/` files next to the code
- Test a single unit (function, method, or struct)
- Fast, with no I/O
- Mock dependencies

### Integration Tests

- In the crate's `tests/` directory
- Test interactions between multiple components
- May use real I/O (or complete mocks)
- Test ports and adapters

 ```rust
 // tests/integration/router.rs
 #[tokio::test]
 async fn full_routing_flow_with_cache() {
     // Setup real cache
     let cache = InMemoryCache::default();
     let router = Router::with_cache(providers(), cache);

     // First call - cache miss
     let result = router.route(request()).await;
     assert!(result.is_ok());

     // Second call - cache hit
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

 # With --test-threads=1 for tests that share state
 cargo test -- --test-threads=1
 ```

## Coverage

 ```bash
 # Install tarpaulin
 cargo install cargo-tarpaulin

 # Run coverage
 cargo tarpaulin --out Xml --output-dir coverage/
 ```

## Related Files

- rust-api-design — For designing testable APIs
- rust-async-patterns — For async tests
- rust-error-handling — For error-path tests

## See Also

- [Testing Rust](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [proptest book](https://altsysrq.github.io/proptest-book/)
- rstest — For parameterized tests
