# Delta for Providers UI

## MODIFIED Requirements

### Requirement: Providers Quota Placeholder

The system SHALL display a quota and usage view at `/providers/quota` using live data. It MUST present locally observed request/token/cost usage separately from upstream-reported quota observations, MUST show each observation's provenance, original `observed_at`, and age, and MUST show unavailable states when observations or required values are absent, unsupported, stale, or malformed. A sample is stale at/after its documented reset timestamp (or `observed_at` plus documented reset duration); missing/malformed reset metadata without a documented duration means expiry unknown: retain an otherwise valid observation as age-stamped history but suppress its current warning. Warnings MUST be computed per connection and compatible metric/window/scope from provider-reported `used_fraction` compared to that connection's configured warning/error thresholds; they MUST NOT use 429 frequency or average incompatible limits. The view MUST render the existing local daily usage trend and any upstream observation history as visibly distinct series. Observed 429 frequency MUST remain a separately named local metric and MUST NOT be described as quota utilization. The system MUST NOT imply automatic background polling or a guaranteed refresh cadence. (Previously: the view showed mocked quota data and a follow-up banner.)

#### Scenario: Navigate to quota

- WHEN the user navigates to `/providers/quota`
- THEN the page fetches and displays available live local usage and upstream observations as distinct data
- AND it does not show mocked data or require a development/follow-up banner
- AND local daily usage trend is visibly rendered separately from upstream observation history

#### Scenario: Show per-connection and provider summaries

- GIVEN multiple connections have usage or upstream observations
- WHEN the page displays provider and connection information
- THEN connection-level data remains distinguishable
- AND a provider summary identifies its coverage and does not combine incompatible limits or windows

#### Scenario: Report unavailable or malformed upstream data

- GIVEN an upstream metric is unsupported, absent, malformed, or has no observation
- WHEN the page renders that metric
- THEN it explicitly shows unavailable/unsupported rather than zero or an inferred estimate
- AND any existing local usage history remains visible
- AND an observation without a valid reset or documented reset duration shows age/expiry unknown and cannot trigger a current warning

#### Scenario: Show warning and rate-limit metrics

- GIVEN reported limit data supports a warning and local 429 history is available
- WHEN the page renders status
- THEN the warning identifies its reported-data basis
- AND its connection, metric, window, and scope are identified
- AND 429 frequency is shown separately and is not called quota utilization

#### Scenario: Show history and freshness

- GIVEN historical local usage and/or upstream observations exist
- WHEN the page renders trends
- THEN local and upstream trends are labeled and displayed separately
- AND local daily trend points come from recorded usage
- AND upstream points show original observation time and age without implying periodic polling or interpolated samples

#### Scenario: No usage or quota data

- GIVEN no local history and no upstream observation exist
- WHEN the user opens the quota view
- THEN the page distinguishes empty local history from unavailable upstream reporting
- AND it does not render either as a numeric quota value
