# Rook Product Requirements

**Status:** Draft — product direction, priority users, target metrics, and several operational guarantees require stakeholder validation.
**Scope:** Rook, the AI gateway within Cortex.
**Audience:** Product stakeholders, application developers, platform operators, and engineers.

This document separates documented current behavior from product proposals and open decisions. A documented capability is not automatically a product commitment or a guarantee that an optional subsystem is enabled in every deployment.

## 1. Product Overview

Rook is Cortex's AI provider proxy and router. It exposes client-facing model APIs, routes requests to configured provider connections, and provides management and operational interfaces. Rook and Huginn are separate products within Cortex. An accepted Huginn ADR requires product boundaries and explicit external contracts for integration; Huginn does not depend on Rook's domain or application crates ([ADR-0001](../huginn/adr/0001-agent-product-and-documentation-boundaries.md)).

The current API and runtime contracts are documented in the [API Reference](api.md), [Architecture](architecture.md), [Configuration Guide](configuration.md), [Providers](providers.md), and [Model Catalog](model-catalog.md).

## 2. Problem and Opportunity

Applications that integrate directly with multiple AI providers can become coupled to provider-specific APIs, credentials, and model identifiers. Provider failures can interrupt requests. Operators may also need a central place to manage connections and inspect request, usage, health, and routing information.

