# Provider Usage and Quota Signals Specification

## Purpose

Defines the provider usage summary contract consumed by the dashboard, including the provenance and availability of measured Cortex-routed usage, observed upstream rate-limit signals, local thresholds, and provider account limits.

## Requirements

### Requirement: Provider Usage Summary Provenance

The provider usage summary endpoint SHALL return usage metrics only as measured for requests routed through Cortex. It MUST NOT label those metrics as total provider-account usage. Estimated cost MUST be distinguishable from provider-reported billing, and cost MUST be unavailable/unknown when pricing is not known rather than defaulting to zero.

#### Scenario: Summarize recorded Cortex traffic

- GIVEN the usage recorder contains entries for requests routed through Cortex for a configured provider
- WHEN a client requests the provider usage summary
- THEN the provider's request and token totals reflect those recorded entries for the requested periods
- AND the response identifies the coverage as Cortex-routed usage
- AND it does not claim the values cover requests made outside Cortex

#### Scenario: Cost is unknown without pricing

- GIVEN recorded usage has no available pricing information
- WHEN the provider usage summary is produced
- THEN cost is represented as unknown/unavailable
- AND it is not represented as zero or authoritative provider billing

#### Scenario: Provider has no usage records

- GIVEN no usage records match a provider and requested period
- WHEN the provider usage summary is produced
- THEN the response distinguishes missing/unavailable usage data from a measured zero where that distinction is not established
- AND it does not infer account quota or remaining capacity from the absence of records

### Requirement: Historical Provider Usage Trends

The provider usage summary SHALL expose historical trend data for its supported periods and measures, including request and token usage, and cost only where known. Trend values MUST derive from recorded Cortex-routed usage and MUST preserve unknown cost as unknown rather than treating it as zero.

#### Scenario: Return historical trend points

- GIVEN recorded usage exists across multiple days in the supported trend period
- WHEN a client requests the provider usage summary
- THEN the response includes time-bucketed trend points for the available historical data
- AND each point's usage values correspond to the records in its bucket

#### Scenario: Historical cost contains unknown values

- GIVEN some records in a trend period have unknown cost due to missing pricing
- WHEN the summary returns historical cost
- THEN unknown cost is not silently converted into a complete or authoritative total
- AND the response communicates cost availability/estimate semantics

### Requirement: Observed Upstream Rate-Limit Signals

The system MAY expose upstream rate-limit observations, such as upstream 429 outcomes or provider-supplied remaining/reset signals, only when those signals were actually captured from the upstream provider. Each signal MUST remain distinguishable from Cortex's own client API rate-limit headers and from locally configured thresholds. Missing signals MUST be represented as not reported/unavailable, not as healthy, unlimited, or no limit.

#### Scenario: Expose captured upstream signal

- GIVEN an upstream provider response was observed by Cortex and contained a rate-limit signal
- WHEN a client requests the provider usage summary
- THEN the response may include the captured signal with its available provider/source and time context
- AND the signal is identified as an observation, not an authoritative account balance

#### Scenario: Do not fabricate absent upstream signal

- GIVEN no upstream rate-limit signal was captured for a provider
- WHEN a client requests the provider usage summary
- THEN the response identifies the signal as not reported or unavailable
- AND it does not provide fabricated remaining/reset values or imply that no limit exists

#### Scenario: Cortex API limiter is not provider quota data

- GIVEN Cortex emits its own client-facing rate-limit headers
- WHEN provider usage information is returned
- THEN those Cortex limiter values are not represented as upstream provider signals or provider account quota

### Requirement: Local Threshold Semantics

Provider `quotaWindowThresholds` SHALL be represented as locally configured monitoring thresholds, separate from observed upstream signals and provider account limits. A warning based on a local threshold MUST identify that provenance and MUST NOT imply that the provider issued the threshold or that the threshold enforces request routing.

#### Scenario: Threshold warning carries local provenance

- GIVEN a provider has a configured local quota-window threshold and the relevant measured value reaches it
- WHEN the summary reports a threshold warning
- THEN the warning identifies the threshold as locally configured
- AND it is separate from any upstream rate-limit observation
- AND it does not claim to be a provider-issued limit or an enforced routing restriction

#### Scenario: No account limit inferred from measured values

- GIVEN usage totals, estimated cost, rate-limited request ratios, or local thresholds are available
- WHEN the provider usage summary is produced
- THEN the system does not derive or claim authoritative account maximum, consumption, or remaining quota from those values

### Requirement: Provider Account Quota Claims

The system MUST NOT report provider account maximum, consumed, or remaining quota as authoritative unless the value is obtained from a verified authoritative provider source or explicitly configured by the user with clear provenance. In this change, no provider account-level quota integration is in scope; unsupported account quota MUST be identified as not provided or not applicable, not fabricated.

#### Scenario: No authoritative account quota source

- GIVEN no verified provider account-quota source or explicit user-configured account limit is available
- WHEN a client requests the provider usage summary
- THEN the response does not claim an account maximum, consumed amount, or remaining amount
- AND it identifies account quota as not provided or unavailable

#### Scenario: Self-hosted Ollama account quota

- GIVEN the provider is self-hosted Ollama
- WHEN a client requests the provider usage summary
- THEN account quota is identified as not applicable or not provided
- AND measured Cortex-routed usage is still returned when available
- AND no zero, unlimited, remaining, or exhausted account quota is invented

### Requirement: Partial Availability and Degradation

The provider usage summary SHALL preserve available provider and field data when an individual provider's usage query or an optional data source is unavailable. An unavailable recorder or provider-management dependency MAY prevent the summary from being produced; otherwise a single provider/field failure MUST NOT erase all available summary data. Each unavailable portion MUST be distinguishable from a measured zero or an absent upstream signal.

#### Scenario: One provider summary query fails

- GIVEN summary data is available for multiple providers but a query fails for one provider
- WHEN a client requests the provider usage summary
- THEN available provider summaries are returned
- AND the affected provider is marked unavailable/degraded
- AND the request does not fail in a way that discards all otherwise available provider data

#### Scenario: Optional signal source unavailable

- GIVEN recorded usage is available but an optional upstream signal source is unavailable
- WHEN a client requests the provider usage summary
- THEN measured usage remains available
- AND the affected signal is marked unavailable/not reported
