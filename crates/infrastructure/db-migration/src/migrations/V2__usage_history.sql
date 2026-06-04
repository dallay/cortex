-- =============================================================================
-- usage_history table
-- =============================================================================
CREATE TABLE usage_history (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id              TEXT NOT NULL,
    provider                TEXT NOT NULL,
    model                   TEXT NOT NULL,
    status                  TEXT NOT NULL CHECK (status IN ('success', 'failure', 'rate_limited', 'timeout')),
    requested_tier          TEXT,
    api_key_id              TEXT,
    connection_id           TEXT,
    tokens_prompt           INTEGER CHECK (tokens_prompt IS NULL OR tokens_prompt >= 0),
    tokens_completion       INTEGER CHECK (tokens_completion IS NULL OR tokens_completion >= 0),
    tokens_cache_read       INTEGER CHECK (tokens_cache_read IS NULL OR tokens_cache_read >= 0),
    tokens_cache_creation   INTEGER CHECK (tokens_cache_creation IS NULL OR tokens_cache_creation >= 0),
    tokens_reasoning        INTEGER CHECK (tokens_reasoning IS NULL OR tokens_reasoning >= 0),
    ttft_ms                 INTEGER CHECK (ttft_ms IS NULL OR ttft_ms >= 0),
    latency_ms              INTEGER NOT NULL CHECK (latency_ms >= 0),
    cost_usd                REAL CHECK (cost_usd IS NULL OR cost_usd >= 0.0),
    timestamp               TEXT NOT NULL
);

CREATE INDEX idx_usage_history_request_id ON usage_history(request_id);
CREATE INDEX idx_usage_history_provider ON usage_history(provider);
CREATE INDEX idx_usage_history_model ON usage_history(model);
CREATE INDEX idx_usage_history_timestamp ON usage_history(timestamp);
CREATE INDEX idx_usage_history_api_key_id ON usage_history(api_key_id);
CREATE INDEX idx_usage_history_connection_id ON usage_history(connection_id);

-- Composite indexes for common date-range+filter query patterns
CREATE INDEX idx_usage_history_timestamp_provider ON usage_history(timestamp, provider);
CREATE INDEX idx_usage_history_timestamp_api_key_id ON usage_history(timestamp, api_key_id);
CREATE INDEX idx_usage_history_timestamp_provider_model ON usage_history(timestamp, provider, model);