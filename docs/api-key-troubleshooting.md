# API Key Troubleshooting Guide

## The Problem

When client API keys stop working with `INVALID_API_KEY` errors, it's usually because the **API key hash secret** changed. Rook hashes API keys using HMAC-SHA256 with a secret. If that secret changes, all existing API keys become invalid because they were hashed with a different secret.

## How It Works

```
API Key (raw) ──HMAC-SHA256──> Hash stored in DB
                 ↑
         API_KEY_HASH_SECRET
```

The same raw API key produces **different hashes** with **different secrets**. This is why changing the secret invalidates all keys.

## Prevention: Set the Secret Permanently

### Option 1: Environment Variable (Recommended for Production)

```bash
# Generate a secure secret
openssl rand -hex 32

# Add to your shell profile (~/.zshrc, ~/.bashrc)
export API_KEY_HASH_SECRET="your-generated-secret-here"
```

Then restart your shell or run `source ~/.zshrc`.

### Option 2: Persistent Secret File (Default Behavior)

On first run, Rook auto-generates a secret and stores it at:

```
~/.local/share/cortex/rook/api_key_secret.key
```

**Problem:** If you delete this file or use a different DB path, Rook generates a new secret and all existing API keys break.

### Option 3: Hybrid (Recommended for Development)

Set the env var AND keep the file. This way:

- The env var takes priority (stability)
- The file serves as backup documentation

```bash
# In your shell or .env file
export API_KEY_HASH_SECRET=$(cat ~/.local/share/cortex/rook/api_key_secret.key)
```

## Starting Rook Correctly

Always set both environment variables:

```bash
# From cortex directory
API_KEY_HASH_SECRET=$(cat ~/.local/share/cortex/rook/api_key_secret.key) \
ROOK_CONFIG=$HOME/.config/cortex/rook.toml \
cargo run -p rook
```

Or with just:

```bash
ROOK_CONFIG=$HOME/.config/cortex/rook.toml cargo run -p rook
```

Rook will read `API_KEY_HASH_SECRET` from environment first, then fall back to the file.

## Recovery: When Keys Already Break

If you're seeing `INVALID_API_KEY` errors and the secret changed:

### Step 1: Find the Current Secret

Check if the env var is set:

```bash
echo $API_KEY_HASH_SECRET
```

Check the secret file:

```bash
cat ~/.local/share/cortex/rook/api_key_secret.key
```

### Step 2: Use the Correct Secret When Starting Rook

```bash
API_KEY_HASH_SECRET="correct-secret-here" \
ROOK_CONFIG=$HOME/.config/cortex/rook.toml \
cargo run -p rook
```

### Step 3: Create a New API Key

If the old key was truly lost (hash mismatch can't be reversed):

1. **Via Bootstrap (if admin user has no password):**
   ```bash
   # Get CSRF token
   curl -s http://localhost:3773/login
   
   # Call bootstrap with setup token from logs
   curl -X POST http://localhost:3773/api/bootstrap/setup \
     -H "Content-Type: application/json" \
     -H "X-CSRF-Token: <token-from-login>" \
     -b "csrf_token=<token-from-login>" \
     -d '{"setup_token":"<from-server-logs>","password":"YourSecurePass123!"}'
   ```

2. **Via Dashboard:**
    - Open <http://localhost:3773>
    - Complete bootstrap setup
    - Create new API key from dashboard

### Step 4: Update Your Config

Update the API key in your client configuration (e.g., opencode.json):

```json
{
  "provider": {
    "rook": {
      "options": {
        "apiKey": "rk-your-new-key-here"
      }
    }
  }
}
```

## Verification: Test Your Setup

```bash
# Test with the API key
curl -s http://localhost:3773/v1/models \
  -H "Authorization: Bearer rk-your-api-key-here"

# Should return: {"data":[...{"id":"openai-primary/gpt-4o",...}]}
```

## Quick Reference

| Scenario        | Secret Source                 | Command                      |
|-----------------|-------------------------------|------------------------------|
| First run       | Auto-generated in file        | Just run `cargo run -p rook` |
| Subsequent runs | Use env var or file           | `echo $API_KEY_HASH_SECRET`  |
| Key breaks      | Match the original secret     | Check env var and file       |
| Can't recover   | Delete DB keys, use bootstrap | See Recovery steps above     |

## Common Mistakes

1. **Deleting the secret file** → All keys invalid
2. **Not setting env var** → Different secret on restart if file deleted
3. **Changing DB path** → New secret file location, old keys invalid
4. **Copying DB without secret** → Secret mismatch

## Docker Test Environment

For quick testing without setting up a full Rook instance, use the Docker test environment:

```bash
dev/run.sh test-up
```

This spins up:

- A Rook server with a stable hash secret (`test-secret-for-dev-only-do-not-use-in-production`)
- API key auto-generated via bootstrap on startup
- An opencode client pre-configured to use the Rook provider

### Testing API Keys in Docker

```bash
# Get the auto-generated API key
API_KEY=$(docker exec rook-test-server cat /run/secrets/api_key)

# Test it
curl http://127.0.0.1:3773/v1/models \
  -H "Authorization: Bearer $API_KEY"
```

> **Note:** Use `127.0.0.1` instead of `localhost` on macOS due to IPv6 port-forwarding issues.

See [docker-test-environment.md](docker-test-environment.md) for full details.

## Security Notes

- The hash secret is NOT the API key itself — it's used to hash API keys
- If leaked, an attacker could verify hashes, but cannot reverse-engineer API keys
- For production, always use environment variables, never store secrets in config files
- Rotate the secret periodically (invalidates all existing API keys — plan accordingly)
