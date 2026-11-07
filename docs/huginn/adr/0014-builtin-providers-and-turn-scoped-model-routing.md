# ADR-0014: Built-in providers and turn-scoped model routing

- Status: Accepted
- Date: 2026-10-13
- Product: Huginn
- Decision basis: explicit user approval to formalize the Providers
  direction and prepare five P0 deliveries and one P1 OAuth delivery. Accepted accepts the
  direction; it does not declare implementation, verification, or product acceptance.

## Context

Huginn needs to manage several connections and choose models without coupling the
agent to a terminal or mixing credentials. Today `ModelProvider` and
`ModelService` delimit inference; the composition publishes `agent:model@1`.
The `OpenAiProvider` adapter implements OpenAI-compatible streaming and
disables redirects. The current configuration builds a provider from
CLI, TOML, and environment variables; it does not constitute a connection manager.

[ADR-0013](0013-authorable-plugin-contract-and-loading.md) establishes static
native composition, declarative contributions, and per-generation registrations.
Current UI callbacks are synchronous and non-blocking: the asynchronous actions
and secret forms covered by this decision are future work, not current APIs.
The loop can infer multiple times, run tools, and compact context
within a turn; resolving selection on each call would produce silent
model or identity changes during the same operation.

## Decision

### Catalog, management, and router

Providers will be a built-in Huginn capability, statically composed per
ADR-0013, with separated responsibilities: provider/model catalog,
connection management and selection, and inference router. The router will be the
sole publisher of `agent:model@1`; adapters will register in the
catalog and will not compete for that service. The concrete identities and types of
the new contracts will be defined during P0.1; this document does not declare existing APIs.

Catalog, actions, registrations, and tasks will be associated with the activation
generation. Publication will be transactional: a failed activation reverts
all its effects; teardown revokes its registrations and cancels its tasks.
An identifier from an old generation cannot execute its replacement.
Activation rollback does not delete pre-existing durable connections; a
failed management operation also cannot leave metadata and secrets inconsistent.

`builtin.providers.core` will provide catalog, connections, and routing in
headless mode. `builtin.providers.ui` will depend on the Providers service and on
the UI contributions; it will register `/login`, `/providers`, `/model`, and `/logout`.
Adapters will be independent internal plugins registered in the catalog.
The UI contribution will be optional: the host owns TTY, focus, keyboard, rendering,
forms, and handling of secret fields. Actions will be declarative
and asynchronous, with pending, success, error, and cancellation states; no I/O will be
done in synchronous callbacks, and providers will not be required to import Ratatui internals.
Secrets will not be introduced as command arguments, history, or contribution
text. This boundary does not turn trusted native extensions into
isolated code or a sandbox.

### Turn-scoped selection and revocation

At the start of a turn, the connection, model, and authorized generation
identity are fixed. All its inferences, including continuation after tools and
compaction, use that fixed selection. Changing the selection affects the next turn;
there is no automatic fallback to another model, connection, or provider on errors.

Authorization is checked before each inference, not only when the turn is
fixed. Revoking, deleting, or replacing the connection's credentials, or revoking
its generation, invalidates subsequent inferences even within the same
turn; a retained credential is never reused, nor is its replacement adopted.
Cancelling a request in progress is best effort: it can stop local consumption,
but it does not prove that the server canceled, stopped billing, or
remotely revoked the credential. Observable state must distinguish this.
In P1, an authorized token refresh within the same authorization will not be
a connection/model change; it must be distinguished from rotation or revocation and
must never restore an invalidated authorization.

### Connections, secrets, and precedence

Persistent connection and selection metadata will be stored separately from
secrets. Each connection has its own identity and a reference to its
credentials; neither sessions, TOML, logs, errors, JSON events, nor UI
contributions may receive secret values. It is not assumed that the general history
lacks sensitive information provided by the user.

On macOS, Keychain will be used. On Linux, a secure backend will be used when
available; if it is missing or locked, the state is reported, and only an
explicit ephemeral mode is offered, never silent plain-text persistence. The
ephemeral mode does not promise credential recovery after a restart. Persistence,
partial failures, deletion, and reconciliation need rollback tests.

Precedence will be: explicit CLI > explicit configuration or chosen environment
> persisted selection > `unconfigured`. Within the second level, explicit
> configuration retains priority over environment defaults;
> the environment used must be determined and associated with the chosen connection.
> Resolution validates a coherent connection: it does not combine one connection's endpoint/model
> with another connection's credentials, nor does it complete an invalid explicit choice by falling back
> to the persisted selection. Without a selection, diagnostics without inference are allowed;
> an inference attempt fails with an actionable state and without secrets.
> Migration of the current configuration will preserve this isolation and explain
> its provenance without presenting a new schema as already implemented.

### Transport and authentication

Custom endpoints require validated TLS, except for local loopback
explicitly authorized for transport without TLS. The exception is not extended
to private networks, nor is certificate validation disabled. Redirects
are rejected by default; any future policy must prevent authorization
leaks to another origin and downgrades to HTTP. The existing configuration that
accepts general HTTP must be adapted and documented in P0.4.

