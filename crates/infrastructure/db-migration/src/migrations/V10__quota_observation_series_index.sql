-- Keep one row per quota series so preflight reads do not scan observation history.
CREATE TABLE provider_quota_observation_latest (
    connection_id       TEXT,
    metric              TEXT NOT NULL,
    scope               TEXT,
    window              TEXT,
    source              TEXT NOT NULL,
    provider_runtime_id TEXT NOT NULL,
    observation_id      INTEGER NOT NULL REFERENCES provider_quota_observations(id),
    PRIMARY KEY (connection_id, metric, scope, window, source, provider_runtime_id)
);

-- SQLite treats NULL values as distinct in composite keys, so the primary key
-- is not the series uniqueness check. This index supports identity lookups.
CREATE INDEX idx_quota_observation_latest_series
    ON provider_quota_observation_latest(
        connection_id, metric, scope, window, source, provider_runtime_id
    );

-- Backfill the latest row for each series, including deterministic tie-breaking.
INSERT INTO provider_quota_observation_latest (
    connection_id, metric, scope, window, source, provider_runtime_id, observation_id
)
SELECT current.connection_id, current.metric, current.scope, current.window,
       current.source, current.provider_runtime_id, current.id
FROM provider_quota_observations AS current
WHERE NOT EXISTS (
    SELECT 1
    FROM provider_quota_observations AS newer
    WHERE newer.connection_id IS current.connection_id
      AND newer.metric = current.metric
      AND newer.scope IS current.scope
      AND newer.window IS current.window
      AND newer.source = current.source
      AND newer.provider_runtime_id = current.provider_runtime_id
      AND (newer.observed_at > current.observed_at
           OR (newer.observed_at = current.observed_at AND newer.id > current.id))
);
