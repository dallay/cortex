-- V5: Model Aliases Table
-- Provides stable alias names that resolve to canonical model IDs

CREATE TABLE IF NOT EXISTS model_aliases (
    alias TEXT PRIMARY KEY NOT NULL,
    canonical TEXT NOT NULL,
    provider_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Index for lookups by canonical model (useful for cycle detection)
CREATE INDEX IF NOT EXISTS idx_model_aliases_canonical ON model_aliases(canonical);

-- Index for provider-scoped queries (future enhancement)
CREATE INDEX IF NOT EXISTS idx_model_aliases_provider ON model_aliases(provider_id);
