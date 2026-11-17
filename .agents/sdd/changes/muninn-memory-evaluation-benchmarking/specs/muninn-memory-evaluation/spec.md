# Muninn Memory Evaluation Specification

## Purpose

Define the evidence and reporting contract for evaluating Muninn memory behavior. The specification keeps deterministic conformance, probabilistic component quality, end-to-end agent memory utility, and operational efficiency distinct. It does not establish an approved benchmark protocol or acceptance policy.

For this change only, the user has authorized design and execution of original roadmap milestone 5: a minimal read-only MCP memory-retrieval connector for Huginn, a deterministic synthetic Huginn task comparison, and one bounded local end-to-end run after all stated gates pass. This scoped authorization does not approve a general-purpose benchmark framework or any excluded resource, data source, policy, or operation below.

## Scope and Evidence Boundaries

This specification defines three evaluation layers:

1. **Deterministic and contract conformance** evaluates observable contract behavior with deterministic checks and records traceability to the TCK and Feature Parity contract. Conformance evidence is not a quality score.
2. **Probabilistic component quality** evaluates applicable memory components, such as extraction, entity resolution, retrieval, evidence retrieval, temporal reasoning or updates, and abstention. Results characterize the selected component and protocol, not general product usefulness.
3. **End-to-end agent memory utility** evaluates whether an agent completes observable tasks correctly while using memory and appropriate supporting evidence. It is not inferred from component scores alone.

Operational efficiency (for example, resource use, latency, or cost when approved for measurement) MUST be reported as a separate dimension. It MUST NOT be blended into an overall quality score.

Current evidence is bounded. `docs/muninn/reference-tck.md` records 189 TCK cases run against an adapted oracle, not Muninn. Adapter mediation changes or supplies behavior; those results do not establish native Muninn conformance or general memory quality. `docs/muninn/h4-measurement.md` records extraction quality as **UNVERIFIABLE**; its differential and smoke evidence do not establish semantic extraction quality. The H5 pilot is descriptive and not accepted; see `.agents/rpi/plan/tasks/muninn-h5-retrieval-context.md` and the status recorded in `docs/muninn/h4-measurement.md`. None of these records may be represented as an accepted general benchmark.

This authorization permits only a deterministic synthetic task comparison for the local milestone. It does not authorize broad benchmark measurement or inference outside that bounded run. The single E2E run is permitted only after all prerequisites in “Bounded milestone 5 execution gate” pass; otherwise it MUST NOT start.

The external Feature Parity draft at `/Users/acosta/Dev/dallay/cortex/tmp/plans/muninn-feature-parity-spec.md` is **outside this repository**, has status **Proposed**, and identifies `docs/muninn/specs/feature-parity.md` as its intended target. It is a linked coordination reference only, not an approved contract. This specification does not copy or approve its conformance requirements. Its §10.3, G5, and G6 are cross-reference/dependency points only. If the draft changes before approval, this evaluation specification MUST continue to identify its version/status and report any conflict for explicit coordination; it MUST NOT silently adopt new requirements. If Feature Parity is later approved or its target becomes authoritative, the owners MUST reconcile scope and precedence explicitly before changing either contract. The approved Feature Parity contract, if one exists, owns product feature/conformance definitions; this specification owns evaluation evidence, layer separation, and reporting rules. Neither silently overrides the other.

## Requirements

### Requirement: Keep evaluation layers and efficiency distinct

An evaluation report MUST identify whether each result belongs to deterministic/contract conformance, probabilistic component quality, end-to-end agent memory utility, or operational efficiency. It MUST NOT combine these layers into one blended score. A conformance pass MUST NOT be presented as proof of quality or utility.

#### Scenario: Report separate evidence layers

- GIVEN a run contains conformance checks, component evaluations, agent tasks, and operational observations
- WHEN the report is produced
- THEN every result is attributed to its applicable layer
- AND operational efficiency is reported separately from all quality layers
- AND no combined overall score obscures the distinct results

#### Scenario: Conformance traceability is not quality evidence

- GIVEN a deterministic contract check passes
- WHEN a report summarizes the result
- THEN it reports the tested contract and its traceability source
- AND it does not infer probabilistic component quality or end-to-end utility from that pass

### Requirement: Trace deterministic conformance without duplicating Feature Parity

