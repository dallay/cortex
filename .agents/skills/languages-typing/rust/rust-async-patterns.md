# Skill: rust-async-patterns

Tokio async coding patterns adapted for the Cortex monorepo.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — filtered and adapted.

## Purpose

Standardize async patterns across the codebase, especially those critical
for preventing deadlocks and ensuring graceful shutdown.

## Included Rules

### 1. async-no-lock-await ⚠️ CRITICAL

> Never hold a `Mutex` or `RwLock` across `.await`.

This is THE critical issue found during the analysis. The codebase CURRENTLY
has errors of this kind in `router_impl.rs`.

 ```rust
 // BAD — Holding a lock across await can cause deadlocks
 async fn bad(state: &Mutex<State>) {
     let mut guard = state.lock().await;
     let data = fetch_from_network().await; // Lock held!
     guard.value = data;
 }

 // GOOD — Lock only for a quick update
 async fn good(state: &Mutex<State>) {
     let data = fetch_from_network().await; // No lock held
     let mut guard = state.lock().await;
     guard.value = data; // Quick update
 }
 ```

**Why does it matter?** Tokio uses a small number of threads for many tasks. If a task
holds a lock during I/O, all other tasks wait — potentially indefinitely.

### 2. async-clone-before-await

> Clone `Arc`/`Rc` data before await points.

This is necessary for futures to be `Send`.

 ```rust
 // BAD — Borrow across await, future not Send
 async fn process(data: Arc<Data>) {
     let slice = &data.items[..]; // Borrow
     expensive_async_op().await;    // Await with active borrow
     use_slice(slice);
 }

 // GOOD — Clone owned data before await
 async fn process(data: Arc<Data>) {
     let items = data.items.clone(); // Owned
     expensive_async_op().await;
     use_items(&items);
 }
 ```

In Cortex: Important for `spawn_blocking` and for passing data to tasks.

### 3. async-join-parallel

> Use `join!` or `try_join!` for independent futures that can run concurrently.

 ```rust
 // BAD — Sequential, 300ms total
 async fn fetch_data() -> (User, Posts, Comments) {
     let user = fetch_user().await;      // 100ms
     let posts = fetch_posts().await;    // 100ms
     let comments = fetch_comments().await; // 100ms
     (user, posts, comments)
 }

 // GOOD — Concurrent, ~100ms
 async fn fetch_data() -> (User, Posts, Comments) {
     let (user, posts, comments) = tokio::join!(
         fetch_user(),
         fetch_posts(),
         fetch_comments(),
     );
     (user, posts, comments)
 }
 ```

**Apply to:** Fallback across multiple providers and parallel health checks.

### 4. async-spawn-blocking

> Use `spawn_blocking` for CPU-intensive work.

 ```rust
 // BAD — CPU work on an async thread
 async fn hash_password(password: String) -> String {
     bcrypt::hash(&password, bcrypt::DEFAULT_COST).unwrap() // Blocks runtime!
 }

 // GOOD — Offload to the blocking thread pool
 async fn hash_password(password: String) -> String {
     tokio::task::spawn_blocking(move || {
         bcrypt::hash(&password, bcrypt::DEFAULT_COST).unwrap()
     })
     .await
     .unwrap()
 }
 ```

Guidelines:

- < 10µs: OK on an async thread
- 10µs - 1ms: Consider `spawn_blocking`
- > 1ms: Definitely use `spawn_blocking`

### 5. async-cancellation-token

> Use `CancellationToken` for graceful shutdown.

 ```rust
 use tokio_util::sync::CancellationToken;

 async fn run_server(shutdown: CancellationToken) {
     loop {
         tokio::select! {
             _ = shutdown.cancelled() => {
                 println!("Shutting down gracefully");
                 cleanup().await;
                 break;
             }
             result = listener.accept() => {
                 let (socket, _) = result?;
                 let conn_token = shutdown.child_token();
                 tokio::spawn(handle_connection(socket, conn_token));
             }
         }
     }
 }
 ```

**Apply to:** Server shutdown in `apps/rook` and graceful request cancellation.

### 6. async-bounded-channel

> Use bounded channels to apply backpressure.

 ```rust
 // BAD — Unbounded channels can cause OOM
 let (tx, mut rx) = mpsc::unbounded_channel();

 // GOOD — Backpressure natural
 let (tx, mut rx) = mpsc::channel(100); // Cap at 100
 ```

Guidelines for buffer size:

- Start with the expected burst size
- Initially err on the smaller side
- Measure in production

## Common Errors in Cortex

 ```rust
 // ❌ ERROR FOUND in router_impl.rs
 let providers = self.providers.read().unwrap();
 // parking_lot::RwLock does not have .read().await()
 // Use tokio::sync::RwLock OR do not await while using parking_lot

 // ✅ Correct fix (option 1: use tokio::sync::RwLock)
 let providers = self.providers.read().await;

 // ✅ Alternative fix (option 2: do not make it async)
 let providers = self.providers.read(); // Sync read, no await
 ```

## Related Files

- `crates/application/rook-usecases/src/router_impl.rs` — ⚠️ Contains errors
- `crates/infrastructure/providers-*/src/` — Providers async
- `apps/rook/src/server.rs` — Server shutdown

## See Also

- rust-anti-patterns — Anti-patterns to avoid
- rust-api-design — API patterns
