# RPI Task — dallay-628-ci-required-gate

**Route:** Direct inline (Plan Mode, RPI tracking — no SDD artifacts requested)
**Authorization:** Implement DALLAY-628 / cortex#272 slices A+B+C. Scope: `.github/workflows/ci.yml`, CI policy tests, strictly necessary operational docs. Non-goals: Rook/Agent product code, unrelated dependency upgrades, GitHub ruleset mutation.
**Branch:** `fix/dallay-628-ci-required-gate` (PR #297; rebased on `main` after CI workflow conflict with merged SonarCloud configuration PR #295)
**Routing inspection:** `.agents/rpi/plan/tasks/` missing, legacy `plan/tasks/` missing — no migration needed, creating new path.

## Route Assessment

**Surface Area:** 2-3 (sonar execution semantics, credentials policy, aggregate gate + ruleset prep — all coupled in one workflow)
**Durability:** not needed (no durable spec/design requested; temporary RPI plan suffices)
**Reversibility:** easy (workflow YAML, reversible)
**Coupling:** isolated to CI (explicit non-goals protect product code)
**Recommended Lane:** Direct inline
**Confidence:** high
**Reasoning:** Exact outcome specified in session handoff; single-file-centered change with policy tests and docs. Subagent delegation adds overhead without risk reduction. RPI task doc controls the three slices.

## Tasks

### RPI-001 Slice A — SonarCloud dependency + credentials (P0)
- Decouple `sonar` from mandatory `test` + `test-frontend` success via `always()` + result-tolerant `if`.
- Explicit step-level `SONAR_TOKEN` policy check (no `secrets` in `if`, no silent skip when required); only the scanner step receives the token.
- Fork / Dependabot-safe: no privileged execution, explicit controlled exception with log.
- Acceptance: backend-only PR runs analysis without frontend tests; frontend-only runs without Rust tests.

### RPI-002 Slice B — `CI / Required` aggregator (P0)
- Add `required` job (`CI / Required`) with `if: always()`, `needs:` all merge-relevant jobs.
- Validate expected vs actual via `needs.<job>.result` + `needs.changes.outputs.*`.
- `success` required when expected; `skipped` only when path-filter justifies; `failure/cancelled` fails gate.
- Include E2E, security (Audit, Trivy, Gitleaks, Semgrep), build, Sonar when required.
- Acceptance: intentional E2E/security/sonar failure → red gate; unexpected skip → red gate; docs-only → green with only applicable controls.

### RPI-003 Slice C — Ruleset #17001295 prep (P0, no mutation)
- Draft required-check migration: add `CI / Required`, transition plan keeping 11 existing checks green first.
- Document bypass review: `OrganizationAdmin` + Integration `915548` (`always`), solo-maintainer review constraint (keep technical gates, align human-approval policy, do not lock self out).
- Do NOT call ruleset API. Acceptance: reviewable doc + checklist, zero ruleset mutation.

### RPI-004 Verification matrix (acceptance)
- Static: YAML parse, `actionlint` if available, policy simulation for docs-only / frontend-only / backend-only / workflow-change / negative cases.
- Scanner: verify `sonarqube-scan-action` step reachable on representative PR shapes (no secrets exfiltration in logs).
- Merge blocking: document non-bypass identity check procedure (ruleset change deferred until gate green).
- Preserve existing checks during migration.
- Record actual evidence in PR body + here.

## Progress
- [x] Authorized + explored (`ci.yml` H1 confirmed: `sonar.needs=[changes,test,test-frontend]` without `always()`)
- [x] RPI-001 implemented locally (real PR Sonar execution still unverified)
- [x] RPI-002 implemented locally (real PR gate behavior still unverified)
- [x] RPI-003 transition doc prepared; ruleset unchanged
- [ ] RPI-004 actual acceptance evidence incomplete

## Evidence
- `node --test .github/scripts/ci-required.test.cjs`: 12/12 passed (docs, E2E-infrastructure-only, frontend, backend, workflow, expected E2E/security failures, missing token, fork/Dependabot policy, skipped always-on checks).
- `actionlint -color .github/workflows/ci.yml`: passed (no output), including merged `sonar-project.properties` path and full-history Sonar checkout / coverage setup from #295.
- Semgrep scan for workflow and three JS files: 0 findings, 0 errors.
- `git diff --check`: passed.
- Not yet run/verified: actual docs/frontend/backend/workflow PRs, SonarCloud scanner runs on representative PRs, negative failing-check PR, merge blocking by non-bypass identity. Ruleset #17001295 was not changed.

## Next
- Review final diffs and make corrections if needed.
- Exercise the acceptance PR matrix in GitHub, capture run IDs/URLs and scanner logs, then test merge blocking as a non-bypass identity.
- Only after successful validation, propose adding `CI / Required` to ruleset #17001295 while preserving existing checks; resolve solo-review/bypass policy before changing review requirements.
