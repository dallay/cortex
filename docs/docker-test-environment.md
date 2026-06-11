# Docker Test Environment

> ephemeral Rook server + opencode client for testing AI SDK integrations

## Overview

Spins up two containers:

- **rook-server** — Rook server with dashboard embedded, API key auto-bootstrapped on startup
- **test** — Ubuntu container with opencode CLI pre-installed and configured to use the Rook provider

The DB lives in tmpfs (ephemeral), and the API key is generated fresh on each `docker compose up`.

## Quick Start

```bash
# Start the environment
dev/run.sh test-up

# Open a shell in the test client
dev/run.sh test-shell

# Tear down
dev/run.sh test-down
```

## Manual Commands

```bash
# Build images
docker compose -f dev/docker-compose.test.yml build

# Start (daemonized)
docker compose -f dev/docker-compose.test.yml up -d

# View logs
docker compose -f dev/docker-compose.test.yml logs -f

# Shell into test client
docker compose -f dev/docker-compose.test.yml exec test bash

# Stop and remove
docker compose -f dev/docker-compose.test.yml down
```

## Verification

### Health Check

```bash
curl http://127.0.0.1:3773/health
→ {"status":"ok","tag":"rook","version":"0.0.1"}
```

### API Key (auto-generated on startup)

```bash
API_KEY=$(docker exec rook-test-server cat /run/secrets/api_key)
curl http://127.0.0.1:3773/v1/models -H "Authorization: Bearer $API_KEY"
→ {"data":[...],"object":"list"}
```

### Dashboard

Dashboard is embedded in the binary and served at `/dashboard/`:

```bash
curl -I http://127.0.0.1:3773/dashboard/
→ HTTP/1.1 303 See Other (redirect to /login)
```

> **Note:** Use `127.0.0.1` instead of `localhost` on macOS due to IPv6 port-forwarding issues with Docker/Orbstack.

### opencode CLI

```bash
docker exec rook-test-client opencode --version
→ 1.16.2
```

opencode config is at `/root/.config/opencode/opencode.json` with the API key auto-injected from the volume.

## Environment Details

### Rook Server

| Setting     | Value                                               |
|-------------|-----------------------------------------------------|
| Port        | 3773                                                |
| Dashboard   | Embedded at `/dashboard/`                           |
| Database    | tmpfs at `/app/data/rook.db` (ephemeral)            |
| Hash Secret | `test-secret-for-dev-only-do-not-use-in-production` |
| API Key     | Auto-generated via bootstrap on startup             |

### Test Client

| Setting        | Value                                                  |
|----------------|--------------------------------------------------------|
| opencode       | 1.16.2 via official installer                          |
| Config         | `/root/.config/opencode/opencode.json`                 |
| API Key Source | Volume mount from rook-server's `/run/secrets/api_key` |

### Volumes

- `api-key` — shared volume containing the auto-generated API key
    - Server writes: `/run/secrets/api_key`
    - Client reads: `/run/secrets/api_key` (mounted read-only)

## Files

```
dev/
├── docker-compose.test.yml          # Container orchestration
├── Dockerfile.test-server          # Rook server with dashboard
├── Dockerfile.test-client          # Ubuntu + opencode
├── docker/
│   ├── test-server-entrypoint.sh   # Bootstrap + API key generation
│   └── test-client-entrypoint.sh  # Config injection
└── test-configs/
    ├── rook-test.toml              # Rook server config
    └── opencode-test.json          # opencode config (API key placeholder)
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│ rook-test-net │
│                                                          │
│  ┌─────────────────────┐    ┌─────────────────────┐    │
│  │   rook-test-server   │    │    rook-test-client  │    │
│  │                     │    │                     │    │
│  │  /run/secrets/      │◄───┼─api-key volume──────┘    │
│  │ api_key          │    │                     │    │
│  │                     │    │  /root/.config/     │    │
│  │  Rook server │    │  opencode/           │    │
│  │  :3773             │    │  opencode.json       │    │
│  │  /dashboard/       │    │                     │    │
│  │ │    │  opencode CLI │    │
│  └─────────────────────┘    └─────────────────────┘    │
│           ▲ │
│           │ curl /v1/models                                │
└───────────┼───────────────────────────────────────────────┘
            │
    ┌───────▼───────┐
    │ Host │
    │  :3773        │
    └───────────────┘
```

## Troubleshooting

### Connection reset by peer on macOS

Use `127.0.0.1` instead of `localhost`:

```bash
curl http://127.0.0.1:3773/health
```

### Server in bootstrap mode

The entrypoint runs bootstrap automatically. If you see "rook is in bootstrap mode" errors in logs, the entrypoint may have failed. Check logs:

```bash
docker logs rook-test-server
```

### API key not in secrets

Ensure the volume is properly mounted:

```bash
docker exec rook-test-server cat /run/secrets/api_key
```

If empty, restart the environment:

```bash
docker compose -f dev/docker-compose.test.yml down && docker compose -f dev/docker-compose.test.yml up -d
```

### opencode not in PATH

The PATH is set via `ENV PATH="/root/.opencode/bin:${PATH}"` in the Dockerfile. If running a shell manually:

```bash
export PATH="/root/.opencode/bin:$PATH"
```

## Security Notes

- **Test environment only** — never use in production
- Hash secret is hardcoded for testing: `test-secret-for-dev-only-do-not-use-in-production`
- tmpfs means the DB is lost on container restart
- API key is auto-generated and stored in an anonymous Docker volume