The conformance layer MUST identify the deterministic contract checks performed and their traceability to applicable TCK cases and/or Feature Parity references. The evaluation specification MUST NOT duplicate Feature Parity's proposed or later approved product-conformance requirements. Where a relevant Feature Parity requirement is unavailable, proposed, changed, or conflicting, the report MUST record that status and leave the corresponding conformance claim unresolved rather than infer approval.

#### Scenario: Record TCK and Feature Parity references

- GIVEN a deterministic check is associated with a TCK case or a Feature Parity requirement
- WHEN the check is recorded
- THEN its result identifies the source reference, tested target, execution mode, and any mediation or adaptation relevant to interpreting the result
- AND an adapter-mediated oracle result is not labeled as a Muninn result

#### Scenario: Feature Parity remains proposed

- GIVEN the external draft remains Proposed and its intended target is `docs/muninn/specs/feature-parity.md`
- WHEN an evaluation plan or report cites it, including §10.3, G5, or G6
- THEN it labels the draft Proposed and external to this repository
- AND treats those sections and gates as coordination/dependency references only
- AND does not assert that their requirements are approved or satisfied

#### Scenario: Feature Parity changes

- GIVEN the external draft or an approved target changes a referenced requirement
- WHEN the change affects evaluation scope or a conformance claim
- THEN the discrepancy and affected references are made explicit
- AND no precedence change or new requirement is applied until owners explicitly reconcile the contracts

### Requirement: Define reproducible scenario and evidence records

Each evaluation scenario MUST have a stable identity and version. Its record MUST be sufficient to reconstruct the intended evaluation context without requiring a prescribed file format. As applicable, it MUST describe an ordered event sequence and evaluation checkpoint; event and query timestamps and relevant time semantics; tenant, owner, session, or other authorization scopes; the expected answer or structured outcome; relevant evidence identifiers; acceptable unknown/abstention outcomes; forbidden disclosures or actions; and mutation, update, or deletion events. It MUST identify authorization cases where scope or access control is relevant. Scenario identity/versioning MUST distinguish changed histories, expectations, evidence, or scope rules.

#### Scenario: Evaluate a temporal history at a checkpoint

- GIVEN a versioned scenario contains timestamped memory events, a later update, and an evaluation checkpoint
- WHEN the scenario is evaluated
- THEN the result is judged against the state and expected outcome at that checkpoint
- AND the scenario record preserves the event order, timestamps, applicable scope, and evidence identifiers

#### Scenario: Identify evidence and abstention expectations

- GIVEN a scenario expects a fact supported by particular memory items, or expects the system to return unknown
- WHEN an outcome is recorded
- THEN the expected answer or structured outcome and relevant evidence IDs are explicit
- AND an allowed abstention/unknown outcome is distinguishable from an incorrect answer or missing result

#### Scenario: Represent forbidden disclosures and actions

- GIVEN a scenario includes protected information or a prohibited action
- WHEN the scenario is evaluated
- THEN the protected disclosure or prohibited action is specified as a forbidden outcome
- AND its evaluation is not reduced to a relevance judgment or ordinary quality score

#### Scenario: Detect scenario version changes

- GIVEN a scenario's event sequence, scope, expected outcome, evidence IDs, or forbidden outcomes change
- WHEN that scenario is included in a later evaluation
- THEN the changed scenario has a distinguishable version and provenance
- AND results from the two versions are not silently treated as the same scenario

### Requirement: Prevent evaluation leakage and retain provenance

Evaluation data MUST be partitioned and managed to prevent leakage between development/tuning, calibration, and final evaluation where those partitions are used. The approved protocol MUST state the partitioning unit and rules before measurement. Closely related histories, shared source material, duplicated conversations, and derivations MUST be handled so that the same underlying information cannot cross partitions in a way that invalidates the intended comparison. Reports MUST preserve scenario, evidence, label, partition, and source provenance sufficient to explain each result.

#### Scenario: Keep related histories in one partition

- GIVEN multiple scenarios share source conversations, facts, or derived examples
- WHEN scenarios are assigned to evaluation partitions
- THEN the approved partitioning rules prevent related material from leaking across partitions
- AND the assignment and rationale are retained in provenance

#### Scenario: Trace an outcome to its source material

