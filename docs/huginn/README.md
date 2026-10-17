<div align="center">

# 🤖 Huginn

Local Rust coding assistant with native tools and MCP integration

[![Rust](https://img.shields.io/badge/rust-1.89%2B-DEA584?logo=rust)](https://www.rust-lang.org)
[![SQLite](https://img.shields.io/badge/sqlite-3-003B57?logo=sqlite)](https://www.sqlite.org)
[![MCP](https://img.shields.io/badge/MCP-ready-7C3AED?logo=modelcontextprotocol)](https://modelcontextprotocol.io)
[![License](https://img.shields.io/github/license/dallay/cortex)](https://github.com/dallay/cortex/blob/main/LICENSE)

**Your local AI coding partner** — runs locally with access to your codebase.

[Huginn Architecture](#-architecture) · [Installation](#-installation) · [Usage](#-usage) · [TUI Guide](#-tui-guide) · [Providers](providers-ui.md)

</div>

---

## 🎯 What is Huginn?

Huginn is a **local coding agent** built in Rust that works with your codebase. Unlike cloud-based AI assistants, Huginn runs entirely on your machine.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         Huginn: Local-First AI                          │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│   ┌─────────────────────────────────────────────────────────────────┐  │
│   │                                                                 │  │
│   │   Your Machine                    Cloud Providers              │  │
│   │   ────────────                    ───────────────              │  │
│   │                                                                 │  │
│   │   ┌───────────┐                   ┌─────────────┐             │  │
│   │   │  Huginn   │ ─── API ───────▶ │  OpenAI     │             │  │
│   │   │  (Local)  │                   │  Anthropic  │             │  │
│   │   └─────┬─────┘                   └─────────────┘             │  │
│   │         │                                                      │  │
│   │         │  Local Tools                                         │  │
│   │         ▼                                                      │  │
│   │   ┌───────────┐                                               │  │
│   │   │ Your Code │ ◀── Read/Write/Execute                        │  │
│   │   └───────────┘                                               │  │
│   │                                                                 │  │
│   │   🔒 Code stays local — prompts and context sent to providers  │  │
│   │                                                                 │  │
│   └─────────────────────────────────────────────────────────────────┘  │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Key Principles

| Principle | Description |
|-----------|-------------|
| 🔒 **Privacy-first** | Code stays local, only model requests go to cloud |
| 💻 **Native tools** | File ops, terminal, git — all built-in |
| 🔌 **Extensible** | MCP stdio client for custom tools |
| 📝 **Scoped context** | `AGENTS.md` controls what Huginn sees |
| 💾 **Session memory** | SQLite-backed conversation history |

---

## ✨ Features

| Feature | Description |
|---------|-------------|
| 🖥️ **Ratatui TUI** | Beautiful terminal interface with colors and layout |
| 🔧 **Native tools** | File read/write, terminal execution, git operations |
| 🔌 **MCP integration** | Connect to any MCP stdio server |
| 💾 **SQLite sessions** | Persistent conversation history |
| 📋 **Scoped context** | Per-repo `AGENTS.md` for agent instructions |
| 🎯 **Tool calling** | Automatic tool selection based on task |
| 📤 **JSON mode** | Machine-readable output for automation |
| 🌐 **Provider agnostic** | OpenAI-compatible API support |

---

## 🏗️ Architecture

```
huginn/
│
├── src/
│   ├── main.rs              # Entry point, CLI parsing
│   │
│   ├── cli/                 # Command handlers
│   │   ├── chat.rs          # Interactive chat mode
│   │   ├── run.rs           # Single command mode
│   │   └── sessions.rs      # Session management
│   │
│   └── runtime/
│       ├── mod.rs           # Core runtime
│       ├── provider.rs      # LLM provider abstraction
│       ├── tools/           # Native tools
│       │   ├── file.rs      # File operations
│       │   ├── terminal.rs  # Shell commands
│       │   ├── git.rs       # Git operations
│       │   └── mod.rs       # Tool registry
│       │
│       ├── mcp/             # MCP client
│       │   ├── client.rs    # Stdio client
│       │   └── protocol.rs  # MCP protocol
│       │
│       ├── session.rs       # SQLite session storage
│       ├── context.rs       # Context building
│       └── ui/              # Ratatui UI components
│
├── runtime/                 # Huginn runtime crate (shared)
│   ├── src/
│   │   ├── service_kernel.rs
│   │   ├── streaming_adapter.rs  # OpenAI-compatible streaming
│   │   ├── tools.rs
│   │   └── sessions.rs
│   │
│   └── tests/
│       └── fixtures/
│
└── AGENTS.md               # Per-repo agent instructions
```

### Runtime Flow

```
User Input
    │
    ▼
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│   Parser    │ ──▶ │   Provider   │ ──▶ │  Response   │
│  (Command)  │     │  (LLM Call)  │     │  (Streaming)│
└─────────────┘     └──────┬───────┘     └──────┬──────┘
                           │
                           ▼
                    ┌──────────────┐
                    │   Tool Call   │
                    │  Execution    │
                    └──────┬───────┘
                           │
        ┌──────────────────┼──────────────────┐
        ▼                  ▼                  ▼
┌──────────────┐   ┌──────────────┐   ┌──────────────┐
│ File System  │   │   Terminal   │   │     Git      │
└──────────────┘   └──────────────┘   └──────────────┘
```

---

## 🚀 Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/dallay/cortex.git
cd cortex

# Install globally
cargo install --path apps/huginn --locked

# Or build only
cargo build --release -p huginn
```

### Requirements

| Requirement | Version | Notes |
|-------------|---------|-------|
| Rust | 1.89+ | MSRV may be higher |
| Cargo | latest | Included with Rust |
| SQLite | system | Usually pre-installed |

### Verify Installation

```bash
huginn --version
```

---

## 📖 Usage

### Quick Start

```bash
# 1. Set environment variables
export HUGINN_BASE_URL=https://api.openai.com/v1
export HUGINN_MODEL=gpt-4o
export HUGINN_API_KEY=your-api-key

# 2. Run in chat mode
huginn --workspace /path/to/your/project chat

# 3. Or run a single command
huginn --workspace /path/to/your/project run "Explain this codebase"
```

### Command Reference

| Command | Description |
|---------|-------------|
| `huginn chat` | Interactive TUI chat (default) |
| `huginn run "<prompt>"` | Single command, exit |
| `huginn sessions` | List saved sessions |
| `huginn resume <uuid>` | Continue a session |
| `huginn --json run "<prompt>"` | JSON output (for scripting) |

### Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `HUGINN_BASE_URL` | For non-mock providers | API endpoint (include `/v1`) |
| `HUGINN_MODEL` | For non-mock providers | Model name |
| `HUGINN_API_KEY` | For auth-required providers | API key (optional for mock/Ollama) |

---

## 🖥️ TUI Guide

When you run `huginn chat`, you get a Ratatui-based terminal interface.

### Layout

```
┌─────────────────────────────────────────────────────────────────────────┐
│ Huginn - /path/to/workspace                              [?] [⚙] [✕] │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│  ┌─────────────────────────────────────────────────────────────────┐   │
│  │                                                                 │   │
│  │  🤖 Assistant                                                  │   │
│  │  ─────────────────                                             │   │
│  │  Welcome to Huginn! Type / for commands.                       │   │
│  │                                                                 │   │
│  │  👤 You                                                         │   │
│  │  ─────────────────                                             │   │
│  │  Explain the project structure                                  │   │
│  │                                                                 │   │
│  │  🤖 Assistant                                                  │   │
│  │  ─────────────────                                             │   │
│  │  This is a Rust monorepo with...                               │   │
│  │                                                                 │   │
│  └─────────────────────────────────────────────────────────────────┘   │
│                                                                         │
├─────────────────────────────────────────────────────────────────────────┤
│ > Explain the project structure                              [Send]   │
│                                                                         │
│ Model: gpt-4o | Provider: OpenAI | Tokens: 1,234                      │
└─────────────────────────────────────────────────────────────────────────┘
```

### Commands (type `/` to open)

| Command | Description |
|---------|-------------|
| `/login` | Configure provider credentials |
| `/providers` | Switch between providers |
| `/model` | Change the active model |
| `/logout` | Clear stored credentials |
| `/exinfo` | System information |
| `/clear` | Clear current conversation |
| `/quit` | Exit Huginn |

### `/settings` modal

Open with `/settings` (chat mode only — `--line-mode` excludes the modal).
The dialog hosts five tabs in the order `theme · indicators · sound · toasts · integrations`.
Navigate with `←/→` (or `Tab`/`BackTab`) and pick rows with `↑/↓`. Enter writes
the working copy to `config.toml` under `[ui]` via `toml_edit` and shows
`Settings saved`; Esc discards without touching the file and shows
`Settings discarded`. v1 persists the values only — none of the tabs is
wired into its runtime subsystem yet, so the dialog is a shell for
follow-up changes. The `integrations` tab exposes a single
`(+) Add provider…` row that surfaces `Add provider not implemented yet`.

The `[ui]` schema lives at the bottom of
[config.example.toml](../../apps/huginn/config.example.toml).
Hand-added keys under `[ui]` are tolerated on load and preserved across
rounds (`toml_edit::DocumentMut`). The active precedence chain is
`Modal > Settings > PluginDialog > completion`: an approval arriving
while the modal is open disposes of the settings dialog first, and Esc
inside settings never reaches the cancel-turn branch.

### Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+C` | Cancel current request |
| `Ctrl+L` | Clear screen |
| `Ctrl+Enter` | Send message |
| `↑/↓` | History navigation |

---

## 🔧 Configuration

### AGENTS.md

Place an `AGENTS.md` file in your repository root to scope Huginn's context:

```markdown
# Agent Instructions

## Project Context
This is a Rust monorepo with Rook (AI proxy) and Huginn (coding agent).

## Coding Standards
- Use 2-space indentation
- Run `cargo fmt` before commits
- Follow Rust idioms

## Available Commands
- `just dev` - Start development
- `just test` - Run tests
- `just ci-local` - Full CI locally
```

### Provider Configuration

Huginn supports OpenAI-compatible APIs:

```bash
# OpenAI
export HUGINN_BASE_URL=https://api.openai.com/v1
export HUGINN_MODEL=gpt-4o

# Anthropic (via proxy)
export HUGINN_BASE_URL=http://localhost:3773/v1
export HUGINN_MODEL=claude-3-sonnet-20240229

# Local Ollama
export HUGINN_BASE_URL=http://localhost:11434/v1
export HUGINN_MODEL=llama3
```

### Mock Provider (Offline Testing)

```bash
# Test without real API calls
huginn --provider mock --workspace /path/to/repo run "list files"
```

The mock provider responds with predefined examples and does not perform real LLM calls.

---

## 🔌 MCP Integration

Connect Huginn to MCP servers for extended capabilities via CLI configuration:

```toml
# huginn.toml (or pass via --config)
[[mcp]]
name = "filesystem"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem@0.6.3", "."]

[[mcp]]
name = "git"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-git@0.5.0"]
```

```bash
# From the Cortex repository root; "." allows filesystem access to this workspace
huginn --workspace . --config huginn.toml chat
```

> ⚠️ **Security Notice:** MCP servers run with your OS permissions and are not sandboxed. Only connect to servers you trust.

### Available MCP Servers

| Server | Version | Purpose |
|--------|---------|---------|
| `@modelcontextprotocol/server-filesystem` | 0.6.3 | File operations |
| `@modelcontextprotocol/server-git` | 0.5.0 | Git operations |
| `@modelcontextprotocol/server-brave` | latest | Web search |
| `@modelcontextprotocol/server-slack` | latest | Slack integration |

---

## 📝 Examples

### Explain Code

```bash
huginn --workspace ./my-project run "Explain the main.rs entry point"
```

### Refactor Code

```bash
huginn --workspace ./my-project run "Refactor the auth module to use async/await"
```

### Write Tests

```bash
huginn --workspace ./my-project run "Write unit tests for the UserService"
```

### Generate Documentation

```bash
huginn --workspace ./my-project run "Add doc comments to the public API"
```

### Git Operations

```bash
huginn --workspace ./my-project run "Create a commit for the new feature with good message"
```

### JSON Output (CI/CD)

```bash
huginn --workspace ./my-project --json run "Analyze the code and return metrics"
```

---

## 🧪 Testing

```bash
# Run all tests
cargo test -p huginn

# With coverage
cargo tarpaulin -p huginn
```

---

## 🔒 Security

| Concern | Mitigation |
|---------|------------|
| API keys | Use environment variables, not config files |
| File access | Workspace-scoped — Huginn only accesses specified directory |
| Shell execution | Terminal commands require explicit approval |
| MCP tools | MCP calls require explicit approval |
| Session data | SQLite DB stored locally (no built-in encryption) |

### Best Practices

1. **Never commit API keys** — use environment variables
2. **Review tool calls** — Huginn shows what it will execute
3. **Scope workspaces** — Use `--workspace` to limit access
4. **Audit sessions** — Check `huginn sessions` for history

---

## 📚 Resources

| Resource | Description |
|---------|-------------|
| [Architecture](adr/README.md) | Design decisions and ADRs |
| [Providers UI](providers-ui.md) | Provider configuration in TUI |
| [Rook](../rook/README.md) | Companion AI proxy |
| [Cortex](../README.md) | Full project documentation |

---

## 📄 License

MIT License - same as [Cortex repository](https://github.com/dallay/cortex).

---

## 🙏 Acknowledgments

- Built with [Ratatui](https://ratatui.rs/) for the TUI
- Inspired by [Claude](https://claude.ai/) and [Cursor](https://cursor.sh/)
- MCP protocol by [Model Context Protocol](https://modelcontextprotocol.io)

---

<div align="center">

**Rust** · **Privacy-first** · **Local-first** · **MCP-native**

</div>
