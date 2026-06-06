# Cache Operations Guide

## Overview

Rook implements a dual-layer caching strategy to minimize latency, reduce provider costs, and improve reliability:

1. **Layer 1 (Signature Cache)**: Request deduplication using SHA-256 signatures
2. **Layer 2 (Token Cache)**: Provider-side token caching (e.g., Anthropic prompt caching)

Both layers work independently but complement each other:
- **Signature cache** serves identical requests from memory (fastest)
- **Token cache** reduces provider-side computation costs when signature cache misses

## Architecture

```
Client Request
    ↓
Signature Cache (Layer 1)
    ↓ (miss)
Provider Request with cache-control header
    ↓
Provider (Layer 2 token cache)
    ↓
Response with x-cache header
    ↓
Store in Signature Cache + Update Token Metrics
    ↓
Client Response
```

## Monitoring Cache Performance

### Unified Cache Stats Endpoint

```bash
curl http://localhost:8080/api/cache/stats
```

**Response:**

```json
{
  "signature_cache": {
    "hits": 1247,
    "misses": 356,
    "hit_rate": 0.7779,
    "entries": 142,
    "evictions": 23
  },
  "token_cache": {
    "hits": 89,
    "misses": 267,
    "tokens_saved": 1456789,
    "estimated_cost_saved_usd": 2.91
  },
  "combined": {
    "total_requests": 1603,
    "cached_requests": 1336,
    "cache_rate": 0.8334
  }
}
```

### Key Metrics

#### Signature Cache Metrics

| Metric       | Meaning                                    | Target         |
|--------------|--------------------------------------------|----------------|
| `hit_rate`   | Percentage of requests served from cache   | > 0.60 (60%)   |
| `entries`    | Current cache size                         | Monitor trend  |
| `evictions`  | TTL or capacity-based removals             | Low is better  |

**Low hit rate (<40%)**: Indicates high request diversity or TTL too short.

**High evictions**: Cache TTL may be too short or `max_entries` too low.

#### Token Cache Metrics

| Metric                       | Meaning                                  | Notes                          |
|------------------------------|------------------------------------------|--------------------------------|
| `hits`                       | Provider-side cache hits                 | Depends on provider support    |
| `tokens_saved`               | Input tokens not recomputed              | Direct cost savings            |
| `estimated_cost_saved_usd`   | Estimated cost savings in USD            | Conservative estimate          |

**High token cache hits**: Provider-side caching is working well (requires `mode = "auto"` or `"always"`).

**Zero token hits**: Either `token_cache.mode = "never"` or providers don't support token caching.

#### Combined Metrics

| Metric            | Meaning                                      | Target         |
|-------------------|----------------------------------------------|----------------|
| `cache_rate`      | Overall cache effectiveness                  | > 0.70 (70%)   |
| `cached_requests` | Signature hits + Token hits                  | Maximize       |

**Combined cache rate**: Measures total caching efficiency across both layers.

### Health Endpoint

The `/health` endpoint includes cache stats for quick checks:

```bash
curl http://localhost:8080/health
```

```json
{
  "status": "healthy",
  "cache_stats": { /* same as /api/cache/stats */ }
}
```

## Configuration Tuning

### Signature Cache (Layer 1)

```toml
[cache]
enabled = true
ttl_secs = 300  # 5 minutes default
max_entries = 10000  # Optional capacity limit

[cache.signature_cache]
enabled = true
inspection_endpoints = true  # Enable /api/cache/signatures
```

**Tuning Guidelines:**

| Scenario                      | Recommended TTL | Reasoning                                      |
|-------------------------------|-----------------|------------------------------------------------|
| High request diversity        | 60-180 seconds  | Shorter TTL, fewer stale entries               |
| Repetitive workloads          | 300-900 seconds | Longer TTL, maximize hit rate                  |
| Real-time data requirements   | 30-60 seconds   | Minimize staleness                             |
| Development/testing           | 10-30 seconds   | Fast iteration, avoid stale responses          |

**max_entries**: Set based on available memory (each entry ~1-10KB depending on response size).

### Token Cache (Layer 2)

```toml
[cache.token_cache]
mode = "auto"  # "auto" | "always" | "never"
providers = []  # Empty = default providers (anthropic, deepseek, qwen, zai)
```

**Mode Selection:**

| Mode     | Use Case                                                                 |
|----------|--------------------------------------------------------------------------|
| `never`  | Default. Disable provider-side caching (safest, no header injection).    |
| `auto`   | Enable only for known providers (recommended for production).            |
| `always` | Force-enable for all providers (experimental, may cause issues).         |

**Provider List:**

```toml
# Enable only for Anthropic
[cache.token_cache]
mode = "auto"
providers = ["anthropic"]

# Enable for multiple providers
[cache.token_cache]
mode = "auto"
providers = ["anthropic", "deepseek", "qwen"]
```

**Prefix Matching**: `"anthropic"` matches `"anthropic"`, `"anthropic-v2"`, `"anthropic-prod"`, etc.

## Cost Savings Calculation

Token cache cost savings are estimated using average provider pricing.

**Formula:**

```
cost_saved_usd = (tokens_saved / 1000.0) * 0.002
```

**Assumptions:**
- Average cost: **$0.002 per 1K input tokens**
- Conservative estimate (actual savings vary by model tier)
- Only input tokens are tracked (providers cache input, not output)

**Example:**