P0 includes an OpenAI-compatible adapter and a local Ollama adapter whose
compatibility will be verified, not inferred from the name. Official OAuth is left
for P1, evaluated per provider and only through documented and permitted flows.
A ChatGPT account or subscription is not equivalent to API credentials or access to the
OpenAI API. No cookies, session tokens, or private consumer-product flows are
imported. Rook, dynamic loading, hot reload, and external runtimes are out of scope.

## Alternatives considered

- A per-provider publisher of `agent:model@1`: reduces intermediation, but
  creates composition collisions and couples the loop to connection management.
  A single router with a separate catalog is chosen.
- Resolving selection per inference and applying fallback: improves availability,
  but changes identity, cost, and behavior during a turn. A per-turn
  fixed selection with explicit failure and checked revocation on each call is chosen.
- Retaining the fixed credential indefinitely: eases continuity, but
  ignores revocations. Identity is retained, not irrevocable authorization.
- Management implemented only in the TUI: faster visual delivery, but prevents
  automation and tests without a terminal. Headless use cases and optional
  UI with declarative actions are chosen.
- Secrets in TOML/SQLite or automatic fallback on Linux: simplifies deployment,
  but increases exposure and hides loss of guarantees. Secure stores
  and an explicit ephemeral mode are chosen, at the cost of per-platform integration.
- General custom HTTP: eases remote development servers,
  but exposes credentials and content. The exception is restricted to explicit
  loopback, at the cost of migrating insecure configurations.
- Generic OAuth or reusing ChatGPT authentication: appears to give uniform
  onboarding, but does not prove permissions or official access to inference.
  Official per-provider OAuth is deferred to P1.
- Rook integration or a dynamic loader now: expands reuse and extensibility,
  but adds contracts and trust outside the goal. Built-in, static
  composition is kept.

## Consequences

- Selection is predictable and testable without network or terminal; the UI does
  not block progress on the core or the adapters.
- Generation management, revocation races, cross-store transactions,
  and blocked or missing credential states are introduced.
- An error or revocation can stop the turn; availability is not prioritized
  over identity. Local cancellation does not imply remote cancellation.
- Linux security depends on the available backend. Ephemeral mode and
  recovery failures must be visible before inferring.
- The scope of automated tests does not equal acceptance of onboarding
  nor validation of every combination of real providers.

## Follow-up and evidence

Inspected baseline: [`application/src/composition.rs`](../../../crates/huginn/application/src/composition.rs),
[`runtime/src/model.rs`](../../../crates/huginn/runtime/src/model.rs),
[`core/src/kernel.rs`](../../../crates/huginn/core/src/kernel.rs),
[`plugin-api/src/v1.rs`](../../../crates/huginn/plugin-api/src/v1.rs),
[`config.rs`](../../../apps/huginn/src/config.rs), and
[`config.example.toml`](../../../apps/huginn/config.example.toml).

Planned deliveries, not implemented by this record:

1. P0.1: contracts, catalog, and router; headless demonstration with doubles and
   tests for per-turn fixing, rollback, and revocation.
2. P0.2: connections, secrets, and headless precedence over P0.1; tests for
   isolation, partial failures, restart, and absence of leaks.
3. P0.3: UI independent of P0.1/P0.2; doubles until P0.4 is available,
   secret forms, cancelable actions, and obsolete generations.
4. P0.4: OpenAI-compatible and local Ollama adapters over P0.1/P0.2;
   streaming, tools, secure transport, and verified compatibility.
5. P0.5: automated integration of P0.1–P0.4, from selection through turns
   and recovery, with rollback, revocation, restart, and secret leakage checks.
6. P1: official per-provider OAuth over the integrated P0 flow; renewal,
   expiration, revocation, and absence of consumer-product credentials.

Engineering tracking created on GitHub:
[P0.1 / #350](https://github.com/dallay/cortex/issues/350),
[P0.2 / #351](https://github.com/dallay/cortex/issues/351),
[P0.3 / #352](https://github.com/dallay/cortex/issues/352),
[P0.4 / #353](https://github.com/dallay/cortex/issues/353),
[P0.5 / #354](https://github.com/dallay/cortex/issues/354), and
[P1 / #355](https://github.com/dallay/cortex/issues/355).
Opening the issues does not attest to implementation.

The concrete types, metadata format, Linux backend, reconciliation
policy, and Ollama matrix will be made concrete with evidence in those deliveries.
They are not implementation decisions closed by this ADR.

QA items [#276](https://github.com/dallay/cortex/issues/276),
[#332](https://github.com/dallay/cortex/issues/332),
[#307](https://github.com/dallay/cortex/issues/307), and
[#280](https://github.com/dallay/cortex/issues/280) remain independent:
this ADR and its deliveries do not close them nor turn them into blockers for
developing Providers. Onboarding acceptance is a separate gate;
delivery tests provide evidence, they do not replace that gate.
