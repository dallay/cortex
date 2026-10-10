# GitHub Issues → Linear: Cortex routing policy

**Status:** Project backfill completed on 2026-10-08; future-issue routing rule **not yet configured**.

## Scope and source of truth

- GitHub repository: [`dallay/cortex`](https://github.com/dallay/cortex).
- Linear team: `Dallay` (`DALLAY`).
- Linear project: [`cortex`](https://linear.app/dallay/project/cortex-0598ad516a88/overview) (project UUID: `1ea06171-1a67-411c-b65f-a829ece58b48`).
- Product labels: `product/rook`, `product/huginn`, `product/shared`.
  `product/agent` is the deprecated alias of `product/huginn` and is
  preserved on historical issues only — see ADR-0011. When configuring
  Linear Triage Rules, include the legacy `product/agent` label as an
  additional OR condition until historical issues are re-labeled by hand.
- GitHub is the **source for issue creation and classification labels**; Linear is the **source for project planning and triage**.
- Preserve the existing GitHub Issues Sync integration; **never create a duplicate Linear issue** for a GitHub issue already synced.

GitHub Issues Sync carries issue title, description, labels, assignee, status, and comments. It does **not** synchronize the Linear project field. Assigning the Linear project therefore requires separate Linear-side routing.

See the GitHub label taxonomy at `.github/labels.yml`, issue classifier at `.github/scripts/issue-labeler.cjs`, and [Linear GitHub integration documentation](https://linear.app/docs/github-integration).

## Completed one-time migration (2026-10-08)

All **49** existing, product-labeled Cortex issues were verified as belonging to the `cortex` project. Of these, **48** were assigned to the project during the backfill; one was already associated.

The **19** issues in `Triage` had one valid product label, one valid type label, and at least one area label and were moved to `Backlog`. Existing `Done`, `In Review`, and `Canceled` statuses were preserved.

Verified end-state at migration time:

| Status | Count |
| --- | ---: |
| Backlog | 20 |
| Done | 27 |
| In Review | 1 |
| Canceled | 1 |
| Triage | 0 |

This is a historical snapshot, not a live dashboard.

## Future intake: preferred native Linear automation

Linear [Triage Rules](https://linear.app/docs/triage) can set the project and status of issues **when they enter Triage**. As documented by Linear, this feature is available on Business and Enterprise plans.

**Manual configuration required:** the current connected Linear interface does not expose a Triage Rule creation operation.

1. Open **Linear → Settings → Teams → Dallay → Triage → Triage Rules**.
2. Create a rule named `Route Cortex GitHub issues`.
3. **Prefer an origin/repository condition** identifying synced issues from `dallay/cortex` *if that condition exists in the rule editor*. Do not apply a team-wide unconditional project assignment: team `Dallay` also owns other projects.
4. If no repository/source filter is available, use an OR condition for
   `product/rook`, `product/huginn`, `product/shared`, or the legacy
   `product/agent` (until historical re-labeling is complete), **but only
   after confirming that GitHub labels are present at the moment the
   issue enters Triage**.
5. **Action:** set **Project → cortex**.
6. **Optional separate rule:** move to **Backlog** only when the issue is already unambiguously classified: exactly one `product/*`, exactly one `type/*`, at least one `area/*`, and no `triage/needs-classification`. If the rule editor cannot express all these conditions, retain `Triage` for human review rather than routing an ambiguous issue automatically.
7. Review rule ordering and conflicts with other `Dallay` Triage Rules; they execute in configured order.
8. Save/publish the rule and record its owner and an updated screenshot or description here.

**Important sequencing limitation:** GitHub issue creation, GitHub labeler execution, and Linear synchronization are separate asynchronous operations. Linear Triage Rules run on **entry** to Triage; a rule keyed only on `product/*` labels may miss the event if labels arrive afterward. Do not claim future routing is implemented until this race has been tested. If no reliable repository-origin filter is available, prefer a dedicated event-driven Linear webhook/API integration with idempotent reconciliation and GitHub-link matching instead of a second issue-creation pipeline.

## Acceptance tests (perform after rule setup)

1. Create a GitHub issue in `dallay/cortex` using a structured Issue Form, product `Rook`, type `bug`, technical area `providers`. Confirm it creates **one** synced `DALLAY-*` issue in Linear, assigned to project `cortex`, with all three classification axes.
2. Repeat for `Huginn` (formerly `Agent`) and `Shared`.
3. Confirm a new issue in a **different** GitHub repository connected to team `Dallay` is **not** assigned to project `cortex`.
4. Confirm incomplete or ambiguous classification remains triaged unless the rule explicitly supports the complete classification requirements.
5. Exercise the **label-arrival race**: confirm routing when GitHub adds labels *after* the Linear issue was initially ingested.
6. Verify subsequent updates to the same GitHub issue do **not** create additional Linear issues or erase the project.
7. If changing status to Backlog automatically, verify GitHub's open/closed state and comment synchronization still behave as intended.

## Ownership and maintenance

- GitHub labels and issue templates: `.github/labels.yml`, `.github/ISSUE_TEMPLATE/`, `.github/issue-labeler-rules.json`.
- Linear project and status routing: Linear **Dallay Triage Rules**.
- After renaming or adding a Cortex product, update the GitHub taxonomy **and** the Linear routing condition.
- Periodically audit synced, product-labeled issues with an empty/wrong Linear project. Preserve existing assignments and completed statuses when repairing them.
- GitHub Actions must **not** hold a Linear API key merely to compensate for a native rule that has not been configured; document the tradeoff before introducing a custom integration.
