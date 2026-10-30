# Skill: rust-memory

Rust memory management patterns adapted for the Cortex monorepo.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — memory rules.

## Purpose

Guidance for optimizing memory use in the codebase. Especially relevant
for providers that process large responses and the in-memory cache.

## Included Rules

### 1. mem-with-capacity

> Use `with_capacity()` when you know the size.

 ```rust
 // ✅ Correct - pre-allocation
 let mut results = Vec::with_capacity(providers.len());
 for provider in providers {
     results.push(process(provider));
 }

 // ❌ Incorrect - reallocations
 let mut results = Vec::new();
 for provider in providers {
     results.push(process(provider));  // May reallocate multiple times
 }
 ```

### 2. mem-avoid-format

> Avoid `format!()` in hot paths.

 ```rust
 // ✅ Correct - string literal
 fn get_error_message() -> &'static str {
     "An error occurred"
 }

 // ❌ Incorrect - allocation on every call
 fn get_error_message() -> String {
     format!("An error occurred")  // Unnecessary allocation
 }
 ```

For logging in loops:

 ```rust
 // ✅ Correct - format arguments directly
 for event in events {
     log::info!("Processing item: {}", event.name);
 }

 // ❌ Incorrect
 for event in events {
     let msg = format!("Processing item: {}", event.name);
     log::info!("{}", msg);  // Double work
 }
 ```

### 3. mem-clone-from

> Use `clone_from()` to reuse allocations across repeated clones.

 ```rust
 let mut buffer = String::with_capacity(1024);
 for source in sources {
     buffer.clone_from(source);  // Reusa la capacity
     process(&buffer);
 }
 ```

### 4. mem-compact-string

> Consider `CompactString` for many small strings.

For short provider and model IDs:

 ```rust
 // Use this instead of String for short IDs
 struct ModelId(CompactString);

 impl ModelId {
     pub fn new(id: &str) -> Option<Self> {
         if id.len() <= 24 {
             Some(ModelId(id.into()))
         } else {
             None
         }
     }
 }
 ```

**Apply only if profiling shows it is necessary.**

### 5. mem-zero-copy

> Use references instead of clones when possible.

 ```rust
 // ✅ Zero-copy - references
 fn process_items(items: &[Item]) {
     for item in items {
         println!("{}", item.name);
     }
 }

 // ❌ Unnecessary allocation
 fn process_items(items: &Vec<Item>) {
     for item in items.clone() {  // Clones the entire Vec
         println!("{}", item.name);
     }
 }
 ```

### 6. mem-boxed-slice

> Use `Box<[T]>` instead of `Vec<T>` for fixed-size data.

 ```rust
 // ✅ Better - pointer + length only (16 bytes)
 struct Document {
     pages: Box<[Page]>,
 }

 // ❌ More overhead (24 bytes: pointer + length + capacity)
 struct Document {
     pages: Vec<Page>,
 }
 ```

### 7. mem-smallvec

> Use `SmallVec` for collections that are usually small.

For HTTP headers or query parameters that are typically few:

 ```rust
 use smallvec::{smallvec, SmallVec};

 struct Request {
     headers: SmallVec<[(String, String); 4]>,  // 4 items inline
 }
 ```

### 8. mem-reuse-collections

> Clear and reuse collections in loops instead of creating new ones.

 ```rust
 // ✅ Correct
 let mut temp = Vec::new();
 for batch in batches {
     temp.clear();  // Reuse the allocation
     for item in &batch.items {
         temp.push(process(item));
     }
     results.push(finalize(&temp));
 }

 // ❌ Incorrect - allocation on every iteration
 for batch in batches {
     let mut temp = Vec::new();  // New allocation each time
     // ...
 }
 ```

## Memory Anti-Patterns

### Avoid excessive cloning

Ya cubierto en `rust-anti-patterns.md`:

- Do not clone when borrowing works
- Use `Arc` for shared ownership
- Clone only en async move

### Avoid `String` for Short Strings

 ```rust
 // ❌ String for short identifiers
 struct ModelId(String);  // 24 bytes + heap

 // ✅ CompactString for short strings
 struct ModelId(CompactString);  // 24 bytes, inline
 ```

## Application in Cortex

### Response Cache

The in-memory cache uses `DashMap` with TTL. Considerations:

- Keys: `CacheKey` derived from a request hash (short)
- Values: `CompletionResponse` can be large
- Avoid unnecessarily cloning complete responses

### Providers

Providers process HTTP responses:

- Use `reqwest` streaming for large responses
- Consider `bytes::Bytes` to avoid copies
- Do not parse the entire JSON if you only need to extract fields

### Routes/Axum

 ```rust
 // ✅ Streaming response
 async fn stream_completion(
     body: axum::body::Body,
 ) -> impl Response {
     let bytes = axum::body::to_bytes(body, 10_000_000).await?;
     // Process without excessive allocation
 }

 // ❌ Buffer the entire body
 async fn stream_completion(
     body: Body,
 ) -> Result<String> {
     let body = body.collect().await?.to_bytes();
     let text = String::from_utf8(body.to_vec())?;  // Multiple allocations
 }
 ```

## Measurement

Measure before optimizing memory:

 ```bash
 # Memory profiling
 valgrind --tool=massif target/release/rook
 cargo flamegraph

 # Or use Instruments on macOS
 Instruments > Allocations
 ```

## Related Files

- rust-anti-patterns — Clone anti-patterns
- rust-async-patterns — Async memory patterns
- rust-linting — Clippy memory lints

## See Also

- [Rust Memory Management](https://doc.rust-lang.org/nomicon/mem.html)
- [bytes crate](https://docs.rs/bytes) — For zero-copy byte handling
