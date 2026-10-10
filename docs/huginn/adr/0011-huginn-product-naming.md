# ADR-0011: Huginn Product Naming and Identity

- Status: Accepted
- Date: 2026-10-10
- Product: Huginn (Cortex coding agent)
- Decision basis: Explicit decision after the personal MVP reached daily-use readiness; supersedes the "product name still open" statement in `docs/huginn/adr/README.md` and the "provisional internal identifier" wording in historical ADRs (the historical decisions themselves remain valid).

## Context

The coding agent inside the Cortex monorepo had been referred to as
`agent` (binary), `cortex-agent` (Cargo package), `agent-core`,
`agent-runtime`, `cortex/agent` (config dir) and `AGENT_*` (env vars).
ADR-0001 explicitly stated that the public product name remained open and
that `agent` was a provisional internal identifier. Several weeks of
daily-use validation and structured acceptance have now passed
(see [`../validation.md`](../validation.md)), so the team is ready to
commit to a stable identity before any public packaging or external
distribution is considered.

## Decision

1. The coding agent's official name is **Huginn**.
2. The future persistent memory product will be named **Muninn**.
3. Both names originate from Odin's two ravens in Norse mythology:
   - Huginn (Old Norse: "thought") — autonomous reasoning and orchestration.
   - Muninn (Old Norse: "memory") — durable knowledge and recall.
4. Huginn and Muninn are distinct products or capabilities within the
   broader Cortex ecosystem; this ADR establishes identity, not a
   shared architecture.
5. Naming collisions with other open-source projects are acknowledged
   and accepted at this project stage; no rename, alternate identity, or
   blocked implementation is planned because of them.
6. A future rename is an option only if actual adoption creates
   material branding problems (trademark dispute, registry takedown,
   install confusion, or similar).
7. This ADR establishes product identity; it does not define or implement
   Muninn's architecture, persistence, or release path.

## Concrete artefact & environment mapping

This decision is implemented by the migration accompanying this ADR:

| Existing            | Canonical (post-ADR)        |
|---------------------|-----------------------------|
| `apps/agent/`       | `apps/huginn/`              |
| `crates/agent/core/` | `crates/huginn/core/`      |
| `crates/agent/runtime/` | `crates/huginn/runtime/` |
| `docs/agent/`       | `docs/huginn/`              |
| `cortex-agent`      | `huginn` (Cargo package)    |
| `agent-core`        | `huginn-core`               |
| `agent-runtime`     | `huginn-runtime`            |
| `agent` (binary)    | `huginn`                    |
| `cortex/agent/config.toml` | `cortex/huginn/config.toml` (preferred) |
| `cortex/agent/sessions.db` | `cortex/huginn/sessions.db` (preferred) |
| `AGENT_BASE_URL`    | `HUGINN_BASE_URL`           |
| `AGENT_MODEL`       | `HUGINN_MODEL`              |
| `AGENT_API_KEY`     | `HUGINN_API_KEY`            |
| `scripts/agent-linux-smoke.sh` | `scripts/huginn-linux-smoke.sh` |
| `agent>` prompt     | `huginn>`                   |

`AGENT_*` variables and the legacy `cortex/agent/config.toml` location
remain readable as a one-release compatibility fallback. New installs use
the canonical paths and variables. `--config` / `--db` overrides still
take priority and are never silently overwritten.

The session lock filename deliberately remains
`cortex-agent-session-<uuid>.lock` for cross-version mutual exclusion.
Using separate old and new lock paths would let the two executables hold
different locks while mutating the same SQLite file. New versions continue
to acquire the old lock name until the migration window is safely closed.
No live SQLite database is copied or migrated through a plain file copy
operation.

## Product classification and tooling

- `product/huginn` becomes the canonical GitHub product label.
- `product/agent` is preserved as a non-managed historical label so
  existing issues keep their classification; the issue classifier
  canonicalises the legacy `agent` keyword (form value and conventional
  scope) to `product/huginn` for new classifications.
- The CI path filter recognises `apps/huginn/**`, `crates/huginn/**`,
  `docs/huginn/**`, and `scripts/huginn-*.sh` as backend changes.
- Huginn's initial supported CI targets remain macOS and Linux; Rook's
  Windows validation is unchanged.

## Alternatives considered

- **Continue with `agent` as a permanent placeholder.** Rejected: the
  personal MVP has reached the maturity needed to commit to a real name,
  and a longer provisional period adds friction for documentation, CI
  labels, and external packaging work that will follow.
- **Pick a different name (e.g. `cortex-coder`, `raven`, `munin`).**
  Rejected: Huginn has been the working title in research notes and the
  team has accepted the Norse mythology basis. A second naming round
  delays the migration without changing the collision trade-off.
- **Brand-availability study and trademark search.** Rejected at this
  stage: the team has consciously accepted the collision risk and will
  revisit only if actual adoption creates material conflict.

## Consequences

- Huginn's identity is now stable enough for documentation, CI labels,
  and external packaging work to reference a single canonical name.
- Historical artefacts (`docs/huginn/adr/0001-0010.md`,
  `.agents/sdd/changes/archive/**`, release notes) still mention the
  legacy identifiers where doing so preserves the historical record;
  this ADR does not rewrite historical decisions.
- The legacy compatibility layer is intentionally narrow: env-var
  fallback, legacy config-file read fallback, and lock-filename
  recognition only. No automated migration of legacy SQLite databases
  is performed; the schema is unchanged and the legacy databases are
  read in place.
- `Muninn` is not implemented by this ADR. Future work that introduces
  Muninn must create a new ADR defining its architecture and its
  relationship with Huginn.

## Follow-up and evidence

- Migration evidence: `cargo check --workspace`, `cargo test -p
  huginn-core -p huginn-runtime -p huginn`, `cargo run -p huginn --
  --provider mock doctor`, and `just huginn-test` pass after the change.
- Operational compatibility acceptance criterion: `scripts/huginn-linux-smoke.sh`
  must succeed on a clean Linux host without contacting a model endpoint.
- Add immutable evidence references (commit, Cargo manifest, label audit
  run) when this ADR is archived.
- Revisit only if naming conflict, trademark complaint, or
  install-distribution friction makes a future rename materially
  worthwhile.