- GIVEN an outcome depends on labeled expected values or evidence identifiers
- WHEN the result is reported
- THEN its scenario and evidence provenance can be traced to the applicable versioned sources
- AND the report distinguishes missing provenance from a negative evaluation outcome

### Requirement: Evaluate component quality with an explicit method

Probabilistic component evaluation MUST name the component, target behavior, applicable scenarios, outcome representation, and evaluation method. The evaluated components MAY include extraction, entity resolution, retrieval, evidence retrieval, temporal reasoning or update handling, and abstention, but a component MUST be marked not applicable when the scenario does not test it. Results MUST not imply coverage of untested components or general memory quality.

#### Scenario: Evaluate a component on applicable cases

- GIVEN a component-quality evaluation includes applicable and non-applicable scenarios
- WHEN component results are summarized
- THEN the component and method are identified
- AND applicable outcomes are reported separately from cases marked not applicable
- AND the summary does not claim untested component coverage

#### Scenario: Separate evidence retrieval from answer correctness

- GIVEN a response is correct but its supporting memory evidence is absent or incorrect, or the response is incorrect despite retrieved evidence
- WHEN the outcome is assessed
- THEN answer/task correctness and evidence retrieval/use are recorded as distinct observations
- AND one does not substitute for the other

### Requirement: Evaluate end-to-end memory utility through observable outcomes

End-to-end evaluation MUST use observable task outcomes and defined acceptance tests for the task. It MUST assess whether relevant memory is used appropriately, including evidence use where applicable. An LLM-as-judge MUST NOT be the sole basis for an end-to-end utility claim. If a judge is proposed, its role, calibration, human reference, uncertainty, prompt/model/version provenance, and limitations MUST be documented, and use of that judge remains subject to explicit approval.

#### Scenario: Assess task outcome and evidence use

- GIVEN an agent task has an observable expected outcome and relevant memory evidence
- WHEN an end-to-end scenario is evaluated
- THEN the task outcome is assessed against its acceptance test
- AND evidence use or failure to use relevant evidence is recorded where applicable
- AND the result is not inferred solely from a component metric

#### Scenario: Run the authorized synthetic Huginn comparison

- GIVEN the exact synthetic scenario, observable assertions, baseline semantics, and narrow claim scope have been approved
- AND protocol interoperability, asynchronous dispatch, immutable scope, and required security tests have passed
- AND local model readiness is verified without credential inspection or model download
- AND the workspace, Huginn session, Muninn database, and output are isolated, unique, disposable, and run-specific
- AND Huginn MCP startup approval and per-call approvals are available
- WHEN the single bounded local end-to-end comparison is run
- THEN it compares Huginn task outcomes with read-only Muninn retrieval available and unavailable under the approved matched conditions
- AND it records deterministic assertions, evidence-use observations, unmatched cases, and deviations without claiming statistical acceptance or generality
- AND any failed prerequisite blocks the run before inference or service startup

#### Scenario: Judge is not the only outcome source

- GIVEN an evaluation proposes an LLM-as-judge
- WHEN the evaluation protocol is reviewed
- THEN the judge's role, calibration, human reference, uncertainty, prompt/model/version provenance, and limitations are specified
- AND the protocol records that judge use requires explicit approval
- AND an unapproved judge result cannot be presented as accepted task evidence

### Requirement: Report outcome status and denominators transparently

An evaluator and report MUST distinguish at least successful, failed, skipped, not applicable, and blocked outcomes. A skipped, not-applicable, or blocked case MUST NOT be counted as a pass, failure, or zero-valued metric unless an explicitly approved metric definition states otherwise. Per-case outcomes and the applicable denominator MUST be available for every aggregate. Deterministic checks MUST use explicit contract assertions; probabilistic or semantic assessments MUST identify their method and uncertainty limits. Reports MUST preserve the distinction between a missing/unevaluable result and an observed zero.

#### Scenario: Distinguish skip, non-applicability, block, and failure

- GIVEN one case fails its assertion, one is skipped, one does not apply, and one cannot run because a prerequisite is blocked
- WHEN results are reported
- THEN the four statuses remain distinct
- AND only the applicable outcomes enter a denominator as defined by the approved metric
- AND none of the non-evaluable statuses is silently converted to zero or pass

#### Scenario: Report per-case outcomes and denominator

