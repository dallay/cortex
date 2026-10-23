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

-- The composite primary key automatically indexes the series identity. Index the
-- referenced observation ID for bounded retention cleanup and joins.
CREATE INDEX idx_quota_observation_latest_observation
    ON provider_quota_observation_latest(observation_id);

-- Backfill the newest row for each series with deterministic tie-breaking.
-- SQLite window functions are supported by the bundled SQLite library (>= 3.25).
INSERT INTO provider_quota_observation_latest (
    connection_id, metric, scope, window, source, provider_runtime_id, observation_id
)
SELECT connection_id, metric, scope, window, source, provider_runtime_id, id
FROM (
    SELECT connection_id, metric, scope, window, source, provider_runtime_id, id,
           ROW_NUMBER() OVER (
               PARTITION BY connection_id, metric, scope, window, source, provider_runtime_id
               ORDER BY observed_at DESC, id DESC
           ) AS series_rank
    FROM provider_quota_observations
)
WHERE series_rank = 1;
