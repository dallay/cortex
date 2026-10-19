-- Upstream-reported quota observations are separate from local usage_history.
CREATE TABLE provider_quota_observations (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_kind         TEXT NOT NULL,
    provider_runtime_id   TEXT NOT NULL,
    connection_id         TEXT,
    metric                TEXT NOT NULL,
    source                TEXT NOT NULL,
    scope                 TEXT,
    window                TEXT,
    quota_limit           REAL,
    remaining             REAL,
    reset_at              TEXT,
    reset_after_milliseconds INTEGER CHECK (reset_after_milliseconds IS NULL OR reset_after_milliseconds >= 0),
    observed_at           TEXT NOT NULL
);

CREATE INDEX idx_quota_observations_provider_time
    ON provider_quota_observations(provider_runtime_id, observed_at DESC);
CREATE INDEX idx_quota_observations_connection_time
    ON provider_quota_observations(connection_id, observed_at DESC);
CREATE INDEX idx_quota_observations_observed_at
    ON provider_quota_observations(observed_at);