- GIVEN a set of cases contributes to a summary metric
- WHEN the summary is reported
- THEN per-case outcomes and the metric's denominator are available
- AND exclusions and their reasons are reported separately

### Requirement: Preserve scenario-level dependence in comparisons and statistics

Paired comparisons MAY be used when the same scenario and compatible conditions are evaluated under each compared configuration. Reports MUST identify the pairing unit and missing or unmatched cases. Repeated executions of one scenario MUST remain associated with that scenario and MUST NOT be counted as independent scenarios. Any future statistical protocol MUST be explicitly approved before measurement and MUST specify metrics, sampling unit, aggregation, paired or unpaired design, uncertainty analysis, and decision rules. No statistical method, sample size, repetition count, confidence level, acceptance score, threshold, or CI gate is approved by this specification.

#### Scenario: Compare configurations on paired scenarios

- GIVEN two configurations are evaluated on the same versioned scenarios under compatible controlled conditions
- WHEN a paired comparison is reported
- THEN the report identifies the scenario-level pairing and any unmatched cases
- AND it does not represent repeated executions of a scenario as additional independent scenarios

#### Scenario: Statistical choices remain unresolved before approval

- GIVEN measurement is proposed but no statistical protocol has been approved
- WHEN the evaluation is prepared
- THEN metrics, sampling unit, aggregation, comparison design, uncertainty analysis, and decision rules are explicitly marked as decisions needed
- AND measurement does not proceed on the assumption that illustrative values are approved

### Requirement: Compare baselines fairly and identify comparison mode

For a controlled comparison, configurations MUST use the same tasks/scenarios, model and provider, tool availability, and context budget, except for the component or configuration dimension intentionally varied. Any unavoidable difference MUST be disclosed and considered in interpretation. A best-effort comparison MAY be reported separately but MUST be labeled as such and MUST NOT be presented as a controlled comparison. Baseline selection remains an explicit decision.

#### Scenario: Controlled configuration comparison

- GIVEN two configurations are intended for a controlled comparison
- WHEN they are evaluated
- THEN the same tasks, model/provider, tools, and context budget are held constant except for the declared variable
- AND any deviation is disclosed with its effect on comparability

#### Scenario: Best-effort comparison

- GIVEN configurations cannot be made equivalent on one or more controlled conditions
- WHEN results are compared
- THEN the comparison is labeled best-effort
- AND it is not described as controlled or causal evidence

For the authorized synthetic comparison, the selected baseline MUST state precisely whether Muninn retrieval is unavailable and MUST control or disclose Huginn session history. The comparison MUST hold the same task, provider/model, tools, prompt, context budget, workspace fixture, and permissions constant where feasible; any deviation MUST be disclosed.

### Requirement: Provide a minimal read-only Huginn MCP retrieval connector

The milestone 5 connector MUST expose retrieval of messages and preferences only. The retrieval capability MUST be composed through an MCP command in `apps/muninn`. Entity retrieval and reasoning-trace retrieval MUST be excluded because the current retrieval APIs do not enforce owner scope for those memory types. Both the message-retrieval path and the preference-retrieval path MUST apply the process's fixed owner scope. The connector MUST NOT expose memory write, update, or delete operations, and MUST NOT fall back to direct bridge/API integration. The Muninn MCP handler, dispatch path, and stdio request loop MUST be asynchronous end-to-end, MUST process requests sequentially, and MUST NOT use `block_on` or a nested runtime. Request and operation bounds and timeout behavior MUST be explicit and fail closed. Protocol version behavior MUST be preserved. Huginn rmcp and Muninn MCP revisions are both source-confirmed as `2026-07-28`; this source match does not establish runtime interoperability, which MUST be verified by an end-to-end protocol test.

#### Scenario: Handle an approved retrieval request asynchronously

- GIVEN the MCP server has started with a valid host-configured scope and Huginn has approved the invocation
- WHEN Huginn invokes the retrieval tool
- THEN the request is dispatched and awaited asynchronously through the handler and stdio loop
- AND the returned capability is limited to bounded read-only retrieval
- AND request processing remains sequential and the existing protocol-version behavior is preserved

#### Scenario: Retrieve messages within the fixed owner scope

- GIVEN the MCP command in `apps/muninn` is composed with a valid fixed owner scope
- WHEN an approved message-retrieval request is handled
- THEN retrieval is limited to messages in that fixed owner scope
- AND the request cannot provide or change the owner scope

