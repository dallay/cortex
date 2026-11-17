# Proposal: Muninn Memory Evaluation and Benchmarking

## Intent

Define a durable, evidence-oriented evaluation specification for Muninn and execute the explicitly authorized roadmap milestone 5: a bounded Huginn end-to-end memory-utility comparison. Existing evidence shows why evaluation layers must remain separate: the recorded TCK run was adapter-mediated against the oracle, not Muninn, and the H5 retrieval pilot is descriptive and remains `measured_not_accepted`. Neither establishes broad product quality or an accepted benchmark.

## Scope

### In Scope

- Specify three independent evaluation layers: conformance (including TCK and Feature Parity traceability), probabilistic component quality, and end-to-end memory utility. Report operational efficiency separately rather than folding it into quality scores.
- Define evaluation evidence and reporting concepts, including traceability, provenance, baseline fairness, uncertainty/statistical reporting, and limitations of LLM-as-judge methods. Keep unapproved protocol and numerical choices explicitly unresolved; examples remain illustrative only.
- Specify non-averaged security gates, including authorization and scope isolation, deletion/revocation behavior, untrusted-memory handling, and protection against cross-scope disclosure.
- Link to the proposed Feature Parity Specification (§10.3 and G5/G6) and existing TCK/H5 evidence without adopting the proposed document as an approved contract or duplicating its conformance requirements.
- Design, implement, and execute roadmap milestone 5 only: a minimal, read-only MCP memory-retrieval connector used by Huginn; a deterministic synthetic Huginn task comparison with memory available versus a selected fair baseline; and one bounded local end-to-end run after all readiness and preflight gates pass.
- Preserve Huginn's MCP server-start and per-invocation approvals, host-controlled immutable owner scope, authorization and scope checks, sequential async request handling, fail-closed behavior, and treatment of memory/tool output as untrusted input. Do not add write, update, or delete MCP operations.
- Use TDD for executable behavior: add focused failing tests before connector or evaluation behavior changes, then implement and refactor while tests remain green.

### Out of Scope

- A general benchmark runner/framework, broad adapter coverage, or a general-purpose evaluation harness.
- External datasets, external-corpus label creation/adjudication workflows, dataset downloads, or external-service queries.
- Cloud or paid providers, credential inspection/use, model downloads, or other cost-bearing resource use.
- Destructive resets or reuse of persistent/shared local data, production access, or changes to the separate Huginn umbrella and H5 acceptance changes.
- A numerical/statistical acceptance policy, approved thresholds, sample/repetition requirements, CI gates, or claims of broad/general benchmark validity.
- Write/update/delete MCP tools, direct bridge/API integration, or an implicit fallback to bridge/API access.
- Changing Muninn's TCK, H4/H5 evidence, or the external Feature Parity draft.

## Rationale

Conformance answers whether observable contracts are met; it is not a semantic-quality or memory-utility score. Probabilistic component evaluation and end-to-end memory utility answer different questions and need separate evidence and interpretation. Operational efficiency is also a distinct report dimension. Existing artifacts demonstrate these boundaries: `docs/muninn/reference-tck.md` documents 189 collected cases run against an adapted oracle, with adapter mediation and a failing overall result; `docs/muninn/h4-measurement.md` records extraction quality as `UNVERIFIABLE`; and `.agents/rpi/plan/tasks/muninn-h5-retrieval-context.md` marks H5 results descriptive and not accepted.

The user superseded the earlier specification-only boundary for milestone 5 and selected option A: a minimal read-only MCP retrieval connector. This is a narrow authorization for that connector and a synthetic local Huginn comparison, not authorization for a general evaluation platform or external/cost-bearing resources. Existing inspection found generic Huginn MCP client support but no Muninn application handlers or established Huginn-to-Muninn route. Protocol compatibility, safe async dispatch, scope enforcement, local model availability, and disposable local state therefore remain explicit readiness gates, not assumptions.

## Approach

Retain the evaluation specification's layer separation, reporting and evidence boundaries, and open decisions. For milestone 5, implement only the selected read-only MCP retrieval path and compose it with Huginn's existing MCP client and approval flow. Bind one immutable owner scope from host-controlled server configuration; fail closed if it is missing or invalid, and do not accept a model-supplied scope selector. Keep async handling end-to-end and sequential; verify protocol compatibility, bounded retrieval, authorization, and failure behavior with focused tests.

Use a small, versioned synthetic scenario and deterministic observable task assertions. Compare matched Huginn task configurations with Muninn retrieval available and unavailable, holding provider/model, tools, prompt, context budget, workspace fixture, and permissions constant where feasible. Name baseline semantics precisely, control or disclose Huginn session history, record unmatched cases and deviations, and do not use an LLM judge as the sole outcome source. Keep correctness, evidence use, security gates, and operational observations distinguishable; do not infer statistical acceptance or generality from this bounded run.

Execution is separately gated. Before inference or service startup, verify local model/provider readiness without inspecting credentials or downloading artifacts; approve the exact scenario, assertions, baseline, and comparison scope; pass protocol/async/scope-safety checks; and prove that Muninn database/volume, Huginn session store, workspace, and output destination are unique, run-specific, disposable local state. Select a unique evidence destination and ensure retained outputs exclude credentials and protected payloads. Start the Huginn MCP server only after its required startup approval, and obtain approval for each invocation. If any gate fails, stop without inference or destructive action and report the blocker. Do not reset or reuse existing state.

Reference `/Users/acosta/Dev/dallay/cortex/tmp/plans/muninn-feature-parity-spec.md` only as a linked, **Proposed** document whose intended target is `docs/muninn/specs/feature-parity.md`. Its §10.3, G5, and G6 inform dependency and boundary handling, but its draft requirements are neither approved by this change nor to be duplicated.

