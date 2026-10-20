<div align="center">

# 🧠 Cortex

The AI Infrastructure Stack for Production

[![CI](https://img.shields.io/github/actions/workflow/status/dallay/cortex/ci.yml?branch=main&label=CI&logo=github)](https://github.com/dallay/cortex/actions/workflows/ci.yml)
[![License](https://img.shields.io/github/license/dallay/cortex)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.81%2B-DEA584?logo=rust&labelColor=4A4A4A)](https://www.rust-lang.org)
[![Node](https://img.shields.io/badge/node-20%2B-339933?logo=nodedotjs&labelColor=4A4A4A)](https://nodejs.org)
[![Vue](https://img.shields.io/badge/vue-3.4-4FC08D?logo=vuedotjs&labelColor=4A4A4A)](https://vuejs.org)
[![Docker](https://img.shields.io/badge/docker-ready-2496ED?logo=docker&labelColor=4A4A4A)](https://www.docker.com)
[![Last Commit](https://img.shields.io/github/last-commit/dallay/cortex/main?label=last%20commit)](https://github.com/dallay/cortex/commits/main)
[![Stars](https://img.shields.io/github/stars/dallay/cortex?style=flat&color=yellow)](https://github.com/dallay/cortex/stargazers)
[![Ask DeepWiki](https://deepwiki.com/badge.svg)](https://deepwiki.com/dallay/cortex)

**Rook** — AI Proxy & Router · **Huginn** — Local Coding Agent

</div>

---

<p align="center">
  <a href="#-what-is-cortex"><b>What is Cortex</b></a> •
  <a href="#-products"><b>Products</b></a> •
  <a href="#-why-cortex"><b>Why Cortex</b></a> •
  <a href="#-quick-start"><b>Quick Start</b></a> •
  <a href="#-architecture"><b>Architecture</b></a> •
  <a href="#-documentation"><b>Docs</b></a>
</p>

---

## 🎯 What is Cortex?

Cortex is a **production-ready AI infrastructure stack** built in Rust that handles everything between your application and the AI providers.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                                                                              │
│    Your App                    Cortex                    AI Providers         │
│  ┌────────┐              ┌────────────┐           ┌─────────────────┐   │
│  │        │              │            │             │                 │   │
│  │  Any   │────────────▶│   Rook     │───────────▶│  OpenAI         │   │
│  │  SDK   │   OpenAI    │   Proxy    │             │  Anthropic      │   │
│  │        │   Compatible│            │             │  Gemini         │   │
│  │        │◀────────────│  + Cache   │◀───────────│  Ollama         │   │
│  └────────┘   Response   │  + Audit   │  Fallback   │  Groq           │   │
│                          └────────────┘             └─────────────────┘   │
│                                                                              │
└──────────────────────────────────────────────────────────────────────────────┘
```

### The Two Products

| Product | Purpose | What it does |
|---------|---------|--------------|
| **🛡️ Rook** | AI Proxy | Routes requests, handles fallbacks, caches responses, logs everything |
| **🤖 Huginn** | Coding Agent | Your local AI coding partner with native tool access |

---

## 📦 Products

### 🛡️ Rook — AI Proxy & Router

> *"Never let an AI outage break your app again."*

Rook is an OpenAI-compatible proxy that sits between your app and AI providers.

```
Request
    │
    ▼
┌─────────────────────────────────────────────┐
│              Rook Proxy                      │
│                                             │
│  1. Authenticate API key                   │
│  2. Log request (audit)                    │
│  3. Check cache (return if hit)            │
│  4. Route to primary provider               │
│     │                                      │
│     ├── Success → Cache → Response         │
│     │                                      │
│     └── Failure → Try next provider         │
│              │                              │
│              └── All fail → Error response  │
└─────────────────────────────────────────────┘
```

#### Key Features

| Feature | Description |
|---------|-------------|
| 🔌 **OpenAI Compatible** | Drop-in replacement for any OpenAI client |
| 🔄 **Automatic Fallbacks** | Define combo chains: `gpt-4` → `claude-3` → `gemini-pro` |
| 🏷️ **Model Aliasing** | Use `gpt-4-turbo` → maps to actual model in config |
| ⚡ **Smart Caching** | TTL-based response caching to cut costs |
| 📊 **Full Audit Trail** | Every request/response logged to SQLite |
| 🔑 **API Key Management** | Create, revoke, rotate keys from dashboard |
| 🚀 **Streaming** | SSE streaming with tool call support |
| 🌐 **Web Dashboard** | Vue 3 SPA embedded in binary |

#### Providers Supported

```
┌────────────────────────────────────────────────────────────────┐
│                                                                │
│   OpenAI          Anthropic          Google Gemini              │
│   ├─ GPT-4o      ├─ Claude 3.5      ├─ Gemini 1.5 Pro        │
│   ├─ GPT-4       ├─ Claude 3       └─ Gemini 1.5 Flash       │
│   └─ GPT-3.5     └─ Claude 2                                   │
│                                                                │
│   Groq            Ollama              Custom (OpenAI-like)     │
│   ├─ Llama 3.1   ├─ Llama 3         ├─ LM Studio             │
│   └─ Mixtral     ├─ Mistral         └─ Local endpoints       │
│                  └─ CodeLlama                                │
│                                                                │
└────────────────────────────────────────────────────────────────┘
```

📖 [Docs](docs/rook/README.md) · [API Reference](docs/rook/api.md) · [Dashboard](apps/rook/dashboard/README.md) · [Configuration](docs/rook/configuration.md)

---

### 🤖 Huginn — Local Coding Agent

> *"Your AI coding partner that respects your privacy."*

Huginn is a Rust-powered coding assistant that runs locally with access to your codebase.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                                                                          │
│   You: "Refactor the auth module to use async/await"                    │
│                                                                          │
│   Huginn Processing:                                                     │
│                                                                          │
│   ┌─────────────┐     ┌──────────────┐     ┌─────────────────────┐   │
│   │  Parse      │────▶│  LLM Call   │────▶│  Execute Tools      │   │
│   │  Intent     │     │  (Provider) │     │                     │   │
│   └─────────────┘     └──────────────┘     ├─────────────────────┤   │
│                                              │ Read auth/*.rs     │   │
│                                              │ Edit files         │   │
│                                              │ Run cargo check    │   │
│                                              └─────────────────────┘   │
│                                                                          │
│   Response: Here's the refactored auth module...                         │
│                                                                          │
└─────────────────────────────────────────────────────────────────────────┘
```

#### Key Features

| Feature | Description |
|---------|-------------|
| 💻 **Native TUI** | Beautiful terminal interface with Ratatui |
| 🔒 **Privacy First** | Your code stays local — only prompts go to providers |
| 🛠️ **Native Tools** | File ops, terminal, git — all built-in |
| 🔌 **MCP Integration** | Connect to any MCP stdio server |
| 💾 **Session Memory** | SQLite-backed conversation history |
| 📋 **Scoped Context** | Per-repo `AGENTS.md` for instructions |

📖 [Docs](docs/huginn/README.md) · [TUI Guide](docs/huginn/README.md#-tui-guide) · [MCP Setup](docs/huginn/README.md#--mcp-integration)

---

## ✨ Why Cortex?

### The Problem

Building with AI is fragile:

| Pain Point | Impact |
|------------|--------|
| Provider goes down | Your app breaks |
| Need to switch models | Code changes everywhere |
| Audit requirements | No visibility |
| Cost spikes | Budget surprises |
| Multiple providers | SDK hell |

### The Solution

```
┌─────────────────────────────────────────────────────────────────────┐
│                         With Cortex                                │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│   ❌ Provider down?     ──▶  ✅ Automatic failover                  │
│   ❌ Hard-coded models  ──▶  ✅ Model aliases                       │
│   ❌ No audit trail     ──▶  ✅ Full request logging                │
│   ❌ SDK complexity     ──▶  ✅ Single OpenAI-compatible API        │
│   ❌ Cost overruns     ──▶  ✅ Response caching                     │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

### Performance

| Metric | Value |
|--------|-------|
| **Response Time** | <5ms overhead (proxy latency) |
| **Cache Hit** | ~50ms (vs 500ms+ API call) |
| **Memory Footprint** | ~20MB (Rook) |
| **Binary Size** | ~15MB (static, musl) |

---

## 🚀 Quick Start

### Prerequisites

```
┌────────────────────────────────────────────────────────────────┐
│  Tool        │  Version  │  Install                           │
├──────────────┼───────────┼────────────────────────────────────┤
│  Rust        │  1.81+   │  curl --proto '=https' -sSf \       │
│              │          │    https://sh.rustup.rs | sh        │
│  Node.js     │  20+     │  nvm install 20                     │
│  pnpm        │  latest  │  npm i -g pnpm                      │
│  Docker      │  latest  │  docker.com/get-started             │
└────────────────────────────────────────────────────────────────┘
```

### Install Rook

```bash
# Clone the repo
git clone https://github.com/dallay/cortex.git
cd cortex

# Build Rook
cargo build --release -p rook

# Run it
cargo run --release -p rook
```

### Configure Rook

```bash
# Create config file
cat > rook.toml << 'EOF'
[server]
host = "127.0.0.1"
port = 3773

[[providers]]
name = "openai"
type = "openai"
api_key = "sk-..."

[[providers]]
name = "anthropic"
type = "anthropic"
api_key = "sk-ant-..."

[routing]
strategy = "priority"
combos = [
  ["openai/gpt-4o", "anthropic/claude-3-5-sonnet"],
]
EOF
```

### Test It

```bash
# Health check
curl http://localhost:3773/health

# OpenAI-compatible chat completions
curl http://localhost:3773/v1/chat/completions \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer your-api-key" \
  -d '{
    "model": "gpt-4",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'
```

### Open Dashboard

```bash
# Dashboard is embedded at /dashboard/
open http://localhost:3773/dashboard/

# Login with admin credentials (set on first run)
```

---

## 🏗️ Architecture

### Project Structure

```
cortex/
│
├── apps/
│   ├── rook/                    # 🛡️ Rook binary
│   │   ├── src/
│   │   │   ├── main.rs          # Entry point
│   │   │   ├── server.rs        # Axum server
│   │   │   └── routes/          # HTTP handlers
│   │   └── dashboard/           # 📊 Vue SPA (embedded)
│   │       ├── src/views/       # Page components
│   │       ├── src/components/ # UI components
│   │       ├── src/stores/     # Pinia state
│   │       └── e2e/            # Playwright tests
│   │
│   └── huginn/                 # 🤖 Huginn binary
│       └── src/
│
├── crates/
│   │
│   ├── domain/                  # 💼 Business logic (zero deps)
│   │   ├── shared-kernel/       # Core types & traits
│   │   └── rook-core/          # Domain models & ports
│   │
│   ├── application/            # 🎻 Use cases
│   │   └── rook-usecases/      # Business rules
│   │
│   ├── infrastructure/         # 🔌 External adapters
│   │   ├── providers-openai/   # OpenAI client
│   │   ├── providers-anthropic/# Anthropic client
│   │   ├── providers-gemini/   # Google Gemini client
│   │   ├── providers-ollama/   # Ollama client
│   │   ├── providers-groq/     # Groq client
│   │   ├── auth-sqlite/        # API key management
│   │   ├── audit-sqlite/       # Request logging
│   │   ├── cache-memory/       # TTL cache
│   │   └── transport-axum/     # HTTP transport
│   │
│   └── huginn/                 # 🤖 Huginn runtime
│       └── runtime/
│
├── docs/                       # 📖 Documentation
│   ├── rook/                   # Rook docs
│   │   ├── architecture.md
│   │   ├── api.md
│   │   ├── configuration.md
│   │   ├── providers.md
│   │   └── deployment.md
│   └── huginn/                 # Huginn docs
│
└── dev/                        # 🐳 Testing
    ├── compose.yml             # Smoke test
    └── compose.e2e.yml         # Full E2E
```

### Tech Stack

| Layer | Technology | Why |
|-------|------------|-----|
| **Language** | Rust 1.81+ | Memory safety, performance |
| **HTTP Server** | Axum | Ergonomic, async |
| **Runtime** | Tokio | Proven async runtime |
| **Database** | SQLite | Zero-config, portable |
| **Frontend** | Vue 3 + TypeScript | Reactive, typed |
| **Build** | Vite | Fast HMR |
| **TUI** | Ratatui | Native, cross-platform |
| **Testing** | Vitest + Playwright | Fast + E2E |

---

## 🛠️ Development

### Setup

```bash
# Install dependencies
just setup

# Verify tools
just doctor
```

### Common Commands

```bash
# Format, lint, test
just fmt
just clippy
just test

# Run locally
just run              # Rook server
just run-dashboard    # Vite dev server
just huginn-run       # Huginn

# E2E testing
just test-e2e         # Full E2E suite
just dev-up           # Smoke test container

# Full CI locally
just ci-local
```

### Code Quality

```bash
# Rust only
cargo fmt --all
cargo clippy --all -- -D warnings

# Dashboard only
cd apps/rook/dashboard
pnpm typecheck && pnpm build && pnpm exec vitest run
```

📖 [Development Guide](docs/development.md) · [Testing Guide](dev/README.md) · [Contributing](CONTRIBUTING.md)

---

## 📚 Documentation

| Guide | Description |
|-------|-------------|
| [Rook Architecture](docs/rook/architecture.md) | How Rook works internally |
| [API Reference](docs/rook/api.md) | All HTTP endpoints |
| [Configuration](docs/rook/configuration.md) | Complete config reference |
| [Providers](docs/rook/providers.md) | Provider setup & quirks |
| [Deployment](docs/rook/deployment.md) | Production deployment |
| [Huginn Docs](docs/huginn/README.md) | Coding agent documentation |
| [Dev Testing](dev/README.md) | Local testing with Docker |

---

## 📊 Project Stats

<div align="center">

| Metric | Value |
|--------|-------|
| 🦀 **Rust Version** | 1.81+ (MSRV) |
| 📦 **Crates** | 15+ packages |
| 🌐 **Providers** | 5 (OpenAI, Anthropic, Gemini, Ollama, Groq) |
| 📄 **License** | MIT |
| ⭐ **Stars** | [GitHub](https://github.com/dallay/cortex/stargazers) |

</div>

---

## 🤝 Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for:

- Development setup
- Code style guidelines
- Pull request process
- Issue templates

We follow the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct).

---

## 📄 License

MIT License. See [LICENSE](LICENSE).

---

## 👥 Community

| Channel | Purpose |
|---------|---------|
| 💬 [Discussions](https://github.com/dallay/cortex/discussions) | Questions & ideas |
| 🐛 [Issues](https://github.com/dallay/cortex/issues) | Bug reports |
| 🌐 [DeepWiki](https://deepwiki.com/dallay/cortex) | AI-powered docs |

---

<div align="center">

**Built with Rust** · **Designed for Production** · **Ready for Your AI Stack**

</div>
