ALTER TABLE provider_connections
    ADD COLUMN supported_models TEXT NOT NULL DEFAULT '[]';

ALTER TABLE provider_connections
    ADD COLUMN api_format TEXT NOT NULL DEFAULT 'openai';
