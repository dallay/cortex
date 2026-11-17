# Acceptance QA Report: muninn-h5-acceptance-protocol

## Identity

- Change: `muninn-h5-acceptance-protocol`
- Mode: openspec
- QA phase: Phase 11 acceptance gate
- Date: 2026-11-09

## Sources of Truth

- Proposal: `.agents/sdd/changes/muninn-h5-acceptance-protocol/proposal.md`
- Specifications: `.agents/sdd/changes/muninn-h5-acceptance-protocol/specs/retrieval-acceptance/spec.md`
- Design: `.agents/sdd/changes/muninn-h5-acceptance-protocol/design.md`
- Tasks: `.agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md`
- Technical verification: `.agents/sdd/changes/muninn-h5-acceptance-protocol/verify-report.md` — PASS WITH WARNINGS for artifact structure and provenance checks only; not product acceptance.
- Fresh evidence supplied for this QA: protocol candidate validator is non-ready; corpus validator and canonical hash pass; source provenance mappings and hashes match baseline Git objects; schema and artifact hygiene checks pass.

## Target and Environment

- Target: Phase 11 review-only H5 protocol candidate and independent, unlabelled corpus candidate. There is no approved product execution or acceptance target.
- Environment: Local worktree and offline artifact checks. Fresh evidence records one query with ten candidates; 11 unique, complete provenance mappings whose source SHA-256 values match baseline Git objects at `09bb21d5fd2c847ac7df5e48f7be06eb08cec445`; all source paths are in the allowed inventory; canonical corpus hash and corpus validation pass; protocol JSON Schema passes with only a CLI deprecation warning; JSON parsing, Markdownlint (0 issues), and `git diff --check` pass. An independent reviewer reported no findings.
- Credentials/permissions: User authorized drafting only. No measurement, labels, freeze, or product acceptance is authorized. Protocol inputs and approvals remain pending.
- Limitations: Artifact checks establish structural validity and source-byte consistency only. The corpus is one query and ten unlabelled candidates, remains draft/not frozen, and is not representative or confirmed sufficient. Source text/path review and sensitivity review remain pending. No labels exist. The prior Clippy `-D warnings` failure remains (93 library / 120 all-target test errors) and was not rerun.

## Capability Inventory

| Capability | Availability | Selected? | Rationale / rejection reason |
|---|---|---|---|
| Offline protocol candidate validation | available | selected | Fresh evidence: candidate is structurally valid but `confirmatory_ready=false`; this is artifact evidence, not acceptance. |
| Corpus validation and canonical hash | available | selected | Fresh evidence passes for one query with ten candidates; does not establish corpus adequacy, representativeness, relevance, or acceptance. |
| Baseline source mapping/hash verification | available | selected | Fresh evidence: all 11 mappings are unique and complete, allowed by inventory, and SHA-256 values match baseline Git objects. Does not constitute source review or sensitivity approval. |
| Protocol JSON Schema and JSON parsing | available | selected | Fresh evidence passes; schema CLI emitted only a deprecation warning. Structural validation does not approve decisions. |
| Markdownlint and `git diff --check` | available | selected | Fresh evidence: Markdownlint 0 issues and diff check passes; hygiene checks are not product acceptance. |
| Independent reviewer | available | selected | No findings reported; independent review does not supply owner approval, corpus freeze, labels, or acceptance. |
| Retrieval, search, inference, or measurement | unavailable | rejected | User authorized drafting only; no measurement or labels. No retrieval/search or model inference was run. |
| Docker, bridge, or database | unavailable | rejected | Explicitly not run; not authorized or needed for this artifact-only acceptance gate. |
| Human source/content and sensitivity review | unavailable | rejected | Owner/source review and project sensitivity review remain pending. No approval may be inferred from provenance or license checks. |
| Labeling, adjudication, and corpus freeze | unavailable | rejected | No labels exist; the candidate remains draft/not frozen and these actions are outside authorization. |
| Browser, responsive, accessibility, and locale checks | not applicable | rejected | Target is review-only protocol/corpus artifacts, not a user interface. |
| Clippy `-D warnings` | unavailable | rejected | Not rerun in this QA; prior failure remains 93 library / 120 all-target test errors. |
| SDD deterministic quality runner | unavailable | rejected | Runner is not configured; execution mode is `fallback`, with no deterministic enforcement claim. |

## Scenario Matrix

