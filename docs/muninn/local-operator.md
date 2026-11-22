# Local operator access

Muninn provides an opt-in, in-process Rust capability for operations on a
single authorized owner's data. Trusted composition code must select the
owner before it shares the capability. Do not take the owner from a request.
The owner must be nonblank, is kept verbatim, and cannot be rebound on the same
client. An owner identifier does not authenticate or authorize a caller.

```rust
use muninn_application::{MemoryClient, MemoryOptions, MemoryPorts};
use muninn_core::{model::MessageRole, MemoryResult};

async fn local_example(ports: MemoryPorts, authorized_owner: &str) -> MemoryResult<()> {
    let memory = MemoryClient::new(ports, &MemoryOptions::default())?
        .with_local_operator(authorized_owner)?;
    let local = memory.local_operator();

    // All four entry points return the same LocalOperator type and scope.
    let graph = memory.graph()?;
    let short = local.short_term().client()?;
    let long = local.long_term().client()?;
    let reasoning = local.reasoning().client()?;

    short.add_message("support", MessageRole::User, "Please review this case.").await?;
    let conversation = graph.get_conversation("support", Some(20)).await?;
    let _trace = reasoning.start_trace("support", "Review the case").await?;
    let _steps = reasoning.get_conversation_trace(conversation.id).await?;
    let _preferences = long.get_preferences(true).await?;
    Ok(())
}
```

Share only the `LocalOperatorView` or the `LocalOperator` with code that needs
this capability. Keep the full `MemoryClient` and its legacy tiers in trusted
code. All four entry points return `NotSupported` unless explicitly enabled.
If you enable the operator on `MemoryClient::read_only`, its reads work but its
writes return `NotSupported` before accessing storage. This does not change any
legacy tier method.

## Operations

- `get_conversation` reads an owner-scoped session and its messages.
  `add_message` appends to that namespace, creating the conversation if needed.
- `get_preferences` reads preferences linked to the owner. `add_preference`
  writes a new preference for the owner using the existing atomic write path,
  without entity links or semantic deduplication.
- `get_conversation_trace` reads hosted steps and tool calls for an owned
  conversation. `start_trace` creates a standalone trace for the owner. It
  does not attach the trace to the conversation or make it appear in the
  hosted steps returned by `get_conversation_trace`. The existing standalone
  trace listing has no owner filter, so the operator does not offer it.

Operator writes do not compute embeddings or run graph extraction. Unlike the
open upstream accessors, none of these entry points exposes a database driver,
credentials, arbitrary Cypher, global graph operations, or HTTP/MCP routes.
The independent, read-only `query.cypher` contract is unchanged. This limited
capability is not full upstream equivalence.