The Cortex overview presents provider outages, model switching, audit visibility, cost surprises, and provider-specific SDK complexity as problems Rook and Cortex address ([Cortex overview](../../README.md#why-cortex)). These statements establish the documented problem framing; the target customer, severity, frequency, and market priority have not been validated in the reviewed product documents.

**Open questions:** Which user problem is most important? Which users experience it, how often, and what do they use today?

## 3. Product Positioning

### Proposed positioning — not approved

> Rook is an AI gateway that gives applications a consistent interface to supported model providers, with policy-driven routing, controlled failover, and operational visibility.

This proposal does not establish self-hosting, a target market, or a distribution promise. Those require validation. The proposal also does not imply universal provider or protocol compatibility.

### Proposed value pillars — not approved

- **Interoperability:** reduce provider-specific integration work for supported API behavior.
- **Resilience:** apply explicit routing and fallback policies within documented constraints.
- **Operational control:** provide management and visibility for configured capabilities.

**Open question:** Is Rook primarily a Cortex component, a gateway for application developers, infrastructure for platform teams, or a product for enterprise deployments? The initial target should be selected before expanding the roadmap.

## 4. Target Users and Jobs to Be Done

The following profiles are working hypotheses based on documented interfaces, not validated personas.

| Working profile | Job to be done | Desired outcome |
|---|---|---|
| Application developer | Send model requests without embedding every supported provider's routing and request handling in application code. | A predictable client API and a clear compatibility contract. |
| Platform operator | Configure provider connections and routing, manage access, and investigate request outcomes. | Controlled changes and enough operational evidence to understand gateway behavior. |

Administrative access is a system role and is not treated here as a separate persona. Confirm whether these two profiles are the right priority users and whether their needs differ by deployment type.

## 5. Product Goals and Non-Goals

### Documented product direction

The repository documents these capabilities as part of Rook's scope:

- Serve client model requests through documented HTTP endpoints.
- Route requests to configured providers and support configured fallback combos.
- Provide management and operational information for capabilities that are enabled and available.

These are not ranked roadmap goals, approved business outcomes, or measured success targets.

### Proposed goals — require approval

- Make the client-facing compatibility boundary explicit for every supported API behavior.
- Make routing and fallback outcomes understandable and predictable for clients and operators.
- Preserve product boundaries so Cortex consumers can integrate through explicit contracts without depending on Rook internals.

### Non-goals

- Huginn's agent execution, tool use, task planning, and workflow orchestration. Huginn is a separate product; accepted product-boundary decisions are recorded in [ADR-0001](../huginn/adr/0001-agent-product-and-documentation-boundaries.md).
- Universal compatibility with every provider, model, protocol, deployment environment, or enterprise capability.
- A commitment to multi-tenancy, organization-level isolation, or a particular hosting model before those choices are approved.
- Replacing the API and architecture references with a duplicate technical specification.

## 6. Product Capabilities

| Capability | Documented current baseline | Product gap or question |
|---|---|---|
| Client APIs | The API reference lists `POST /v1/chat/completions`, `GET /v1/models`, and `POST /v1/messages`. Client API-key enforcement is configuration-dependent. | Define the compatibility matrix, supported fields, error contract, and any provider-dependent behavior. |
| Provider connections | Connections have provider kind, authentication type, credentials, optional base URL, active state, priority, and model information. Connection management is configuration-dependent. | Define the product's supported connection lifecycle and the security and availability expectations for management. |
| Routing and aliases | Routing strategies and model aliases are documented; combo aliases use the reserved `rook/` namespace. | State what a client-visible model identifier means and what selection guarantees apply. |
| Fallback combos | Combos try ordered steps; documented recoverable errors include 429, 5xx, and network failures. Non-429 4xx errors stop the combo. Per-step and overall timeout limits are documented. | Confirm that current behavior is the intended product contract; define product-level expectations for cancellation, retries, and exhausted routes. |
| Streaming | Combo fallback applies before streaming starts; no fallback occurs after the first chunk is sent. Provider stream support varies. | Publish a precise compatibility and failure matrix for streaming and supported provider capabilities. |
| Management and security | Management APIs use admin sessions; state-changing management calls require CSRF. Client API-key authentication is disabled by default and can be enabled. | Define required secure configurations for intended deployment and network exposure models. |
| Usage, cost, health, and telemetry | Endpoints and their availability constraints are documented. Usage endpoints return `503 USAGE_RECORDER_UNAVAILABLE` if the recorder is unavailable; telemetry returns `501` when disabled. | Define operator questions these surfaces must answer, retention expectations, and behavior when optional recording is unavailable. |
| Dashboard | A dashboard is embedded in the Rook binary and provides visual management capabilities. | Validate which operator workflows are essential and establish usability and accessibility criteria. |

## 7. Functional Requirements

The requirements below describe intended product outcomes. “Current baseline” records documented behavior and does not claim that every requirement is complete or enabled in every deployment. Gaps remain open until assessed against implementation and tests.

| ID | Product requirement | Motivation | Documented current baseline | Gap / status |
|---|---|---|---|---|
| PR-1 | A client must be able to submit model requests through a clearly defined Rook API contract. | Reduce provider-specific integration work for supported behavior. | The API reference documents OpenAI-compatible chat completions, model listing, and an Anthropic messages endpoint. | Compatibility breadth and exact guarantees require a published matrix. |
| PR-2 | A client-facing model identifier must resolve according to an explicit, documented Rook contract. | Keep client integration predictable as providers and connections change. | Model aliases, provider model listings, and virtual combo aliases are documented. | The product meaning and stability promise for each identifier require definition. |
| PR-3 | Rook must route a request only to destinations eligible under the applicable configured policy and authorization constraints. | Prevent unexpected provider selection and unauthorized upstream calls. | Architecture documentation describes connection eligibility, combo step resolution, and authorization checks. | Verify coverage across routing modes and describe user-visible outcomes. |
| PR-4 | Rook must apply configured fallback only under explicit, documented failure conditions. | Recover from eligible provider failures without hiding incompatible or permanent errors. | Combo behavior documents fallback on 429, 5xx, and network failures; other 4xx errors stop. | Confirm whether this behavior is the approved product contract; define cancellation and retry expectations. |
| PR-5 | Rook must preserve response integrity when a streaming response has begun. | A partially delivered response cannot safely be replaced by an unrelated provider response without a defined recovery contract. | Current documentation says no combo fallback occurs after the first chunk is sent. | Keep this limitation explicit unless a compatible recovery mechanism is designed and approved. |
| PR-6 | Rook must enforce management authorization and request-integrity protections for management operations. | Protect provider configuration, credentials, and administrative actions. | Management APIs use admin sessions; mutating requests require CSRF validation. | Define deployment-specific exposure requirements and test denied cases against the contract. |
| PR-7 | Rook must report whether operational data is available, unavailable, or disabled rather than representing missing data as a valid measurement. | Enable operators to distinguish system state from absent observations. | Usage and telemetry endpoint error behavior is documented. | Define consistent operator-facing expectations across health, audit, usage, and telemetry. |
| PR-8 | Rook and other Cortex products must integrate through explicit contracts and preserve their separate product responsibilities. | Allow products to evolve without direct domain or application coupling. | Accepted ADR-0001 establishes separate products and explicit external integration contracts; Huginn must not depend on Rook domain/application crates. | Define any required runtime integration and its contract separately; no new integration is implied here. |

## 8. Non-Functional Requirements

The requirements in this section are proposed product constraints unless explicitly identified as an accepted boundary or documented behavior. They are not measured service guarantees.

| Area | Proposed expectation | Current evidence / open work |
|---|---|---|
| Security | Protect credentials, enforce authorization before upstream access, and avoid exposing secrets in responses or logs. | Project-wide and Rook instructions define security invariants; verify each product contract and deployment mode. |
| Reliability | Make gateway failures, provider failures, and exhausted fallback routes distinguishable. Do not imply that gateway health guarantees provider availability. | `/health` reports gateway and provider information; define product-level availability semantics. |
| Performance | Measure Rook's processing overhead separately from upstream provider latency before setting targets. | No approved latency target or measurement method is established in the reviewed product documentation. |
| Observability | Provide enough request and routing evidence for an operator to explain a decision and outcome, subject to privacy and retention policy. | Usage, audit, health, and telemetry information are documented; minimum coverage, privacy, and retention remain open. |
| Resource management | Define limits and behavior for timeouts, concurrency, backpressure, and cancellation as product guarantees where needed. | Some timeout constraints exist for combos; broader resource guarantees and targets require assessment. |
| Compatibility | State which API features are supported, conditional on provider/model capability, or unsupported. | The model catalog distinguishes capability claims and provenance; a complete client compatibility matrix remains open. |

No availability, latency, throughput, retention, or recovery target is approved by this PRD.

## 9. Integration Boundaries

### Accepted boundary

Rook and Huginn are separate products within Cortex. Their integration must use explicit external contracts; Huginn must not depend on Rook's domain or application crates. This boundary is recorded in accepted [ADR-0001](../huginn/adr/0001-agent-product-and-documentation-boundaries.md).

### Proposed responsibility boundary — requires product confirmation

> Rook owns model access and routing decisions. Consumers own their business logic and agent orchestration.

This is a proposed product principle, not a new accepted architecture decision. Any runtime integration must be justified by a consumer need and specified through an explicit contract.

### Deployment and distribution

An accepted release ADR states the Rook GA distribution channels and Rust installation method ([ADR-0015](../huginn/adr/0015-separate-product-releases-from-cargo-publishing.md)). This does not, by itself, establish a self-hosting product promise or prove that Rook can be operated independently of every other Cortex component.

**Open questions:** Is independent operation a product goal? Is self-hosting the primary deployment model? Which deployment environments are in scope?

## 10. Acceptance Criteria

These criteria express proposed observable product outcomes. They do not claim that the current implementation passes them.

1. Given a supported client request and a valid configured destination, Rook returns a response that conforms to the published API compatibility contract.
2. Given a client-visible model identifier, an operator can determine its documented resolution behavior without relying on undocumented provider-selection rules.
3. Given a routing policy and request, only destinations eligible under that policy and the caller's authorization are considered for upstream execution.
4. Given a failure classified as recoverable by the published fallback contract, Rook evaluates the next permitted destination; for a non-recoverable failure, Rook returns the documented error without trying an unauthorized or prohibited destination.
5. Given that streaming has begun, Rook does not silently replace the partial response with a response from another provider unless a separately approved recovery contract preserves response integrity.
6. Given an unauthenticated or unauthorized management request, or a state-changing request without valid CSRF protection, Rook denies the operation according to the management API contract.
7. Given unavailable usage recording or disabled telemetry, the operator can distinguish unavailable data from valid zero-valued measurements using the documented endpoint response.
8. Given a processed request, an authorized operator can inspect the routing outcome and available request evidence without access to provider credentials or unapproved sensitive content.

The exact wire formats, status codes, limits, and provider-specific details belong in the relevant API, configuration, provider, model-catalog, and architecture references.

## 11. Success Metrics

No approved product success metrics, targets, owners, baselines, or review periods were found in the reviewed product documents. The following are candidates for stakeholder review, not commitments.

| Candidate metric | Proposed definition | Definition needed before adoption |
|---|---|---|
| End-to-end request success rate | Successful eligible requests divided by eligible requests. | Define eligible requests and separate caller errors, gateway failures, and upstream failures. |
| Fallback recovery rate | Eligible failed-primary requests recovered by fallback divided by eligible failed-primary requests. | Exclude cases where fallback is prohibited; define attempt and recovery semantics. |
| Gateway overhead p95 | 95th percentile of latency attributable to Rook processing, excluding upstream generation time. | Define instrumentation boundaries and sampling. |
| Routing traceability coverage | Requests with sufficient evidence to explain routing and outcome divided by processed requests. | Define required evidence, privacy constraints, and retention. |
| First successful integration time | Time for a target user to complete a first successful model request. | Define user cohort, measurement method, and success conditions. |
| Usage attribution coverage | Applicable requests with attributable provider/model/usage data divided by applicable requests. | Distinguish provider-reported values, estimates, and unknown values. |

Establish a measurement baseline and owners before adopting numeric targets.

## 12. Current Baseline and Gaps

This is a documentation-backed baseline, not a complete code-to-test audit. The “documented” state means a current project reference describes the behavior; it does not independently prove the behavior passes tests or is enabled in a deployment.

### Original baseline requirements

The six requirements from the initial draft are retained here as documented baseline statements. They describe existing documented scope, not approved future commitments.

| ID | Baseline requirement | Documented current behavior | Source |
|---|---|---|---|
| RQ-1 | Rook exposes a client-facing model API. | The API reference documents chat completions and model listing. Client API-key enforcement depends on configuration. | [API Reference](api.md) |
| RQ-2 | Rook routes requests to configured provider connections. | Provider connections are managed dynamically; active, configured connections are eligible for routing. | [Providers](providers.md), [Architecture](architecture.md) |
| RQ-3 | Rook supports configured fallback behavior. | Combos use ordered steps and documented execution constraints. | [Architecture](architecture.md) |
| RQ-4 | Rook protects management operations. | Management APIs use an admin session; state-changing requests require CSRF validation. | [API Reference](api.md) |
| RQ-5 | Rook provides operational request and health information. | Health, usage, cost, and telemetry information are documented; availability depends on recorder and telemetry configuration. | [API Reference](api.md) |
| RQ-6 | Rook allows operators to configure supported runtime behavior. | Configuration includes server, routing, cache, database, authentication, provider management, and rate-limit settings. | [Configuration Guide](configuration.md) |

| Area | State in reviewed documentation | Evidence | Gap to close |
|---|---|---|---|
| Client API surface | Documented | [API Reference](api.md) lists client endpoints and authentication behavior. | Define and publish the complete compatibility matrix. |
| Provider configuration | Documented; connection-management capabilities are configuration-dependent | [Providers](providers.md), [Configuration Guide](configuration.md), and [API Reference](api.md). | Verify expected lifecycle and operational guarantees. |
| Routing and fallback | Documented | [Architecture](architecture.md) and [Configuration Guide](configuration.md) describe routing and combo behavior. | Assess implementation and tests against PR-3 to PR-5; clarify product guarantees. |
| Authentication and CSRF | Documented | [API Reference](api.md) and [Configuration Guide](configuration.md). Client API-key authentication is disabled by default. | Define secure deployment requirements and verify enforcement evidence. |
| Model capability claims | Documented | [Model Catalog](model-catalog.md) defines capability declaration and provenance rules. | Complete client-facing compatibility documentation and validate provider-specific claims. |
| Usage and telemetry | Documented; optional subsystems can be unavailable or disabled | [API Reference](api.md) documents unavailable responses. | Define operational expectations, privacy, retention, and measurement coverage. |
| Cortex product boundary | Accepted architecture/product boundary | [ADR-0001](../huginn/adr/0001-agent-product-and-documentation-boundaries.md). | Specify any required product integration through separate contracts. |
| Distribution | Accepted release decision | [ADR-0015](../huginn/adr/0015-separate-product-releases-from-cargo-publishing.md). | Validate deployment and independent-operation claims separately. |

A status of `Implemented`, `Partially Implemented`, `Documented Only`, `Missing`, or `Not Applicable` requires a focused assessment against code and tests. This PRD does not assign those implementation statuses without that evidence.

## 13. Dependencies and Risks

### Dependencies

- Reachable upstream providers, supported provider APIs, and valid credentials.
- Rook configuration and required persistence for enabled capabilities.
- Client adherence to the published API and authentication contract.
- Operators with authorization to configure connections and management access.
- Measurements and telemetry sufficient to evaluate any approved success metrics.

### Risks

- **Unvalidated positioning:** the primary user, differentiation, and deployment model are not approved.
- **Ambiguous compatibility:** “OpenAI-compatible” can create expectations beyond the documented endpoint and provider/model capability set.
- **Fallback integrity:** retries and fallback can affect latency, cost, and response semantics; streaming has an explicit point after which fallback does not occur.
- **Optional feature expectations:** configuration and recorder availability affect management and operational surfaces.
- **Security exposure:** client API-key authentication is disabled by default; intended network exposure and required deployment safeguards need an explicit product decision.
- **Unmeasured quality:** no product-level performance or availability targets are approved.
- **Boundary drift:** adding agent workflow or business orchestration to Rook could blur the accepted separation between Cortex products.

## 14. Decision Log

Recommendations below remain proposals until stakeholders explicitly accept them. Accepted decisions are linked to their existing ADRs and are not re-decided here.

| ID | Decision | Recommendation or evidence | Status |
|---|---|---|---|
| DEC-01 | Priority users | Prioritize application developers and platform operators as working hypotheses. | Open — validate users and needs. |
| DEC-02 | Product positioning | Position Rook as an AI gateway focused on interoperability, resilience, and operational control. | Proposed — not approved. |
| DEC-03 | Deployment model | Evaluate self-hosting and independent operation as product goals. | Open — not established by this PRD or release ADR. |
| DEC-04 | Consumer and isolation model | Define whether access and isolation are scoped to an installation, API key, or organization. | Open — do not assume multi-tenancy. |
| DEC-05 | API compatibility | Publish a matrix of supported, provider-dependent, and unsupported behavior. | Proposed — validate scope against implementation and tests. |
| DEC-06 | Routing and fallback | Confirm recoverable failures, retry semantics, cancellation, and exhausted-route behavior as product contracts. | Open — technical behavior is documented; product approval remains necessary. |
| DEC-07 | Streaming recovery | Preserve the documented rule that fallback does not occur after streaming begins unless a compatible recovery contract is approved. | Documented constraint; future change requires approval. |
| DEC-08 | Security by deployment | Define minimum authentication and access requirements for each supported exposure model. | Open — current API-key authentication is disabled by default. |
| DEC-09 | Operational traceability | Define minimum evidence for explaining a routing decision and request outcome, with privacy and retention limits. | Proposed — not approved. |
| DEC-10 | Success metrics | Select metrics, owners, baselines, and review periods before setting targets. | Open. |
| DEC-11 | Cortex integration | Preserve accepted product separation and define any consumer integration through explicit contracts. | Accepted boundary: [ADR-0001](../huginn/adr/0001-agent-product-and-documentation-boundaries.md). |
| DEC-12 | Roadmap and gap prioritization | Assess baseline behavior against approved requirements before creating implementation initiatives. | Proposed next step; no roadmap commitment. |

## Approval Readiness

This PRD is a product-definition draft, not an approved roadmap or a complete implementation audit. Before approval, stakeholders should confirm the priority users, product positioning, deployment and isolation assumptions, API compatibility scope, security expectations, operational guarantees, and success metrics. After those decisions, assess the documented baseline against implementation and tests before creating new delivery initiatives.
