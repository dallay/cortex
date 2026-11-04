# ADR-0014: Builtin Providers and Turn-Scoped Model Routing

- Status: Accepted
- Date: 2026-10-13
- Product: Huginn
- Decision basis: Explicit user approval to formalize the Providers direction
  and prepare five P0 deliverables and one P1 OAuth deliverable. Accepted adopts
  the direction; it does not claim implementation, verification, or product acceptance.

## Context

Huginn needs to manage multiple connections and choose models without coupling the
agent to a terminal or mingling credentials. Today `ModelProvider` and
`ModelService` define inference boundaries; composition publishes `agent:model@1`.
The `OpenAiProvider` adapter implements OpenAI-compatible streaming and
disables redirects. The current configuration constructs a provider from
CLI, TOML, and environment variables; it does not constitute a connection manager.

[ADR-0013](0013-authorable-plugin-contract-and-loading.md) establishes static native
composition, declarative contributions, and generation-scoped registrations.
Current UI callbacks are synchronous and non-blocking: asynchronous actions
and secret forms in this decision represent future work, not existing APIs.
The agent loop may infer multiple times, execute tools, and compact context
within a single turn; resolving selection on each call would cause silent model
or identity changes during a single operation.

## Decision

### Catalog, Management, and Router

Providers will be a builtin Huginn capability, statically composed per
ADR-0013, with separated responsibilities: provider/model catalog,
connection management and selection, and inference router. The router will be the
sole publisher of `agent:model@1`; adapters will register in the catalog
and will not compete for that service. Concrete identities and types of
new contracts will be defined during P0.1, without claiming existing APIs here.

Catalog, actions, registrations, and tasks will be associated with the activation
generation. Publication will be transactional: a failed activation reverts
all its effects; teardown revokes its registrations and cancels its tasks.
An identifier from an older generation cannot execute its replacement.
Activation rollback does not delete pre-existing durable connections; a
failed management operation must not leave metadata and secrets inconsistent.

`builtin.providers.core` will provide catalog, connections, and routing in
headless mode. `builtin.providers.ui` will depend on the Providers service and UI
contributions; it will register `/login`, `/providers`, `/model`, and `/logout`.
Adapters will be independent internal plugins registered in the catalog.
UI contribution will be optional: the host owns TTY, focus, keyboard, rendering,
forms, and handling of secret fields. Actions will be declarative
and asynchronous, with pending, success, error, and cancellation states; no I/O
will occur in synchronous callbacks nor will providers be required to import Ratatui internals.
Secrets will not be passed as command arguments, history, or contribution text.
This boundary does not turn trusted native extensions into isolated code or a sandbox.

### Turn-Scoped Selection and Revocation

At the start of a turn, connection identity, model, and authorized generation
are pinned. All inferences within that turn, including tool continuations and
context compaction, use that pinned configuration. Changing selection affects
the subsequent turn; there is no automatic fallback to another model, connection,
or provider on error.

Authorization is checked before each inference, not only when pinning the
turn. Revoking, deleting, or replacing connection credentials, or revoking
its generation, invalidates subsequent inferences even within the same turn;
retained credentials are not reused nor are replacements automatically adopted.
In-flight request cancellation is best effort: it may stop local consumption,
but does not prove that the server has cancelled, stopped billing, or remotely
revoked the credential. Observable state must distinguish this.
In P1, an authorized token refresh within the same authorization will not be
a connection/model change; it must be distinguished from rotation or revocation
and never restore an invalidated authorization.

### Connections, Secrets, and Precedence

Persistent connection metadata and selection will be stored separately from
secrets. Each connection has its own identity and credential reference;
sessions, TOML, logs, errors, JSON events, and UI contributions must never
receive secret values. General history cannot be assumed to lack sensitive
user-supplied information.

macOS will use Keychain. Linux will use a secure backend when available;
if missing or locked, status will be reported and only an explicit ephemeral mode
will be offered, never silent plaintext persistence. Ephemeral mode does not
promise credential recovery across restarts. Persistence, partial failures,
deletion, and reconciliation require rollback tests.

Precedence will be: explicit CLI > explicit configuration or chosen environment
> persisted selection > `unconfigured`. Within the second level, explicit
configuration retains priority over environment defaults; the environment used
must be determined and bound to the chosen connection.
Resolution validates a consistent connection: it does not mix endpoint/model
of one connection with credentials of another, nor does it complete an invalid
explicit choice by falling back to persisted selection. Without selection,
diagnostics without inference are permitted; an inference attempt fails with an
actionable error state and without secrets.
Migration of current configuration will preserve this isolation and document
provenance without presenting a new schema as already implemented.

### Transport and Authentication

Custom endpoints require validated TLS, except for explicitly authorized local
loopback for non-TLS transport. The exception is not extended to private
networks, and certificate validation is not disabled. Redirects are rejected by
default; any future policy must prevent authorization leaks to another origin
and downgrade to HTTP. Existing configuration accepting general HTTP must be
adjusted and documented in P0.4.

