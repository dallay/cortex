# Labeling policy

This is the contract for automatic issue classification in `dallay/cortex`.
The system is declarative and deterministic. No LLM is used. Consistency beats free interpretation.

## Taxonomy

Source of truth: `.github/labels.yml` (synced by `labels-sync.yml`, non-destructive).

- `product/*` — owning product, exactly one per issue: `rook`, `agent`, `shared`.
  PRs may touch several products; issues never do.
- `area/*` — technical areas, zero or more on PRs, at least one on classified issues:
  `auth`, `ci`, `core`, `dashboard`, `dependencies`, `docs`, `observability`,
  `providers`, `release`, `runtime`, `storage`, `testing`, `transport`.
- `type/*` — work type, exactly one per classified issue:
  `bug`, `feature`, `enhancement`, `chore`, `test`, `milestone`, `performance`.
- `priority/*` — `high`, `low`. Owned by humans; the classifier never touches them.
- Operational — `security`, `stale`. Owned by their workflows; the classifier never touches them.
- `triage/needs-classification` — needs a human. Added when information is missing
  or contradictory; removed once the issue is fully classified.

## Classification priority

1. Issue Form body (`### Product`, `### Technical Area`). Preferred source for new issues.
2. Conventional title `prefix(scope):`. Scope gives product, prefix gives type.
3. Existing valid labels. A valid human decision is preserved; it is not overwritten
   by weak keyword matches.
4. Triage. When information is missing or contradictory, add
   `triage/needs-classification`. Never invent a responsible product.

Product is never inferred from body keywords. A body like
“Agent integration with Rook” must triage, not assign two products.

## Conventional titles

Format: `prefix(scope): subject`, for example `fix(agent): improve MCP session recovery`.

Scopes (must be exactly one): `rook`, `agent`, `shared`.

| Prefix | Type |
| --- | --- |
| `fix` | `type/bug` |
| `feat`, `feature` | `type/feature` |
| `chore`, `docs` | `type/chore` |
| `test`, `tests` | `type/test` |
| `perf`, `performance` | `type/performance` |
| `refactor`, `enhancement`, `improvement` | `type/enhancement` |
| `milestone` | `type/milestone` |

Examples:

- `fix(agent): improve MCP session recovery` → `product/agent` + `type/bug` + `area/runtime`
- `feat(rook): add Anthropic fallback` → `product/rook` + `type/feature` + `area/providers`
- `docs(shared): labeling policy` → `product/shared` + `type/chore` + `area/docs`

## Issue Forms

`blank_issues` is `false`. New issues must use a form:

- `bug.yml` presets `type/bug`
- `feature.yml` presets `type/feature`
- `technical-task.yml` presets `type/chore`

Each form requires `Product` and `Technical Area`. GitHub renders answers as
`### Product` and `### Technical Area` in the issue body; the classifier reads them
case-insensitively. If a user edits the body and breaks the headings, the classifier
falls back to the title and then to existing labels.

## Preservation and safety

- Idempotent: running the classifier twice produces no additional changes.
- Managed prefixes only: `product/`, `type/`, `area/`, `triage/`.
  `priority/*`, `security`, `stale`, and Renovate-managed `area/dependencies`
  plus any unmanaged label are never added or removed by ordinary classification.
- Exclusivity: an issue never ends with `product/rook` and `product/agent` together.
  An explicit product (form or title scope) may replace the previous one;
  an ambiguous mention may not.
- Areas are additive: the classifier only adds missing `area/*`, never removes.
- Types preserve humans: a single valid existing `type/*` wins over a stale title prefix.
  Titles only fill a missing type or resolve zero-or-many into one.
- `GITHUB_TOKEN` edits labels only, never the body, so `opened/edited/reopened`
  does not recurse (`labeled` does not trigger the workflow).

## Exceptions

- Renovate `Dependency Dashboard` → `product/shared` + `area/dependencies` + `type/chore`.
  It has no form and no conventional scope by design.
- Forks: `pr-labeler.yml` stays on `pull_request`, not `pull_request_target`.
  Do not change this without a security review.
- Multi-product PRs are normal (path-based). Multi-product issues are a bug; triage them.

## Weekly audit

`labels-audit.yml` runs Mondays 09:00 UTC and on demand. It scans open issues
(excluding PRs) and fails with a summary table when it finds:

- missing or multiple `product/*`
- missing or multiple `type/*`
- missing `area/*`
- invalid managed values (for example `product/unknown`)
- undocumented labels (not in `labels.yml` and not preserved)

Fix by setting exactly one valid `product/*`, one `type/*`, at least one `area/*`,
and either removing undocumented labels or adding them to `.github/labels.yml`.

## Maintenance

- Rules: `.github/issue-labeler-rules.json` (versioned, `version` bump on breaking changes).
- Classifier: `.github/scripts/issue-labeler.cjs` (`classify()` pure, `run()` for `github-script`).
- Tests: `.github/scripts/issue-labeler.test.cjs`, run `node --test .github/scripts/issue-labeler.test.cjs`.
  Cover forms, titles, Renovate, multi-product mentions, safe corrections, idempotence.
- Manual re-run: `Auto Label Issues` → `workflow_dispatch` with `issue_number`.
- Actions are pinned by tag and maintained by Renovate (`github-actions` manager).
  Prefer SHA pinning with a `# vX` comment for production, as `labels-sync.yml` does.
- Adding a product, area, or type: update `labels.yml` first, then `issue-labeler-rules.json`
  (`products`/`areas`/`types`, `scopes`, `conventional`, `areaKeywords`), then forms,
  then tests. Keep dropdown options and rules in sync.
- Renovate labels must always use documented names (today: `area/dependencies`, plus
  `area/ci` for workflow updates). Never reintroduce bare `dependencies`, `rust`,
  `javascript`, `github-actions`, or `docker` labels.

## Files

```text
.github/
├── labels.yml
├── labeler.yml
├── issue-labeler-rules.json
├── scripts/issue-labeler.cjs
├── scripts/issue-labeler.test.cjs
├── ISSUE_TEMPLATE/config.yml
├── ISSUE_TEMPLATE/bug.yml
├── ISSUE_TEMPLATE/feature.yml
├── ISSUE_TEMPLATE/technical-task.yml
└── workflows/issue-labeler.yml
└── workflows/labels-audit.yml
```
