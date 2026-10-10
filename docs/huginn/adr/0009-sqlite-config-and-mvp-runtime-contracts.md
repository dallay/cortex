# ADR-0009: SQLite, explicit configuration and MVP runtime contracts

- Status: Accepted
- Date: 2026-10-06
- Product: Cortex agent (provisional name)
- Decision basis: Approved MVP plan and implementation choices within that scope.

## Context

ADR-0005 through ADR-0008 established model boundaries, trust and durable context.
Implementation requires concrete storage, configuration and compatibility choices.
The approved implementation plan specifies SQLite and summarization with the same
provider, refining the earlier proposals in ADR-0008.

## Decision

Use SQLite with a versioned schema and an original-history JSON record per session.
Persist a separate summary and boundary, and guard live sessions with cross-process
advisory locks. Unknown future schema versions fail; interrupted effects never replay.

Use TOML user configuration and CLI overrides. Resolve model credentials from a named
environment variable, never from session history. A mock provider supports offline
diagnostics; an OpenAI-compatible provider supports actual model calls. The CLI grants
exact action names with --allow for one invocation; there are no wildcard grants or
restored permissions. MCP children inherit only explicitly configured variables.

Pin the official rmcp client to 3.5.1 for the workspace's Rust 1.89 toolchain. Keep
stdio processes trusted, with launch approval, call approval and bounded shutdown.
Load scoped AGENTS.md context and summarize older complete turns with the selected
provider. Stop safely when conservative context accounting cannot fit the request.

Detailed contracts and limits live in the implementation specification. A public
agent name, distribution tags, real-endpoint compatibility and Linux daily-use
acceptance remain separate milestones.

## Alternatives considered

- Append-only files: simpler inspection, more custom locking and indexing logic.
- Normalized message/event tables: better large-session efficiency, more schema work.
- Model tokenizers: more accurate budgets, provider-specific dependency and mapping.
- Persistent broad permission grants: less interaction, weaker control over effects.

## Consequences

The storage is deliberately simple and atomic but rewrites the session record on
updates. Long-session performance needs measurement before scaling beyond personal
use. Conservative budgeting can compact early or reject large tool contexts.
Credentials supplied to a provider are excluded from persistence; user prompts and
repository text still remain in the original history.

## Follow-up and evidence

See the [implementation specification](../implementation-specification.md),
[usage guide](../README.md) and [validation record](../validation.md).

## Runtime ownership invariant

The advisory lock is a runtime ownership contract: at most one executor may
control a persisted session at a time. It is not a substitute for SQLite's
transactional integrity, nor is it merely a database-write lock. The owner
acquires the lock before loading the state for a resumed execution and keeps
it through the final persistence step. Independent sessions may execute
concurrently. Subagents sharing one conversation require a separate
coordinator design; they do not bypass this invariant.
