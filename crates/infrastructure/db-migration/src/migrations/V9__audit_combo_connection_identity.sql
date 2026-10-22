-- Preserve combo-step and effective provider identity in audit records.
ALTER TABLE audit ADD COLUMN combo_id TEXT;
ALTER TABLE audit ADD COLUMN combo_step_index INTEGER;
ALTER TABLE audit ADD COLUMN connection_id TEXT;
ALTER TABLE audit ADD COLUMN provider_runtime_id TEXT;

CREATE INDEX idx_audit_combo_id ON audit(combo_id);
CREATE INDEX idx_audit_connection_id ON audit(connection_id);
