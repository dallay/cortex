-- V1__allowed_models_providers.sql
-- Adds allowed_models_json and allowed_providers_json restriction columns to api_keys.
-- Empty JSON array ('[]') means unrestricted (all models/providers allowed).

ALTER TABLE api_keys ADD COLUMN allowed_models_json   TEXT NOT NULL DEFAULT '[]';
ALTER TABLE api_keys ADD COLUMN allowed_providers_json TEXT NOT NULL DEFAULT '[]';
