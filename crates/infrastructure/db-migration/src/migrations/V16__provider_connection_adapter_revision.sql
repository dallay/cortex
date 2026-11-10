ALTER TABLE provider_connections
    ADD COLUMN adapter_revision INTEGER NOT NULL DEFAULT 1
    CHECK (adapter_revision >= 1);
