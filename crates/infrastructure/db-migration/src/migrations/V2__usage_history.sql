-- =============================================================================
-- usage_history table
-- =============================================================================
CREATE TABLE usage_history (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id              TEXT NOT NULL,
    provider                TEXT NOT NULL,
    model                   TEXT NOT NULL,
    status                  TEXT NOT NULL,
    requested_tier          TEXT,
    api_key_id              TEXT,
    connection_id           TEXT,
    tokens_prompt           INTEGER,
    tokens_completion       INTEGER,
    tokens_cache_read       INTEGER,
    tokens_cache_creation   INTEGER,
    tokens_reasoning        INTEGER,
    ttft_ms                 INTEGER,
    latency_ms              INTEGER NOT NULL,
    cost_usd                REAL,
    timestamp               TEXT NOT NULL
);

CREATE INDEX idx_usage_history_request_id ON usage_history(request_id);
CREATE INDEX idx_usage_history_provider ON usage_history(provider);
CREATE INDEX idx_usage_history_model ON usage_history(model);
CREATE INDEX idx_usage_history_timestamp ON usage_history(timestamp);
CREATE INDEX idx_usage_history_api_key_id ON usage_history(api_key_id);
CREATE INDEX idx_usage_history_connection_id ON usage_history(connection_id);