#### Scenario: Retrieve preferences within the fixed owner scope

- GIVEN the MCP command in `apps/muninn` is composed with a valid fixed owner scope
- WHEN an approved preference-retrieval request is handled
- THEN retrieval is limited to preferences in that fixed owner scope
- AND the request cannot provide or change the owner scope

#### Scenario: Exclude memory types without owner-enforced retrieval

- GIVEN current retrieval APIs do not enforce owner scope for entities or reasoning traces
- WHEN the MCP tools are composed
- THEN no entity-retrieval or reasoning-trace-retrieval capability is exposed
- AND the connector remains limited to messages and preferences

#### Scenario: Reject missing or invalid host scope

- GIVEN the MCP server process has no explicit valid host-controlled owner scope
- WHEN the server starts
- THEN startup fails before the server accepts requests
- AND a model-supplied scope cannot select or change the owner scope

#### Scenario: Bound or time out retrieval

- GIVEN a retrieval request exceeds an operation/request bound or its timeout
- WHEN the handler processes the request
- THEN it stops or rejects the operation within the defined bound
- AND it returns a failure without unbounded work, nested runtime execution, or scope fallback

#### Scenario: Verify runtime protocol interoperability

- GIVEN source inspection confirms Huginn rmcp and Muninn MCP revisions `2026-07-28`
- WHEN the client and server perform the required runtime interoperability test
- THEN compatible initialization and retrieval behavior are demonstrated
- AND source revision matching alone is not reported as runtime verification

### Requirement: Bind one immutable owner scope per MCP server process

The MCP server MUST obtain exactly one immutable owner scope from explicit host-controlled configuration and MUST fail startup when that scope is absent or invalid. Both the message-retrieval path and the preference-retrieval path MUST enforce this same fixed scope. It MUST NOT accept a model-supplied scope selector. This scope is a data-scope selection for one local server process, not authentication and not a multi-tenant identity mechanism. Huginn startup and per-call HostPolicy approvals remain required; the connector MUST NOT grant or bypass approval. Memory and tool output MUST be treated as untrusted input, and authorization and other security controls MUST fail closed. Tests and the bounded run MUST NOT reset or reuse shared/persistent data.

#### Scenario: Preserve approval and untrusted-output controls

- GIVEN Huginn has not approved MCP startup or a particular invocation, or returned memory/tool output contains instruction-like content
- WHEN startup or the invocation is attempted, or the output is consumed
- THEN the applicable HostPolicy approval is required and cannot be bypassed
- AND the output is treated as untrusted input
- AND authorization failures and security-control failures fail closed

#### Scenario: Do not use shared data or reset state

- GIVEN the only available database, session store, workspace, or output is shared or persistent
- WHEN connector tests or the bounded E2E run are prepared
- THEN the affected test or run is blocked
- AND no reset or reuse of the shared data occurs

### Requirement: Preserve run reproducibility and safe evidence

Each measurement run MUST have a manifest and retained raw per-scenario results plus a human-readable report. The manifest MUST record, as applicable, source revisions; runtime, model and provider identities; configuration; dataset, scenario, evidence, and label versions or hashes; evaluator versions; execution environment; relevant seeds; and repetition details when repetitions have been approved. Reports and retained outputs MUST be secret-safe and MUST NOT expose credentials or protected payloads. Provenance MUST distinguish verified identities from caller-supplied or unavailable identities. These are content requirements, not a commitment to a final filename or schema.

#### Scenario: Reproduce a recorded run

- GIVEN a run has completed under an approved protocol
- WHEN its report is reviewed or the run is reproduced
- THEN the manifest identifies the relevant source, runtime, configuration, input, evaluator, environment, and approved repetition provenance
- AND raw per-scenario outcomes can be related to the human-readable report

#### Scenario: Protect secrets and identify incomplete provenance

- GIVEN a run artifact contains sensitive runtime context or lacks verified identity for a relevant component
- WHEN artifacts are retained or reported
- THEN secrets and protected payloads are excluded or redacted
- AND unverified or unavailable provenance is marked as such rather than asserted as verified

### Requirement: Enforce independent security and privacy gates

