# Deployment Guide

This guide covers deploying Rook as a production AI gateway, either locally with Docker or in a cloud environment.

## Prerequisites

- Docker (for containerized deployment)
- [Ollama Cloud API key](https://ollama.com/settings/keys) (for cloud providers)

## Environment Variables

Rook requires several environment variables depending on the features you enable:

### Required for Provider CRUD (`provider_crud.enabled = true`)

```bash
# Encryption passphrase (min 12 characters, keep secret!)
ENCRYPTION_PASSPHRASE="your-secure-passphrase-here"

# Encryption salt (generate once with: openssl rand -base64 16 | tr -d '=' | tr '+/' '-_')
ENCRYPTION_SALT="Z3G83UBdTUkfGGWr-QDnQg"
```

### Required for Client API Key Auth (`auth.api_keys.enabled = true`)

```bash
# Secret for hashing client API keys (generate with: openssl rand -hex 32)
API_KEY_HASH_SECRET="your-hash-secret-here"
```

### Optional

```bash
# Log level (default: info)
RUST_LOG=info

# Config file path (default: ~/.config/cortex/rook.toml)
ROOK_CONFIG=/app/config/rook.toml
```

## Docker Deployment

### Build the Image

```bash
# Using the provided Dockerfile
docker build -f apps/rook/Dockerfile -t rook:latest .

# Or for multi-arch (requires buildx)
docker buildx build --platform linux/amd64,linux/arm64 \
  -f apps/rook/Dockerfile -t rook:latest .
```

### Run with Ollama Cloud

```bash
# Create config directory
mkdir -p rook-config

# Create config file (rook-config/rook.toml)
cat > rook-config/rook.toml << 'EOF'
[server]
host = "0.0.0.0"
port = 8080

[routing]
strategy = "priority"

[cache]
enabled = true
ttl_secs = 300

[database]
db_path = "/app/data/rook.db"

[auth.api_keys]
enabled = false
allow_env_fallback = true

[provider_crud]
enabled = true
EOF

# Run the container
docker run -d \
  --name rook \
  -p 8080:8080 \
  -v $(pwd)/rook-config:/app/config:ro \
  -v rook-data:/app/data \
  -e ENCRYPTION_PASSPHRASE="your-secure-passphrase-min-12-chars" \
  -e ENCRYPTION_SALT="your-generated-salt" \
  -e RUST_LOG=info \
  rook:latest

# Verify it's running
curl http://localhost:8080/health
```

### Docker Compose Example

```yaml
version: '3.8'

services:
  rook:
    image: rook:latest
    ports:
      - "8080:8080"
    volumes:
      - ./rook-config/rook.toml:/app/config/rook.toml:ro
      - rook-data:/app/data
    environment:
      - ENCRYPTION_PASSPHRASE=${ENCRYPTION_PASSPHRASE}
      - ENCRYPTION_SALT=${ENCRYPTION_SALT}
      - RUST_LOG=info
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 30s
      timeout: 5s
      retries: 3
    restart: unless-stopped

volumes:
  rook-data:
```

### Environment File

Create a `.env` file (never commit this to version control):

```bash
# Required for provider_crud
ENCRYPTION_PASSPHRASE=your-secure-passphrase-min-12-chars
ENCRYPTION_SALT=Z3G83UBdTUkfGGWr-QDnQg

# Required for auth.api_keys (optional)
API_KEY_HASH_SECRET=your-hash-secret-here
```

## Adding Providers

After Rook is running, add providers via the REST API:

### Via curl

```bash
# Add Ollama Cloud provider
curl -X POST http://localhost:8080/api/providers \
  -H "Content-Type: application/json" \
  -d '{
    "name": "ollama-cloud-primary",
    "provider_kind": "ollama-cloud",
    "auth_type": "api_key",
    "credentials": {
      "api_key": "ollama-xxxxxxxxxxxxxxxx"
    },
    "is_active": true,
    "priority": 1
  }'

# Verify provider was added
curl http://localhost:8080/api/providers
```

### Via Dashboard

1. Navigate to `http://localhost:8080/providers`
2. Click "Add Provider"
3. Select "Ollama Cloud"
4. Enter your API key from [ollama.com/settings/keys](https://ollama.com/settings/keys)
5. Click "Test Connection" to verify
6. Click "Save"

## Testing the Gateway

```bash
# List available models
curl http://localhost:8080/v1/models \
  -H "Authorization: Bearer your-client-api-key"

# Non-streaming request
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-client-api-key" \
  -d '{
    "model": "llama3.2",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'

# Streaming request
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-client-api-key" \
  -d '{
    "model": "llama3.2",
    "messages": [{"role": "user", "content": "Count to 5"}],
    "stream": true
  }'
```

## Health Checks

```bash
# Overall health
curl http://localhost:8080/health

# Per-provider health (after providers are added)
curl http://localhost:8080/api/telemetry/summary
```

## Production Checklist

- [ ] Generate and securely store `ENCRYPTION_PASSPHRASE` and `ENCRYPTION_SALT`
- [ ] Use a persistent database path (not in-memory)
- [ ] Enable `auth.api_keys` for production if exposing to network
- [ ] Configure rate limiting if needed
- [ ] Set up monitoring/log aggregation
- [ ] Use a reverse proxy (nginx, Caddy, etc.) for TLS termination
- [ ] Consider adding health checks and auto-restart policies

## Kubernetes Deployment

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: rook
spec:
  replicas: 2
  selector:
    matchLabels:
      app: rook
  template:
    metadata:
      labels:
        app: rook
    spec:
      containers:
        - name: rook
          image: rook:latest
          ports:
            - containerPort: 8080
          env:
            - name: ENCRYPTION_PASSPHRASE
              valueFrom:
                secretKeyRef:
                  name: rook-secrets
                  key: passphrase
            - name: ENCRYPTION_SALT
              valueFrom:
                secretKeyRef:
                  name: rook-secrets
                  key: salt
            - name: RUST_LOG
              value: "info"
          volumeMounts:
            - name: config
              mountPath: /app/config
              readOnly: true
            - name: data
              mountPath: /app/data
          resources:
            requests:
              memory: "128Mi"
              cpu: "250m"
            limits:
              memory: "512Mi"
              cpu: "1000m"
          livenessProbe:
            httpGet:
              path: /health
              port: 8080
            initialDelaySeconds: 10
            periodSeconds: 30
          readinessProbe:
            httpGet:
              path: /health
              port: 8080
            initialDelaySeconds: 5
            periodSeconds: 10
      volumes:
        - name: config
          configMap:
            name: rook-config
        - name: data
          persistentVolumeClaim:
            claimName: rook-data
---
apiVersion: v1
kind: Service
metadata:
  name: rook
spec:
  type: LoadBalancer
  ports:
    - port: 80
      targetPort: 8080
  selector:
    app: rook
```

## Generating Secrets

```bash
# Generate encryption salt
openssl rand -base64 16 | tr -d '=' | tr '+/' '-_'
# Example output: Z3G83UBdTUkfGGWr-QDnQg

# Generate API key hash secret
openssl rand -hex 32
# Example output: a1b2c3d4e5f6...

# Generate a client API key (for auth.api_keys)
openssl rand -hex 32
# Example output: rk_live_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```