If `tokens_saved = 1,456,789`:
```
cost_saved_usd = (1,456,789 / 1000.0) * 0.002 = $2.91
```

**Real-World Pricing (for reference):**
- Anthropic Claude Opus: $15 per 1M input tokens = $0.015 per 1K
- Anthropic Claude Sonnet: $3 per 1M input tokens = $0.003 per 1K
- DeepSeek: ~$0.14 per 1M input tokens = $0.00014 per 1K

The $0.002 per 1K estimate is conservative and applies across providers.

## Inspection and Debugging

### List Cached Signatures

```bash
curl http://localhost:8080/api/cache/signatures
```

Returns all cached entries with metadata (signature, model, provider, expiration, hit count).

**Use Cases:**
- Verify cache is working
- Identify hot cache keys
- Debug unexpected cache behavior

### Retrieve Cached Response by Signature

```bash
curl http://localhost:8080/api/cache/signature/<signature>
```

Returns the full cached `CompletionResponse` for a given signature.

**Use Cases:**
- Validate cached content matches expectations
- Debug cache staleness issues
- Compare cached vs. fresh responses

### Clear Cache

```bash
# Clear entire cache
curl -X DELETE http://localhost:8080/api/cache

# Clear specific entry
curl -X DELETE http://localhost:8080/api/cache/<signature>
```

**When to Clear:**
- After configuration changes (TTL, providers)
- Force fresh responses after provider issues
- Testing/development

## Troubleshooting

### Low Signature Cache Hit Rate (<40%)

**Causes:**
1. High request diversity (many unique requests)
2. TTL too short (entries expire before reuse)
3. Cache disabled or misconfigured

**Solutions:**
- Increase `ttl_secs` (e.g., 300 → 600)
- Check `cache.enabled = true` and `cache.signature_cache.enabled = true`
- Review request patterns (are there truly repeated requests?)

### Zero Token Cache Hits

**Causes:**
1. `token_cache.mode = "never"` (default)
2. Provider doesn't support token caching (e.g., OpenAI)
3. `cache-control` header not being injected

**Solutions:**
- Set `token_cache.mode = "auto"` or `"always"`
- Verify provider supports token caching (Anthropic, DeepSeek, Qwen, ZAI)
- Check logs for `cache-control` header injection

### High Evictions

**Causes:**
1. `max_entries` capacity limit reached
2. TTL too short for workload

**Solutions:**
- Increase `max_entries` (if memory allows)
- Increase `ttl_secs` to reduce churn
- Monitor memory usage (`entries * avg_response_size`)

### Token Cache Not Working for Anthropic

**Checklist:**
1. Verify `token_cache.mode = "auto"` or `"always"`
2. Verify provider ID starts with `"anthropic"` (prefix match)
3. Check Anthropic responses for `x-cache` header (use network inspector)
4. Ensure `cache-control: max-stale=3600` is sent (check request logs)

If `x-cache: hit` appears but metrics don't increment, check parsing logic in `providers-anthropic/src/lib.rs`.

## Production Recommendations

### Configuration

```toml
[cache]
enabled = true
ttl_secs = 300  # 5 minutes
max_entries = 50000  # ~50K entries, ~250MB-500MB RAM

[cache.signature_cache]
enabled = true
inspection_endpoints = false  # Disable in production for security

[cache.token_cache]
mode = "auto"
providers = ["anthropic", "deepseek"]  # Explicit provider list
```

### Monitoring Alerts

| Metric                   | Alert Threshold | Action                                |
|--------------------------|-----------------|---------------------------------------|
| `signature_cache.hit_rate` | < 0.40        | Investigate request patterns, tune TTL |
| `cache_rate`             | < 0.60          | Review cache configuration             |
| `evictions` (rate)       | High velocity   | Increase `max_entries` or TTL          |
| `token_cache.hits`       | Zero for 1h     | Verify token cache config              |

### Security

- **Disable inspection endpoints in production**: Set `inspection_endpoints = false` to prevent leaking cached responses.
- **Authenticate cache management**: DELETE endpoints should require session auth (current implementation is open — TODO).
- **Audit cache clears**: Log all manual cache clear operations for forensics.

## Appendix: Provider Token Cache Support

| Provider    | Token Cache Support | Header Sent                     | Response Header  | Notes                        |
|-------------|---------------------|---------------------------------|------------------|------------------------------|
| Anthropic   | ✅ Yes              | `cache-control: max-stale=3600` | `x-cache: hit\|miss` | Prompt caching (input only) |
| DeepSeek    | ✅ Yes              | `cache-control: max-stale=3600` | `x-cache: hit\|miss` | Similar to Anthropic         |
| Qwen        | ✅ Yes              | `cache-control: max-stale=3600` | `x-cache: hit\|miss` | Alibaba Cloud models         |
| ZAI         | ✅ Yes              | `cache-control: max-stale=3600` | `x-cache: hit\|miss` | ZAI provider                 |
| OpenAI      | ❌ No               | Not sent                        | N/A              | No native prompt caching     |
| Gemini      | ❌ No               | Not sent                        | N/A              | No native prompt caching     |
| Groq        | ❌ No               | Not sent                        | N/A              | No native prompt caching     |
| Ollama      | ❌ No               | Not sent                        | N/A              | Local model, no caching API  |

**Default providers** (when `providers = []` and `mode = "auto"`): `anthropic`, `deepseek`, `qwen`, `zai`