## Affected Areas

| Area | Impact | Description |
|------|--------|-------------|
| `.agents/sdd/changes/muninn-memory-evaluation-benchmarking/` | Modified | Proposal/specification/design/tasks define the evaluation boundaries and authorized milestone; this proposal does not update the other artifacts. |
| Huginn runtime MCP integration and Muninn MCP server | Modified | Implement only the narrow read-only retrieval connector and focused tests, subject to protocol, async, scope, and security checks. |
| `.agents/rpi/plan/tasks/muninn-memory-evaluation-benchmarking/evidence/<unique-run-id>/` | New, gated | Store secret-safe run manifest, per-case outcomes, and report only after execution gates pass. |
| `docs/muninn/reference-tck.md`, `docs/muninn/h4-measurement.md`, `.agents/rpi/plan/tasks/muninn-h5-retrieval-context.md` | Reference only | Preserve existing evidence and its stated limitations; no modification proposed. |
| External Feature Parity draft | Linked reference only | Proposed, not approved; do not duplicate or promote its conformance requirements. |

## Dependencies

- The local TCK, H4, and H5 records provide bounded evidence and must retain their stated limitations.
- The external Feature Parity Specification is a proposed reference; coordinate §10.3/G5/G6 without assuming approval or duplicating its contract.
- A compatible, already available local model/provider, an approved deterministic synthetic scenario/assertion and baseline choice, verified MCP protocol/async/scope behavior, isolated disposable local database/session/workspace/output state, and required Huginn startup and per-invocation approvals are prerequisites to execution.
- Failure of a prerequisite blocks the local run. It does not authorize model downloads, credential inspection, external services, production access, or destructive resets.

## Open Decisions

- Which exact synthetic task scenario, assertions, security cases, and evidence-use rule will be used, and what narrow claim will the case support?
- Which precise baseline mode will be used, and how will Huginn session history and other context be controlled or disclosed?
- Which already available local provider/model is selected, and can it run without downloads, credentials inspection, external network access, or cost?
- Which verified isolated disposable local backend/session/workspace setup and unique evidence destination are available?
- Which non-numerical comparison/reporting details are needed for this run? No statistical method, sample size, repetition count, confidence level, acceptance score, threshold, or CI gate is approved.
- Which LLM-as-judge use cases, calibration evidence, human review, and uncertainty disclosures are acceptable, if any? A judge is not the sole outcome source for this milestone.

## Risks

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| Oracle-adapted TCK evidence or H5 pilot results are presented as Muninn conformance or accepted benchmark evidence. | High | Preserve the recorded execution target, adapter mediation, and `measured_not_accepted` status in traceability and reports. |
| A draft Feature Parity requirement is treated as approved or duplicated in the evaluation contract. | Medium | Link the draft with its Proposed status and intended target; coordinate §10.3/G5/G6 and keep conformance ownership separate. |
| MCP protocol, async handling, or scope binding is assumed safe without end-to-end verification. | High | Gate connector and evaluation execution on focused compatibility, async, authorization, scope-isolation, and failure-path tests; fail closed on uncertainty. |
| Local model, baseline, or scenario differences make the comparison invalid or overstate utility. | Medium | Require explicit readiness and scenario/assertion/baseline choices; keep conditions matched, disclose deviations, and limit claims to the observed synthetic case. |
| Shared or persistent database, session, workspace, or output state is damaged or exposed. | High | Prove run-specific disposability and unique outputs before startup; prohibit resets/reuse; stop if isolation cannot be established. |
| MCP approvals or untrusted memory protections are bypassed, or retrieval discloses cross-scope data. | High | Preserve startup and per-call approval, host-bound immutable scope, read-only capability, untrusted-output handling, and independent non-averaged security gates. |
| Probabilistic metrics or LLM-as-judge outputs are over-interpreted as objective truth. | Medium | Require method, limitations, uncertainty, and provenance disclosures; prohibit judge-only task outcomes and defer acceptance rules until separately approved. |

## Rollback Plan

Revert the focused connector and evaluation implementation through normal version control if they fail review or verification. Preserve any already-created evidence as an audit record unless its authorized owner directs otherwise. Remove only explicitly identified run-specific disposable resources after evidence preservation and review; never reset, delete, or tear down shared/pre-existing state. Rejected documentation can be revised through the SDD workflow without changing TCK/H4/H5 evidence or the separate Feature Parity draft.

## Success Criteria

- [ ] The resulting specification keeps conformance, probabilistic component quality, and end-to-end memory utility distinct, and reports operational efficiency separately.
- [ ] Security gates are explicit, independent, and non-averaged; existing TCK/oracle and H5 evidence retains its verified limitations.
- [ ] The selected MCP connector exposes retrieval only, binds scope through host-controlled configuration, preserves Huginn startup and per-invocation approvals, and passes focused protocol, async, authorization, scope, and failure-path tests written using TDD.
- [ ] A deterministic synthetic Huginn task comparison records observable assertions, evidence use, baseline semantics, provenance, and unmatched/deviating cases without treating illustrative values as approved policy.
- [ ] A local end-to-end run occurs only after model readiness, scenario/assertion/baseline selection, protocol/async/scope-safety checks, disposable local state and unique output verification, and required Huginn approvals all pass; otherwise execution is blocked without inference or destructive action.
- [ ] No general runner/framework, broad adapter, external dataset or labels, cloud/paid provider, credential inspection, model download, destructive reset, production access, statistical acceptance policy, CI gate, write/update/delete MCP tool, or bridge/API integration is added.
