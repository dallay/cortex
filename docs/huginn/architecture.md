# Huginn Architecture

Technical deep-dive into Huginn's internal design and component interactions.

## Crate Structure

Huginn follows a layered architecture with clear separation between domain contracts, application logic, runtime implementations, and presentation:

```
crates/huginn/
├── core/                    # Domain layer — product contracts only
│   ├── src/kernel.rs        # Plugin system, Supervisor, lifecycle
│   ├── src/providers.rs     # ModelProvider trait
│   └── src/connections.rs   # Connection abstractions
│
├── application/             # Application layer — composition logic
│   ├── src/composition.rs   # Plugin composition
│   ├── src/loop_engine.rs   # Agent loop implementation
│   ├── src/instructions.rs  # Instruction management
│   └── src/connections.rs   # Connection access
│
├── runtime/                 # Infrastructure — concrete implementations
│   ├── src/mcp.rs           # MCP client (Model Context Protocol)
│   ├── src/tools.rs         # Tool registry and native tools
│   ├── src/sessions.rs      # SQLite session store
│   ├── src/providers.rs     # Provider implementations (OpenAI, Anthropic, etc.)
│   └── src/composition.rs   # Production composition
│
├── presentation/            # Presentation layer — UI adapters
│   ├── src/terminal.rs     # Ratatui TUI implementation
│   ├── src/approval.rs     # Approval prompt handling
│   └── src/completion.rs   # Completion/suggestion UI
│
├── plugin-api/              # Plugin API definitions (public interface)
│   └── src/v1/
│
└── reference-extension/    # Reference plugin implementation

apps/huginn/                # Binary crate (CLI entry point)
```

## Plugin System

Huginn uses a **dependency-injection plugin system** based on the `Supervisor` pattern.

### Core Concepts

#### ServiceId

Services are identified by a versioned name:

```rust
ServiceId::new("agent:loop", 1)  // "agent:loop@1"
```

#### Manifest

Each plugin declares what it provides and requires:

```rust
pub struct Manifest {
    pub id: String,              // Unique plugin identifier
    pub provides: Vec<ServiceId>, // Services this plugin offers
    pub requires: Vec<ServiceId>, // Services this plugin needs
}
```

#### Plugin Trait

```rust
#[async_trait]
pub trait Plugin: Send + Sync {
    fn manifest(&self) -> Manifest;
    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()>;
    async fn deactivate(&mut self) -> Result<()> {
        Ok(())
    }
}
```

`deactivate` is optional: it has a default implementation that succeeds, so a
plugin only implements it when it needs cleanup work.

This is a shortened excerpt of the contract. The implementation also defines
lifecycle hooks for stopping, task completion and termination; consult
[`kernel.rs`](../../crates/huginn/core/src/kernel.rs) for the complete API.

### Supervisor

The `Supervisor` manages plugin lifecycle:

| Method | Purpose |
|--------|---------|
| `register(plugin)` | Add a plugin to the system |
| `start()` | Activate plugins in dependency order |
| `resolve::<T>(id)` | Get a registered service |
| `remove(id)` | Unload a plugin and dependents |
| `shutdown()` | Shut down all plugins |
| `diagnostics()` | Inspect plugin state and activation errors |

The methods are asynchronous where lifecycle work requires it. See
[`kernel.rs`](../../crates/huginn/core/src/kernel.rs) for exact signatures.

### Plugin States

```
Discovered → Blocked → Active
                ↓
             Failed
                ↓
            Unloaded
```

### Plugin Context

During activation, plugins receive a `PluginContext` that allows them to:

```rust
// Register a service
ctx.provide::<MyService>(service_id, value)?;

// Resolve a dependency
let dep = ctx.resolve::<OtherService>(&required_id)?;

// Track async tasks for cleanup
ctx.own_task(task_handle);

// Register resources for cleanup on failure
ctx.own_resource(resource);
```

## Core Traits

### ModelProvider

The model contract streams text, tool calls and usage. Routers can pin one model
identity for the duration of a turn:

```rust
#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn is_configured(&self) -> bool;
    async fn pin_turn(&self, cancel: CancellationToken)
        -> Result<Option<Arc<dyn ModelProvider>>>;
    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> Result<BoxStream<'static, Result<ModelDelta>>>;
}
```

The default `is_configured` and `pin_turn` implementations are omitted here.
See [`core/src/lib.rs`](../../crates/huginn/core/src/lib.rs) for the full contract.

### Tool and ToolRegistry

Tools separate preparation from execution so the host can approve effects before
execution. The registry exposes definitions and lookup; plugin lifecycle owns
registration:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn prepare(&self, call: &ToolCall, ctx: &ToolContext)
        -> Result<PreparedAction>;
    async fn execute(&self, action: PreparedAction, ctx: &ToolContext)
        -> Result<String>;
}

