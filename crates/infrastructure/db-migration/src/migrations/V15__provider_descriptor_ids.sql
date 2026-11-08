-- Canonicalize legacy provider spellings to descriptor ids.
--
-- `ProviderKind` serde output ("ollamacloud", "azureopenai") disagreed with
-- `as_str()` ("ollama-cloud", "azure-openai"). Both spellings may exist in
-- stored rows; every row now carries the canonical descriptor id. Reads keep
-- accepting the legacy spellings through the descriptor alias map.
-- Rename only legacy rows whose name is already used by the canonical provider.
-- Probe suffixes against both spellings before changing identity. Including the
-- unique rowid in each suffix keeps generated names distinct from one another.
WITH RECURSIVE candidates(row_id, canonical_id, name, attempt, candidate) AS (SELECT legacy.rowid,
                                                                                     CASE legacy.provider_kind
                                                                                         WHEN 'ollamacloud'
                                                                                             THEN 'ollama-cloud'
                                                                                         ELSE 'azure-openai' END,
                                                                                     legacy.name,
                                                                                     0,
                                                                                     legacy.name ||
                                                                                     ' (migrated ' ||
                                                                                     printf('%020d', legacy.rowid) ||
                                                                                     '-0)'
                                                                              FROM provider_connections AS legacy
                                                                              WHERE legacy.provider_kind IN ('ollamacloud', 'azureopenai')
                                                                                AND EXISTS (SELECT 1
                                                                                            FROM provider_connections AS canonical
                                                                                            WHERE
                                                                                                canonical.provider_kind =
                                                                                                CASE legacy.provider_kind
                                                                                                    WHEN 'ollamacloud'
                                                                                                        THEN 'ollama-cloud'
                                                                                                    ELSE 'azure-openai' END
                                                                                              AND canonical.name = legacy.name)
                                                                              UNION ALL
                                                                              SELECT row_id,
                                                                                     canonical_id,
                                                                                     name,
                                                                                     attempt + 1,
                                                                                     name ||
                                                                                     ' (migrated ' ||
                                                                                     printf('%020d', row_id) ||
                                                                                     '-' ||
                                                                                     (attempt + 1) ||
                                                                                     ')'
                                                                              FROM candidates
                                                                              WHERE EXISTS (SELECT 1
                                                                                            FROM provider_connections AS existing
                                                                                            WHERE
                                                                                                existing.provider_kind IN
                                                                                                (canonical_id,
                                                                                                 CASE canonical_id
                                                                                                     WHEN 'ollama-cloud'
                                                                                                         THEN 'ollamacloud'
                                                                                                     ELSE 'azureopenai' END)
                                                                                              AND existing.name = candidate))
UPDATE provider_connections
SET name = (SELECT candidate
            FROM candidates
            WHERE row_id = provider_connections.rowid
            ORDER BY attempt DESC
            LIMIT 1)
WHERE rowid IN (SELECT row_id FROM candidates);

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
