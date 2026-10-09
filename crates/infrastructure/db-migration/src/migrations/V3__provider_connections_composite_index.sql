-- =============================================================================
-- provider_connections composite index for runtime lookup
-- Speeds up active-connection lookup ordered by priority and recency.
-- =============================================================================
CREATE INDEX IF NOT EXISTS idx_provider_connections_runtime_active_priority_created
ON provider_connections (provider_runtime_id, is_active, priority ASC, created_at DESC);