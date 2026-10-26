# Canonical product contexts

Global principles live in `.agents/AGENTS.md`. Product contexts specialize that
contract and cannot weaken its invariants. Edit the canonical Markdown here;
AgentSync manages the destination links through `.agents/agentsync.toml`.
All repository paths in the contexts are interpreted from the repository root,
except dashboard-local paths explicitly identified in `dashboard.md`.

## Scope and inheritance

| Context | Scope |
| --- | --- |
| `rook.md` | `apps/rook`, `rook-core`, `rook-usecases`, and explicitly mapped Rook infrastructure crates |
| `huginn.md` | `apps/huginn` and `crates/huginn` |
| `dashboard.md` | `apps/rook/dashboard`, in addition to the global and Rook contracts |

`shared-kernel`, `test-support` and the generic `sse-stream` utility receive only
root instructions. Infrastructure mappings name individual crates; new crates
must be classified deliberately rather than inheriting Rook rules automatically.
When adding a scope, update all native-agent mappings and its Copilot `applyTo`
patterns together. Keep product-neutral abstractions outside product contexts.

The same canonical files are projected as nested `AGENTS.md`, `CLAUDE.md` and
`GEMINI.md`. The root target owns nested `AGENTS.md` for tools that use that
convention; Codex does not need a second target managing the same paths.
Copilot receives `.github/instructions/<context>.instructions.md` links, using
`applyTo` frontmatter to select the same explicit scopes. Dashboard files match
both Rook and dashboard patterns. Do not create overlapping destination owners.

AgentSync creates links; each assistant controls instruction discovery and
inheritance. A successful sync alone does not prove that a running assistant has
loaded the contexts. Restart or refresh its context when necessary.

## Verification

From the repository root:

```sh
pnpm exec agentsync apply --dry-run
pnpm agents:sync
pnpm exec agentsync status
pnpm exec markdownlint-cli2 ".agents/AGENTS.md" ".agents/contexts/*.md"
```

Inspect the dry run for collisions, unexpected backups and destinations outside
the intended scopes before applying. Check that each generated link resolves to
its canonical file. Repeat the dry run after applying; it should propose no new
or updated context links. Separate existing unrelated status issues from new ones.

For a discovery smoke check in each assistant, open a fresh session scoped to
`crates/domain/rook-core` and another to `crates/huginn/application`. Ask it to
identify the instruction files loaded and applicable architecture, security and
TDD rules. Rook should receive the root and Rook contracts; Huginn should receive
the root and Huginn contracts. Repeat in the dashboard to check all three levels,
and in `shared-kernel` to check that neither product context applies.

For behavior evaluation, use read-only review prompts for an authorization bug,
a gateway optimization and a TUI approval interaction. Expected criteria are
authorization before effects with regression tests, a measured performance
baseline, and keyboard/focus/denial behavior respectively. These are manual
acceptance scenarios, not evidence of completed code changes or automated tests.

## References

- [AgentSync sync types](https://dallay.github.io/agentsync/concepts/sync-types/)
- [Copilot repository and path instructions](https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/add-custom-instructions/add-repository-instructions)
- [Gemini hierarchical memory discovery](https://geminicli.com/docs/core/)