pub trait ToolRegistry: Send + Sync {
    fn definitions(&self) -> Vec<ToolDefinition>;
    fn get(&self, name: &str) -> Option<Arc<dyn Tool>>;
}
```

### Tool hooks

`huginn-core::hooks::ToolHooks` is a generation-owned registry published as
`agent:tool-hooks@1`. The loop asks it about every tool call before
`prepare`. A `Block` verdict, or a failing hook, ends the call with an error and
no approval prompt. Hooks have no verdict that grants authorization
([ADR-0016](adr/0016-restrict-only-tool-hooks.md)).

### SessionStore

Persistent session management:

```rust
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn save(&self, session: &Session) -> Result<()>;
    async fn load(&self, id: &str) -> Result<Session>;
    async fn list(&self) -> Result<Vec<SessionInfo>>;
}
```

### AgentLoop

The host provides the prompt, approval policy, event sink and cancellation token:

```rust
#[async_trait]
pub trait AgentLoop: Send + Sync {
    async fn run(
        &self,
        session: &mut Session,
        prompt: String,
        approvals: &dyn ApprovalPolicy,
        events: &dyn EventSink,
        cancellation: CancellationToken,
    ) -> Result<()>;
}
```

`AgentLoop` also provides a default `compact` method. See the core API for its
exact signature.

## Native Tools

Built-in tools registered by the Huginn runtime:

| Tool | Description | Module |
|------|-------------|--------|
| `list_files` | List workspace files | `runtime/src/tools.rs` |
| `read_file` | Read a UTF-8 file | `runtime/src/tools.rs` |
| `search_files` | Search literal text in workspace files | `runtime/src/tools.rs` |
| `write_file` | Create or replace a file; requires approval | `runtime/src/tools.rs` |
| `edit_file` | Apply a reviewed text replacement; requires approval | `runtime/src/tools.rs` |
| `shell` | Run a POSIX shell command; requires approval and is not sandboxed | `runtime/src/tools.rs` |

## MCP Integration

Huginn supports **Model Context Protocol (MCP)** servers as tool sources:

A server definition includes a command, arguments, explicit environment values,
and optional mappings from host variable names. See the configuration schema
and examples in [`mcp.rs`](../../crates/huginn/runtime/src/mcp.rs); server
configuration must not be confused with an approval or sandbox boundary.

### MCP Client Lifecycle

1. Parse server configs from `huginn.toml`
2. Spawn each server as a child process
3. Initialize MCP handshake
4. Discover tools via `tools/list`
5. Register tools in the registry
6. Monitor for tool availability
7. Clean shutdown on exit

## Event System

The agent emits structured events during execution:

```rust
pub enum Event {
    TurnStarted { session_id: String },
    Text { text: String },
    ToolStarted { call: ToolCall },
    ApprovalRequested { request: ApprovalRequest },
    ApprovalResolved { id: String, approved: bool },
    ToolFinished { id: String, output: String, is_error: bool },
    Usage { input_tokens: u64, output_tokens: u64 },
    Compacted { through: usize },
    TurnFinished,
    Error { message: String },
}
```

### Event Flow

```
User Input → AgentLoop → Model → [ToolCall] → Approval → Execute → Response
                ↓
             Event::ToolStarted → Event::ToolFinished
                ↓
             Event::TurnFinished
```

## Approval System

Before executing actions that require authorization, the application asks the host policy:

```rust
#[async_trait]
pub trait ApprovalPolicy: Send + Sync {
    async fn approve(
        &self,
        request: &ApprovalRequest,
        cancel: CancellationToken,
    ) -> Result<bool>;
}
```

### ApprovalRequest Actions

The `ApprovalRequest` contains an action identifier and a preview. Current native
mutations use `native.write_file` or `native.edit_file`; shell execution uses
`native.shell`; MCP calls identify the server and tool, for example
`mcp.github.search`. The host policy decides whether to prompt or honor an
explicit one-invocation grant. There is no fixed `FileWrite`/`BashCommand` enum
in the core contract.

## Session Persistence

`SqliteSessions` persists session records and uses a separate cross-process
advisory lease while a conversation is active. The schema is versioned; see
[`runtime/src/sessions.rs`](../../crates/huginn/runtime/src/sessions.rs) and
ADR-0009 for storage and recovery contracts. The database schema is an internal
implementation detail, not a stable public API.

## Composition

The composition root is in `apps/huginn`; runtime builders assemble providers,
tools, sessions and instructions as lifecycle-managed capabilities. For example,
`huginn_runtime::composition::build_native` creates a supervisor for a configured
model, native tool registry, session store, loop configuration and tool timeout.
The builder that accepts connection access is
`build_native_with_connections`. See
[`runtime/src/composition.rs`](../../crates/huginn/runtime/src/composition.rs)
for the current signatures and plugin assembly.

## Provider Integration

The current runtime implements two inference adapter kinds:

| Adapter kind | Description |
|--------------|-------------|
| `openai` | OpenAI-compatible endpoint |
| `ollama` | Local Ollama endpoint with local-model checks |

Anthropic, Google Gemini and Groq are not implemented as built-in Huginn
inference adapters in the current runtime. Provider connection workflows and
turn-scoped routing are documented in [providers.md](providers.md),
[connections.md](connections.md), and ADR-0014.

The CLI's current legacy configuration uses `HUGINN_BASE_URL`, `HUGINN_MODEL`,
and `HUGINN_API_KEY`; consult the [Huginn configuration guide](README.md) for
current setup details.

## Validation Evidence

The Huginn validation record documents focused test suites, architecture checks,
and known platform limitations. See [validation.md](validation.md) for dated
results. Those results are evidence for the recorded commit and environment; they
do not guarantee that the same checks pass on the current checkout.

## Related Documentation

- [README](README.md) — Usage guide and quick start
- [providers.md](providers.md) — Provider configuration
- [connections.md](connections.md) — Connection management
- [authoring.md](authoring.md) — Plugin authoring guide
- [validation.md](validation.md) — Testing and validation