Authorization and scope isolation, deletion or revocation behavior, injection resistance for untrusted memory, and protected-data controls MUST be evaluated as independent hard gates where applicable. A single confirmed unauthorized disclosure or prohibited action MUST be reported as a gate failure and MUST NOT be masked by aggregate quality, utility, or efficiency results. The report MUST state the scope and limitations of the security cases; passing a finite suite MUST NOT be claimed to prove system security.

Before a dataset or external scenario source is adopted, its license, privacy implications, consent basis where applicable, retention constraints, contamination risk, suitability, and safe handling MUST be reviewed and recorded. Dataset selection and adoption remain open until that review is complete and authorized.

#### Scenario: Prevent cross-scope disclosure

- GIVEN a scenario contains memory belonging to a different tenant, owner, session, or protected scope
- WHEN an unauthorized query is evaluated
- THEN the result records whether any protected information was disclosed
- AND a confirmed disclosure is an independent hard-gate failure regardless of aggregate scores

#### Scenario: Verify deletion or revocation at later checkpoints

- GIVEN a scenario deletes or revokes memory before a later query checkpoint
- WHEN the later query or retrieval is evaluated
- THEN the scenario checks that deleted or revoked information is not used or disclosed as prohibited
- AND any confirmed prohibited use or disclosure fails the applicable independent gate

#### Scenario: Treat memory content as untrusted input

- GIVEN stored memory includes instruction-like or adversarial content
- WHEN an agent performs a task using that memory
- THEN the scenario checks for prohibited action, policy override, or protected-data disclosure
- AND a confirmed prohibited outcome is not averaged away

#### Scenario: Dataset adoption is not yet reviewed

- GIVEN a candidate external dataset lacks a documented license, privacy, consent, retention, contamination, suitability, or handling review
- WHEN dataset adoption is proposed
- THEN the dataset remains unapproved for evaluation use
- AND no benchmark result is represented as based on an approved dataset

### Requirement: Keep operational efficiency separate

Operational efficiency observations MUST identify their measure, scope, environment, and provenance and MUST be reported separately from conformance, component quality, and end-to-end utility. This specification does not select efficiency metrics, resource budgets, or acceptance limits.

#### Scenario: Report efficiency without blending quality

- GIVEN an evaluation records operational efficiency observations alongside task outcomes
- WHEN the report is produced
- THEN efficiency observations are presented as a separate dimension
- AND they do not alter or conceal quality or security outcomes

### Requirement: Record decisions needed and honor non-goals

The evaluation program MUST maintain an explicit decisions-needed state for all unapproved protocol choices. The sole approved exception to the prior specification-only boundary is the bounded milestone 5 connector, deterministic synthetic Huginn task comparison, and one local E2E run gated as specified here. This change does not approve a general benchmark runner/framework, broad adapter, external datasets or external-corpus labels, cloud/paid providers, credential inspection or use, model downloads, destructive resets, production access, a numerical/statistical acceptance policy, CI gates, or memory write/update/delete operations or direct bridge/API integration. No numeric thresholds, sample sizes, repeat counts, confidence level/method, model choice, external dataset, or CI gate is approved. Any user-provided examples—including scenario counts, per-category counts, repetition counts, example scores or intervals, and example configurations—remain illustrative and MUST NOT become normative without separate explicit approval.

The following decisions remain open: the exact synthetic scenario, assertions, baseline semantics, evidence-use rule, security cases, and claim scope; already available local model/provider readiness; isolated disposable local resources and unique evidence destination; non-numerical reporting details; and whether/how an LLM judge may be used. External data adoption, provider/model expansion, metrics and efficiency measures, labels, sample size and sampling unit, repetition design, aggregation/statistical comparison, uncertainty method, confidence reporting, acceptance rules, thresholds, and CI gates are not authorized by this change.

Explicit non-goals are implementing a general-purpose runner or harness; broad adapter coverage; acquiring external datasets or producing external-corpus labels; cloud/paid-provider use; credential inspection/use; model downloads; destructive resets; production access; numerical/statistical acceptance policy or CI gates; write/update/delete MCP operations; direct bridge/API integration; and changing the TCK, H4/H5 evidence, or Feature Parity draft. Only the specifically authorized minimal connector and one gated synthetic local E2E run are in scope. These limits do not prohibit ordinary deterministic protocol, async, scope, authorization, bounds, timeout, and security tests required to verify the connector.

