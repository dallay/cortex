UPDATE provider_connections
SET api_format = 'anthropic'
WHERE provider_kind = 'anthropic' AND api_format = 'openai';
