# Provider Quota Specification

## Purpose

Separate locally observed usage from upstream-reported quota. Capability is source- and metric-specific; no vendor coverage is presumed.

## Requirements

### Requirement: Distinguish Local Usage from Upstream Observations

The system MUST represent local request/token/cost usage independently from upstream quota observations. Neither local usage nor observed HTTP 429 frequency is upstream quota utilization.

#### Scenario: Display both data classes

- GIVEN local usage and an upstream observation exist
- WHEN quota information is requested
- THEN each is returned separately, without inference between them

#### Scenario: Local usage without upstream observation

- GIVEN local usage exists but upstream data does not
- WHEN quota information is requested
- THEN local usage remains available and upstream quota is unavailable, not estimated

### Requirement: Preserve Provenance and Freshness

Each upstream observation MUST retain its metric, source, `observed_at`, and available scope/window/reset metadata. Current-warning eligibility MUST follow a documented reset timestamp, or a documented reset duration anchored to `observed_at`; an observation is stale at/after its reset. The system MUST preserve and expose observation age and MUST NOT treat the latest stored sample as fresh by virtue of being latest. If reset metadata is absent or malformed and no documented reset duration is available, the observation MAY remain in historical display with its age and expiry marked unknown, but MUST NOT qualify for a current warning.

#### Scenario: Observation provenance

- GIVEN a response supplies supported quota metadata
- WHEN recorded and retrieved
- THEN its value, source, metric, time, and available scope/window remain associated

#### Scenario: Missing or malformed quota values

- GIVEN the reported limit or remaining value is absent or uninterpretable
- WHEN the response is processed
- THEN no quota value is fabricated and that metric is unavailable

#### Scenario: Reset expiry determines current eligibility

- GIVEN an observation has a valid documented reset timestamp or reset duration
- WHEN quota information is requested
- THEN it is eligible for current warning classification only before its reset
- AND it is marked stale at or after reset while retaining its original `observed_at` and age

#### Scenario: Reset metadata cannot establish expiry

- GIVEN reset metadata is absent or malformed and no documented reset duration is available
- WHEN quota information is requested
- THEN any otherwise valid observation is historical with age and expiry unknown
- AND it is not eligible for current warning classification

### Requirement: Scope Observations and Aggregates Honestly

The system MUST retain connection identity for distinct observations. Provider aggregates MUST identify coverage and MUST NOT combine incompatible metrics/windows. Local usage MUST preserve known provider/connection identity.

#### Scenario: Different connection limits

- GIVEN a provider has connections with different limits
- WHEN data is presented
- THEN observations remain distinguishable and any summary identifies coverage without hiding differences

#### Scenario: Usage without connection attribution

- GIVEN usage has no connection attribution
- WHEN aggregated
- THEN it remains identifiable as unassociated, not assigned arbitrarily

### Requirement: Explicit Unavailability

The system MUST distinguish unavailable upstream data from reported zero or empty local history. Unsupported metrics MUST be unavailable; no capability is presumed.

#### Scenario: Provider does not report a metric

- GIVEN a metric is unsupported or unreported
- WHEN quota information is requested
- THEN it is marked unavailable/unsupported, without substituting local usage or configured thresholds

#### Scenario: No data yet

- GIVEN neither local rows nor upstream observations exist
- WHEN requested
- THEN empty local history and unavailable upstream reporting are distinct, not numeric zero quota

### Requirement: Warnings Use Reported Limits

Quota warnings MUST use compatible provider-reported limit data only. For each managed connection and observation, `used_fraction = (limit - remaining) / limit` MAY be compared to that connection's configured `quotaWindowThresholds.warning` and `.error` only when the reported limit and remaining values match in metric, window, and scope, and the observation is eligible under the freshness requirement. Error threshold means critical; warning threshold means warning; otherwise status is ok. Missing/malformed values, non-positive limit, remaining outside `[0, limit]`, unknown expiry, or absent connection thresholds make warning status unavailable. Results MUST remain per connection/metric/window and MUST NOT average thresholds or utilization across connections. 429 frequency MAY be shown separately, but MUST NOT be utilization or a substitute warning.

#### Scenario: Warning from reported headroom

- GIVEN a reported limit and comparable remaining value meet a warning condition
- WHEN status is presented
- THEN any warning identifies its metric, scope/window, and provider-reported basis
- AND it is evaluated against that connection's configured fraction thresholds, not its 429 frequency

#### Scenario: Incompatible or missing limits

- GIVEN limit and remaining values do not match by metric, window, or scope, or a valid current observation is unavailable
- WHEN warning status is presented
- THEN the status is unavailable rather than inferred or aggregated from another connection

#### Scenario: 429 frequency remains separate

- GIVEN local history includes 429s
- WHEN shown
- THEN frequency is separately named and never called quota utilization

### Requirement: Retain Separate Histories Without Polling Assumptions

The system MUST retain local history and keep upstream history separate, containing only actual provider observations. It MUST NOT imply background polling or guaranteed cadence.

#### Scenario: View historical trends

- GIVEN either history exists
- WHEN trends are requested
- THEN local daily usage history and upstream response observations are separately labeled series
- AND upstream points retain their own observation times and age without interpolation or implied polling

#### Scenario: No periodic observations

- GIVEN no new response supplied an observation
- WHEN history is viewed later
- THEN the last point retains its time, is not shown as refreshed, and local history remains available
