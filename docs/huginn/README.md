<div align="center">

# 🤖 Huginn

Local Rust coding assistant with native tools and MCP integration

[![Rust MSRV](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Fdallay%2Fcortex%2Fmain%2Fapps%2Fhuginn%2FCargo.toml&query=%24.package%5B%27rust-version%27%5D&label=Rust%20MSRV&suffix=%2B&color=DEA584&logo=rust)](https://www.rust-lang.org)
<!-- Huginn MSRV from apps/huginn/Cargo.toml -->
[![SQLite](https://img.shields.io/badge/sqlite-3-003B57?logo=sqlite)](https://www.sqlite.org)
[![MCP](https://img.shields.io/badge/MCP-ready-7C3AED?logo=modelcontextprotocol)](https://modelcontextprotocol.io)
[![License](https://img.shields.io/github/license/dallay/cortex)](https://github.com/dallay/cortex/blob/main/LICENSE)

**Your local AI coding partner** — runs locally with access to your codebase.

[Huginn Architecture](#-architecture) · [Installation](#-installation) · [Usage](#-usage) · [TUI Guide](#-tui-guide) · [Providers](providers-ui.md)

</div>

---

The [Markdown task templates](prompt-templates.md) allow reusing
prompts with `/prompts:name` in the TUI, line mode, and `huginn run`.

## Shell command isolation

On macOS, every `run_shell` command runs through `/usr/bin/sandbox-exec` with a
deny-by-default Seatbelt profile. The profile allows access to the selected
workspace and required shell runtime files. It denies network and socket
access. `Deny` prevents the operation; `Allow once` authorizes only that
operation and does not remove the sandbox. Huginn has no shell sandbox opt-out
or persistent permission grant.

The sandbox applies only to `run_shell` and its child processes. Native tools
and MCP servers are outside this isolation boundary and retain their separate
policy controls. Linux and Windows do not receive equivalent isolation from
this feature. Apple deprecates `sandbox-exec` and Seatbelt interfaces. They can
change or become unavailable, so Huginn fails the shell operation if it cannot
apply the profile. Shell commands that need network access or paths outside the
workspace can fail by design. This mechanism is not an Apple support guarantee.

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

| Principle             | Description                                       |
|-----------------------|---------------------------------------------------|
| 🔒 **Privacy-first**  | Code stays local, only model requests go to cloud |
| 💻 **Native tools**   | File ops, terminal, git — all built-in            |
| 🔌 **Extensible**     | MCP stdio client for custom tools                 |
| 📝 **Scoped context** | `AGENTS.md` controls what Huginn sees             |
| 💾 **Session memory** | SQLite-backed conversation history                |

---

## ✨ Features

| Feature                  | Description                                         |
|--------------------------|-----------------------------------------------------|
| 🖥️ **Ratatui TUI**       | Beautiful terminal interface with colors and layout |
| 🔧 **Native tools**      | File read/write, terminal execution, git operations |
| 🔌 **MCP integration**   | Connect to any MCP stdio server                     |
| 💾 **SQLite sessions**   | Persistent conversation history                     |
| 📋 **Scoped context**    | Per-repo `AGENTS.md` for agent instructions         |
| 🎯 **Tool calling**      | Automatic tool selection based on task              |
| 📤 **JSON mode**         | Machine-readable output for automation              |
| 🌐 **Provider agnostic** | OpenAI-compatible API support                       |

---

## 🏗️ Architecture

Huginn is composed from separate Rust crates. `apps/huginn` is the composition
root; `application` defines use cases and ports; `runtime` and `presentation`
provide concrete capabilities; `core` owns shared contracts and the plugin
kernel. The plugin API supports trusted, statically linked extensions.

See [Huginn architecture](architecture.md) for the crate boundaries, lifecycle,
and component details.

An opt-in [WASM Rust outline experiment](wasm-outline.md) loads one independently
built component in headless mode. It does not change the supported native
authoring path or accept WASM as a production runtime.

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

| Requirement | Version | Notes                 |
|-------------|---------|-----------------------|
| Rust        | 1.89+   | MSRV may be higher    |
| Cargo       | latest  | Included with Rust    |
| SQLite      | system  | Usually pre-installed |

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

### Standalone provider login

Run `huginn auth login` directly in an interactive terminal to configure a provider
without starting a conversation. The command uses the same searchable runtime
catalog, masked fields, and secure connection storage as TUI `/login`.

```bash
huginn --config /path/to/huginn.toml --db /path/to/sessions.db auth login
```

Global `--config`, `--db`, and `--workspace` options retain their existing meaning.
Connection metadata uses the selected database's `providers.db` extension; the
command does not create the conversation database. An invalid explicit config
fails rather than falling back to another config. A concurrent connection writer
or unavailable secure store remains an error; no automatic storage fallback is
introduced. API-key forms also offer the existing explicit ephemeral mode, whose
credentials are lost when this command exits. When the selected provider already
has a connection, the command opens the connection form directly and never the
connection management screen; the new connection is stored separately and does
not overwrite the existing one.

A successful save prints **Connection configured locally** and exits with status

0. This confirms local storage, not remote authorization or model availability.
   Select a model with `/model` on the next TUI startup; the command does not change
   an already running session. Escape returns to the previous form and cancels at
   the catalog. Ctrl+C cancels. Cancellation and failure both return nonzero, with
   different messages; neither reports success. If a write was pending when the
   command was cancelled, inspect connections on the next startup to reconcile its
   status. Cancellation does not revoke remote credentials.

Piped input, redirected output, `--json`, and `--line-mode` are rejected. There
is no API-key argument or stdin secret-entry mode. OAuth and device-code login
remain unsupported. Secrets must only be entered in the host-owned masked form.

### Agent Skills catalog

`huginn skills list` shows global and project metadata without configuring
a model, opening SQLite, or starting MCP. `huginn --json skills list` returns
entries, diagnostics, and a SHA-256 fingerprint. See the
[catalog contract](skills.md) for paths, limits, and link policy.
The listing does not load bodies into the model. To invoke a skill, use
`/skill:<name> [task]` in the TUI, line mode, or `huginn run`.
The TUI offers names and descriptions in autocomplete. Loading preserves
untrusted provenance and does not grant additional permissions.

### Command Reference

| Command                        | Description                    |
|--------------------------------|--------------------------------|
| `huginn chat`                  | Interactive TUI chat (default) |
| `huginn run "<prompt>"`        | Single command, exit           |
| `huginn sessions`              | List saved sessions            |
| `huginn resume <uuid>`         | Continue a session             |
| `huginn --json run "<prompt>"` | JSON output (for scripting)    |

### Environment Variables

| Variable          | Required                    | Description                        |
|-------------------|-----------------------------|------------------------------------|
| `HUGINN_BASE_URL` | For non-mock providers      | API endpoint (include `/v1`)       |
| `HUGINN_MODEL`    | For non-mock providers      | Model name                         |
| `HUGINN_API_KEY`  | For auth-required providers | API key (optional for mock/Ollama) |

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

| Command      | Description                    |
|--------------|--------------------------------|
| `/login`     | Configure provider credentials |
| `/providers` | Switch between providers       |
| `/model`     | Change the active model        |
| `/logout`    | Clear stored credentials       |
| `/exinfo`    | System information             |
| `/clear`     | Clear current conversation     |
| `/quit`      | Exit Huginn                    |

### `/settings` modal

Open with `/settings` (chat mode only — `--line-mode` excludes the modal).
The dialog hosts five tabs in the order `theme · indicators · sound · toasts · integrations`.
Use `Tab`/`BackTab` to switch tabs and `↑/↓` to move the cursor.
Space changes the selected option; moving the cursor does not change its value.
In `toasts`, `←/→` also changes the value. In the other tabs,
`←/→` switches sections. Enter saves the working copy to `[ui]` in
`config.toml` using `toml_edit` and shows `Settings saved`.
Esc discards the copy without modifying the file and shows `Settings discarded`. v1 persists the values only — none of the tabs is
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

On narrow terminals, the header shows the active tab and its position.
The `/help` view supports scrolling with `↑/↓` and `PgUp`/`PgDn`.

### Keyboard Shortcuts

| Shortcut                                                      | Action                 |
|---------------------------------------------------------------|------------------------|
| `Ctrl+C`                                                      | Cancel current request |
| `Ctrl+L`                                                      | Clear screen           |
| `Enter` / `Ctrl+Enter`                                        | Send message           |
| `Shift+Enter` (compatible terminals) / `Alt+Enter` / `Ctrl+J` | Insert a newline       |
| `↑/↓`                                                         | History navigation     |

The composer grows with the draft up to a limit based on the terminal's
height. If the text takes more rows, the composer shifts the earlier rows
and keeps the cursor visible. `Shift+Enter` requires a terminal compatible with
the extended keyboard protocol; `Alt+Enter` and `Ctrl+J` are alternatives.
Pastes of at least three lines or more than 150 characters are shown as
`[Pasted ~N lines]`; when sending a normal prompt, Huginn expands the markers
and applies `trim()`, which removes leading and trailing whitespace. `/skill:`
invocations keep their existing handling. The paste's internal content
is preserved, and the marker is not sent. A paste exceeding the input
limit of 64 KiB is discarded.

---

## 🔧 Configuration

### Configuration file

See [config.example.toml](../../apps/huginn/config.example.toml) for every
setting. The default location is `~/.cortex/huginn/config.toml`: `~/.cortex` is
the shared Cortex home, and each product keeps its own directory inside it.
The path resolves from the operating system home directory, so it is the same
on macOS, Linux and Windows. If no file exists, Huginn runs with defaults and
never creates one. CLI options override the file; environment variables provide
defaults for the endpoint and model. API keys are loaded from a named
environment variable. Set `api_key_env = ""` for a local endpoint
without authentication.

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

Huginn discovers instructions along the **project chain**: from the workspace up to
the project root (the nearest ancestor with a `.git` marker), it loads the
`AGENTS.md` of every directory, from general to specific. Subtrees outside that
chain are never visited, so a large repository, a vendored copy or a directory
full of third-party files cannot widen the trusted context. If the workspace has no
project marker, only its own `AGENTS.md` is loaded.

An optional `.agents/rules/` directory under the workspace adds extra rule files:
Markdown only, two levels deep, and only when the directory exists. Its existence
is the configuration; nothing is scanned when it is absent.

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

For deterministic acceptance tests, the mock also follows a `huginn-mock:` line in
its latest user or tool message, the way an instruction-following model would:
`huginn-mock: <tool> <json-arguments>` requests that tool, and
`huginn-mock: load_skill <name>` requests an advertised skill by name. Those
requests pass through HostPolicy and normal approvals like any other model
request; the directive never grants permissions.

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

| Server                                    | Version | Purpose           |
|-------------------------------------------|---------|-------------------|
| `@modelcontextprotocol/server-filesystem` | 0.6.3   | File operations   |
| `@modelcontextprotocol/server-git`        | 0.5.0   | Git operations    |
| `@modelcontextprotocol/server-brave`      | latest  | Web search        |
| `@modelcontextprotocol/server-slack`      | latest  | Slack integration |

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

For repository setup, mock-provider workflows, the architecture guard, and focused Huginn tests,
see the [Huginn development guide](../development/huginn.md).

---

## 🔒 Security

| Concern         | Mitigation                                                  |
|-----------------|-------------------------------------------------------------|
| API keys        | Use environment variables, not config files                 |
| File access     | Workspace-scoped — Huginn only accesses specified directory |
| Shell execution | Terminal commands require explicit approval                 |
| MCP tools       | MCP calls require explicit approval                         |
| Session data    | SQLite DB stored locally (no built-in encryption)           |

### Best Practices

1. **Never commit API keys** — use environment variables
2. **Review tool calls** — Huginn shows what it will execute
3. **Scope workspaces** — Use `--workspace` to limit access
4. **Audit sessions** — Check `huginn sessions` for history

---

## 📚 Resources

| Resource                        | Description                   |
|---------------------------------|-------------------------------|
| [Architecture](adr/README.md)   | Design decisions and ADRs     |
| [Providers UI](providers-ui.md) | Provider configuration in TUI |
| [Rook](../rook/README.md)       | Companion AI proxy            |
| [Cortex](../README.md)          | Full project documentation    |

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

## Core provider plugins

`cargo run -p huginn -- chat` includes three independent core plugins. `/login`
shows a single entry per provider:

- **Local Ollama** (`ollama`): initial endpoint `http://127.0.0.1:11434`, with no key
  or secret-store selection. It explicitly authorizes local HTTP. The
  exception only allows literal loopback addresses, not `localhost` or
  remote hosts.
- **Ollama Cloud** (`ollama-cloud`): requires an API key and uses the fixed
  official endpoint `https://ollama.com`. It does not require a local daemon. The form does not
  allow sending the Cloud key to another host.
- **Custom — OpenAI-compatible** (`openai`): enter the service's base URL (include `/v1` if the service requires it). Choose authentication within the flow;
  the API key is optional. It does not implement the Anthropic protocol. It keeps the
  `openai` ID to reuse saved connections.

Leave `Model ID` empty to query the catalog and choose a model before
persisting the connection. If you enter an ID, it is checked against the catalog when
available. If creation fails, the form remains open with the specific reason
and retains the public values; the submitted secret is discarded, and it must be
re-entered. An empty catalog or a connection/authentication error does not save an
incomplete connection. If Custom does not implement the catalog (`/models` returns 404 or 405), enter the ID manually: the service will check
that model when it receives the inference request. Other failures do not trigger this
fallback. After saving, use `/model` to select the connection for chat.
No models are downloaded, and Ollama is not started automatically.

The Ollama catalog and capabilities use `/api/tags` and `/api/show`;
inference uses `/v1`. Previous local connections with a key remain
compatible, but new local creations do not request a key. Each plugin owns
its own registration and can be retired without retiring the other providers. To
add a core provider, register another `ProviderAdapter` from a plugin with its
own identity. The selector uses the catalog; it does not need a branch per provider.
