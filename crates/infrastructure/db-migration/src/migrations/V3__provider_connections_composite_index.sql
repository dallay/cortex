-- =============================================================================
-- provider_connections composite index for find_connection_id_by_runtime
-- Supports: SELECT ... FROM provider_connections
--           WHERE provider_runtime_id = ?1 AND is_active = 1
--           ORDER BY priority ASC, created_at DESC LIMIT 1
-- =============================================================================
CREATE INDEX IF NOT EXISTS idx_provider_connections_runtime_active_priority_created
ON provider_connections (provider_runtime_id, is_active, priority ASC, created_at DESC);