P0 includes an OpenAI-compatible adapter and a local Ollama adapter whose
compatibility will be verified, not inferred by name. Official OAuth is deferred
to P1, evaluated per provider and only through documented and permitted flows.
A ChatGPT account or subscription is not equivalent to credentials or OpenAI API access.
Cookies, session tokens, or private consumer product flows are not imported.
Rook, dynamic loading, hot reload, and external runtimes remain out of scope.

## Alternatives Considered

- One `agent:model@1` publisher per provider: reduces indirection, but
  causes composition collisions and couples the agent loop to connection management.
  A single router with separate catalog was chosen.
- Resolve selection per inference and apply fallback: improves availability,
  but alters identity, cost, and behavior during a turn. Turn-scoped pinning
  with explicit failure and per-call revocation checks was chosen.
- Retain pinned credentials indefinitely: eases continuity, but ignores
  revocations. Identity is preserved, not irrevocable authorization.
- Management implemented only in TUI: faster visual delivery, but prevents
  headless automation and testing. Headless use cases and optional UI with
  declarative actions were chosen.
- Secrets in TOML/SQLite or automatic fallback on Linux: simplifies deployment,
  but increases exposure and conceals lost guarantees. Secure stores and explicit
  ephemeral mode were chosen, accepting per-platform integration cost.
- General custom HTTP: facilitates remote dev servers, but exposes credentials
  and content. The exception is restricted to explicit loopback, requiring migration
  of insecure configurations.
- Generic OAuth or reusing ChatGPT authentication: gives the appearance of uniform
  onboarding, but does not prove permissions or official inference access.
  Official per-provider OAuth was deferred to P1.
- Rook integration or dynamic loader now: increases reuse and extensibility,
  but adds foreign contracts and trust models. Static integrated composition was kept.

## Consequences

- Selection is predictable and testable without network or terminal; UI does
  not block progress on core or adapters.
- Introduces generation management, revocation races, cross-store transactions,
  and locked or absent credential states.
- An error or revocation can halt a turn; availability is not prioritized
  over identity. Local cancellation does not imply remote cancellation.
- Linux security depends on available backend. Ephemeral mode and recovery
  failures must be observable before inferring.
- Automated test scope is not equivalent to onboarding acceptance or validation
  of all real provider combinations.

## Tracking and Evidence

Inspected baseline: [`application/src/composition.rs`](../../../crates/huginn/application/src/composition.rs),
[`runtime/src/model.rs`](../../../crates/huginn/runtime/src/model.rs),
[`core/src/kernel.rs`](../../../crates/huginn/core/src/kernel.rs),
[`plugin-api/src/v1.rs`](../../../crates/huginn/plugin-api/src/v1.rs),
[`config.rs`](../../../apps/huginn/src/config.rs), and
[`config.example.toml`](../../../apps/huginn/config.example.toml).

Planned deliverables, not implemented by this record:

1. P0.1: contracts, catalog, and router; headless demo with test doubles and
   turn-pinning, rollback, and revocation tests.
2. P0.2: headless connections, secrets, and precedence on P0.1; isolation,
   partial failure, restart, and secret leakage tests.
3. P0.3: independent UI on P0.1/P0.2; doubles until P0.4 is available,
   secret forms, cancellable actions, and stale generations.
4. P0.4: OpenAI-compatible and local Ollama adapters on P0.1/P0.2;
   streaming, tools, secure transport, and verified compatibility.
5. P0.5: automated integration of P0.1–P0.4, from selection to turns
   and recovery, with rollback, revocation, restart, and secret leakage.
6. P1: official per-provider OAuth on integrated P0 flow; renewal,
   expiration, revocation, and absence of consumer product credentials.

Engineering tracking created in GitHub:
[P0.1 / #350](https://github.com/dallay/cortex/issues/350),
[P0.2 / #351](https://github.com/dallay/cortex/issues/351),
[P0.3 / #352](https://github.com/dallay/cortex/issues/352),
[P0.4 / #353](https://github.com/dallay/cortex/issues/353),
[P0.5 / #354](https://github.com/dallay/cortex/issues/354), and
[P1 / #355](https://github.com/dallay/cortex/issues/355).
Opening issues does not demonstrate implementation.

Concrete types, metadata format, Linux backend, reconciliation policy,
and Ollama matrix will be finalized with evidence in those deliverables.
They are not implementation decisions settled by this ADR.

QA issues [#276](https://github.com/dallay/cortex/issues/276),
[#332](https://github.com/dallay/cortex/issues/332),
[#307](https://github.com/dallay/cortex/issues/307), and
[#280](https://github.com/dallay/cortex/issues/280) remain independent:
this ADR and its deliverables do not close them or treat them as blockers for
developing Providers. Onboarding acceptance is a separate gate;
deliverable tests provide evidence but do not replace that gate.