| ID | Capability | Acceptance scenario | Result | Evidence or reason |
|---|---|---|---|---|
| QA11-01 | Protocol artifact validation | The protocol candidate records accepted ANN–exact decisions: `ann_exact=true`, `K=10`, and both `ExactTopKRecall@10=1.0` and `OrderedTopKMatch@10=1.0` per query; one run; per-query reporting; descriptive-only macro-average; observed ties without retrospective tie-breaking; undefined required metrics block `PASS`. | PASS | Fresh validator evidence and technical verification confirm these candidate fields. This is conformance of a review-only artifact, not a measured product outcome or protocol-run approval. |
| QA11-02 | Protocol artifact validation | Absolute relevance remains disabled and no relevance threshold or labels are inferred. | PASS | Candidate records `absolute_relevance=false`; no labels exist and relevance approval remains pending. |
| QA11-03 | Protocol readiness and approval | A candidate protocol cannot authorize execution while corpus scope, ANN comparison scope, identity/provenance, repository approval, and gate approval remain unresolved. | BLOCKED | Validator reports `confirmatory_ready=false`; these protocol fields/approvals remain pending. No measurement is authorized. Structural validation is not product acceptance. |
| QA11-04 | Corpus validation and provenance | The candidate corpus is valid, canonically hashed, and every ID has a unique, complete source mapping to an eligible baseline source with matching source hash. | PASS | Fresh evidence: corpus validator and canonical hash pass; all 11 mappings are unique/complete; all paths are allowed and SHA-256 source hashes match baseline Git objects at `09bb21d5fd2c847ac7df5e48f7be06eb08cec445`. This proves artifact/provenance consistency only. |
| QA11-05 | Corpus acceptance / owner review | The corpus is accepted as sufficient and representative for the intended evaluation. | BLOCKED | Only one query with ten unlabelled candidates exists. It is a minimal draft review sample, not representative or confirmed sufficient. Owner review and explicit freeze are pending. Do not infer acceptance from meeting the ten-candidate structural minimum. |
| QA11-06 | Human source/content and sensitivity review | Source paths and text are reviewed and approved for suitability and sensitivity before freeze. | BLOCKED | Source review and sensitivity review remain pending; project sensitivity classification is absent. Mapping/hash validation is not review approval. |
| QA11-07 | Labels and absolute relevance | Human labels, rubric, and adjudication support an absolute relevance gate. | BLOCKED | No labels exist; absolute relevance is false. Labeling, rubric approval, and adjudication are separate later steps and were not authorized. |
| QA11-08 | Retrieval/search/measurement | An approved, frozen corpus and approved protocol produce observable retrieval results for acceptance. | BLOCKED | User authorized drafting only. No retrieval/search, model inference, measurement, Docker, bridge, or database operation was performed. Protocol is non-ready and corpus is not frozen. |
| QA11-09 | Fresh artifact checks | Candidate JSON/schema and documentation artifacts parse and pass the recorded hygiene checks. | PASS | Fresh evidence: protocol JSON Schema passed with only CLI deprecation warning; JSON parsing passed; Markdownlint reported 0 issues; `git diff --check` passed. These checks do not imply product acceptance. |
| QA11-10 | Integration quality gate | The bridge passes Clippy with `-D warnings`. | BLOCKED | Prior Clippy failure remains: 93 library / 120 all-target test errors. It was not rerun for this QA. |

## Untested Scope

- Scope: Product acceptance, corpus sufficiency/representativeness, content suitability, source sensitivity, owner approval/freeze, labels/adjudication, confirmed protocol-run approval, encoder identity and operational provenance, retrieval/search behavior, and measurements.
- Reason: User authorized drafting only. The protocol is non-ready; corpus review/freeze and approval inputs are pending. No measurement or labels are authorized. Docker, bridge, database, inference, and retrieval/search were not run.
- Re-run prerequisite: Complete owner review of all corpus text and source mappings, sensitivity review, and explicit freeze of an exact corpus version/hash; resolve protocol identity/provenance, repository and gate approvals and all applicable scopes; obtain explicit authorization for any requested execution. Resolve the existing Clippy blocker for integration as applicable.

## Findings

| ID | Severity | Scenario / location | Evidence | Status |
|---|---|---|---|---|
| QA11-F01 | P1 | QA11-03/05/08 — protocol readiness and product acceptance | Protocol validator reports `confirmatory_ready=false`; corpus is one-query draft, not frozen, and not established as sufficient or representative. No execution is authorized. | Open — blocks acceptance and execution; not an artifact-validation defect. |
| QA11-F02 | P1 | QA11-06/07 — source sensitivity review and labels | Source/content and sensitivity reviews are pending; no labels or adjudication exist. | Open — blocks freeze and any future absolute-relevance acceptance. |
| QA11-F03 | P1 | QA11-10 — integration quality gate | Existing Clippy failure remains (93 library / 120 all-target test errors); not rerun. | Open — integration risk. |

## Verdict

`BLOCKED`

### Rationale

The supplied fresh evidence supports the review-only artifact constraints: the candidate protocol has the accepted ANN–exact decisions and keeps absolute relevance disabled; the corpus and canonical hash validate; all 11 provenance mappings are unique, complete, allowed, and match baseline source hashes; schema, parsing, Markdownlint, and diff checks pass. These results validate structure and declared source consistency only. They do not establish product acceptance.

Acceptance and execution are blocked. The corpus contains one query and ten unlabelled candidates, remains draft/not frozen, and is neither representative nor confirmed sufficient. Source/content and sensitivity review, corpus freeze, labels, applicable corpus and ANN scopes, identity/provenance, repository approval, and gate approval remain pending. `confirmatory_ready=false`. No retrieval/search, inference, measurement, Docker, bridge, or database work was performed. The prior Clippy failure remains. Do not archive.

### Limitations and Handoff

- QA does not fix code or artifacts other than this report and the phase state handoff.
- Product acceptance is not claimed. Protocol/corpus validators and provenance hashes are not product acceptance evidence.
- No archive is recommended. Keep acceptance and execution blocked until the review, freeze, approval, and authorization prerequisites above are satisfied; rerun QA against the approved target then.
- `fallback` mode applies because the SDD quality runner is not configured; deterministic enforcement is not claimed.
