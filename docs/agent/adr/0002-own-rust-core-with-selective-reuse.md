# ADR-0002: Own Rust core with selective reuse

- Status: Accepted
- Date: 2026-10-06
- Product: Cortex agent (provisional name)
- Decision basis: Explicit choices made during the agent MVP planning conversation.

## Context

The references provide different lessons: Cordis and DeepSeek Harness inform
composition and lifecycle, Pi informs extension ergonomics, and Goose provides
a Rust agent reference.

A permanent Goose fork could deliver existing functionality quickly, but would
make the agent inherit its core architecture and upstream maintenance burden.

## Decision

Build an original Rust core and reuse suitable libraries for commodity capabilities.
Do not use a permanent Goose fork as the product foundation.

Goose, Pi and Cordis are reference systems, not compatibility targets.
Keep ownership of composition semantics, lifecycle and the agent-loop contract.
Evaluate dependency compatibility and licensing before incorporating libraries or code.

A Goose-versus-greenfield comparison spike is not a required gate for this direction.
Focused experiments remain appropriate for concrete unresolved implementation risks.

## Alternatives considered

- Goose fork: faster access to existing features, with architectural and maintenance coupling.
- Comparative spike before choosing a foundation: useful evidence, but the product direction
  is now explicitly chosen.
- Writing every dependency ourselves: increases maintenance without advancing the core thesis.

## Consequences

- The project owns more initial implementation work.
- Architecture can follow the accepted composition principles directly.
- Reuse decisions remain separate from adopting an entire upstream product.
- Reference behavior must be distinguished from project requirements.

## Follow-up and evidence

Record the origin and license of reused code. Select dependency versions during
implementation, with the repository's security and lockfile checks.
