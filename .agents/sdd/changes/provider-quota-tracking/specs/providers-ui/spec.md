# Delta for providers-ui

## MODIFIED Requirements

### Requirement: Providers Quota Placeholder

The `/providers/quota` route SHALL display a real provider-usage dashboard, not mocked quota data or a placeholder banner. Its visible page label MUST be `Provider usage`. The page MUST distinguish usage recorded for requests routed through Cortex from upstream provider rate-limit observations, locally configured thresholds, and provider account limits. It MUST NOT present Cortex-routed usage as an account-wide total or infer authoritative account limits from usage, cost, rate-limit ratios, or local thresholds. The page SHALL show historical usage trends, indicate unavailable or unknown data honestly, and retain the last successful result with a visible stale/degraded indication if a refresh fails.

#### Scenario: Navigate to provider usage

- GIVEN the dashboard is available
- WHEN the user navigates to `/providers/quota`
- THEN the page is labeled `Provider usage`
- AND it displays real usage data when available rather than mocked data
- AND it does not display the placeholder/follow-up banner

#### Scenario: Explain Cortex-routed usage coverage

- GIVEN usage records exist for requests routed through Cortex
- WHEN the page displays request, token, or cost totals
- THEN those totals are identified as measured Cortex-routed usage
- AND the page makes clear that they do not necessarily represent the provider account's total usage

#### Scenario: Render historical trends

- GIVEN the API returns historical provider usage trend points
- WHEN the page renders the usage dashboard
- THEN the historical trend data is presented for the returned periods
- AND any estimated cost is labeled as an estimate
- AND cost without available pricing is shown as unknown rather than zero

#### Scenario: Keep local thresholds distinct from provider limits

- GIVEN a provider has locally configured quota-window thresholds
- WHEN the page displays a threshold warning
- THEN the warning identifies the threshold as locally configured
- AND it does not describe that threshold as a provider-issued account limit or remaining quota

#### Scenario: Show observed upstream rate-limit signals only when captured

- GIVEN an upstream provider response includes a rate-limit signal that Cortex actually captured
- WHEN the page displays provider rate-limit information
- THEN it identifies the information as an observed upstream signal
- AND it presents only the captured signal and its available provenance/freshness
- AND it does not treat absence of a signal as proof that no limit exists or that the provider is healthy

#### Scenario: Refresh failure preserves last successful data

- GIVEN the page has displayed a successful usage result
- WHEN a later refresh fails
- THEN the last successful result remains visible
- AND the page visibly indicates that the data is stale or refresh is degraded
- AND the failure is not presented as if the retained values were freshly retrieved

#### Scenario: Self-hosted Ollama has no account quota

- GIVEN a self-hosted Ollama provider has recorded Cortex-routed usage
- WHEN the page displays its usage
- THEN measured usage is shown when available
- AND account quota is identified as not applicable or not provided
- AND the page does not fabricate zero, unlimited, remaining, or exhausted quota values
- AND cost is zero only when explicitly configured/priced as zero; otherwise it is unknown

#### Scenario: Ollama Cloud has no captured rate-limit signal

- GIVEN an Ollama Cloud connection has no captured upstream rate-limit signal
- WHEN the page displays its status
- THEN the signal is shown as not reported or unavailable
- AND the page does not imply a healthy state or invent a limit

#### Scenario: Missing provider usage data

- GIVEN a provider has no matching usage records for the displayed period
- WHEN the page renders that provider
- THEN it communicates that usage data is unavailable or absent for the period
- AND it does not represent missing data as measured zero usage or zero account quota

#### Scenario: Partial refresh or provider data failure

- GIVEN usage data for one provider or field cannot be retrieved while other results are available
- WHEN the page renders the response
- THEN available provider and field data remains visible
- AND the affected provider or field is marked unavailable/degraded
- AND the whole page is not replaced by a global error solely because of that partial failure