### Requirement: Gate the bounded milestone 5 execution

The single authorized local E2E run MUST NOT begin until local model readiness is verified without credential inspection or model download; the exact synthetic scenario, assertions, baseline, and claim scope are approved; protocol interoperability, async behavior, and scope tests pass; a unique isolated disposable workspace, Huginn session, Muninn database, and output destination are verified; and required Huginn MCP startup and per-call HostPolicy approvals are obtained. This approval MUST NOT be interpreted as approval for cloud/cost-bearing resources, secrets, production access, or shared/persistent data. A failed gate MUST block service startup and inference. Efficiency observations, if separately authorized and collected, MUST remain separate; security controls remain independent hard gates.

#### Scenario: All gates pass for one local run

- GIVEN every execution prerequisite is verified and the required startup and per-call approvals are obtained
- WHEN the approved synthetic comparison is executed
- THEN exactly one bounded local E2E run is performed within the approved claim scope
- AND no cloud/paid provider, credential inspection/use, model download, production endpoint, destructive reset, or shared data is used

#### Scenario: A gate fails or requires an unapproved resource

- GIVEN any prerequisite is missing, invalid, or requires an unapproved cloud/cost resource, secret access, model download, production access, or shared/persistent data
- WHEN the run is considered
- THEN execution is blocked before startup and inference
- AND the missing prerequisite is reported without treating this milestone authorization as broader approval

#### Scenario: Illustrative values are not approved parameters

- GIVEN a planning input contains example counts, repetitions, scores, intervals, or configurations
- WHEN the specification or a future protocol records those examples
- THEN they are labeled illustrative and decisions-needed
- AND they are not used as thresholds, sample requirements, acceptance criteria, or CI gates without explicit approval

#### Scenario: Work remains within the milestone-specific authorization

- GIVEN this change includes the expressly authorized milestone 5 work
- WHEN implementation or execution is considered
- THEN only the minimal read-only connector and the single gated synthetic local E2E run are permitted
- AND general benchmark infrastructure, external data/labels, unapproved resources, quantitative policy, CI gates, and excluded operations remain out of scope

## Local Evidence References

- `docs/muninn/README.md` — current compatibility boundaries and distinction between TCK/oracle evidence and Muninn evidence.
- `docs/muninn/reference-tck.md` — 189 TCK cases run against an adapted oracle, not Muninn; adapter mediation and bounded interpretation.
- `docs/muninn/h4-measurement.md` — extraction quality is UNVERIFIABLE; differential results are limited to declared behavior and H4; does not accept H5 or full parity.
- `.agents/rpi/plan/tasks/muninn-h5-retrieval-context.md` — H5 descriptive pilot and current acceptance limitations; not accepted benchmark evidence.
- `docs/muninn/specs/parity-matrix.csv` — static traceability inventory, not completed parity evidence.
- External Proposed draft (outside repository): `/Users/acosta/Dev/dallay/cortex/tmp/plans/muninn-feature-parity-spec.md`; intended target `docs/muninn/specs/feature-parity.md`. See Scope and Evidence Boundaries for coordination and precedence.

## Decisions Needed

No fixed sample size, repetition count, confidence level or method, statistical method, acceptance score, threshold, model choice, external dataset, or CI gate is approved. The one authorized synthetic local run is descriptive and bounded; it does not establish a quantitative acceptance policy or generality. Any broader measurement requires separate authorization and approval of its metrics, sampling unit, aggregation, comparison design, uncertainty analysis, and decision rules. Scenario-level provenance MUST be preserved, and repeated runs MUST NOT be treated as independent scenarios. All decisions listed under “Record decisions needed and honor non-goals” remain open.

## Non-Goals

- Implementing a general-purpose runner or evaluation harness, or broad adapter coverage.
- Acquiring external datasets or creating external-corpus labels.
- Cloud/paid-provider use, credential inspection/use, model downloads, destructive resets, or production access.
- General inference, measurement, or benchmark execution beyond the single gated synthetic local milestone 5 run.
- Selecting acceptance thresholds, numerical targets, sample sizes, repetition counts, confidence level/method, model choices, or CI gates.
- Write/update/delete MCP operations or direct bridge/API integration.
- Modifying Muninn product behavior, the TCK, H4/H5 evidence, or the separate Feature Parity draft.
