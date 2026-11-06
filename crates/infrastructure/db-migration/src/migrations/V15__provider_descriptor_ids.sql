-- Canonicalize legacy provider spellings to descriptor ids.
--
-- `ProviderKind` serde output ("ollamacloud", "azureopenai") disagreed with
-- `as_str()` ("ollama-cloud", "azure-openai"). Both spellings may exist in
-- stored rows; every row now carries the canonical descriptor id. Reads keep
-- accepting the legacy spellings through the descriptor alias map.
UPDATE provider_connections
SET provider_kind = 'ollama-cloud'
WHERE provider_kind = 'ollamacloud';

UPDATE provider_connections
SET provider_kind = 'azure-openai'
WHERE provider_kind = 'azureopenai';

UPDATE provider_quota_observations
SET provider_kind = 'ollama-cloud'
WHERE provider_kind = 'ollamacloud';

UPDATE provider_quota_observations
SET provider_kind = 'azure-openai'
WHERE provider_kind = 'azureopenai';
