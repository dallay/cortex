<div align="center">

# 🤖 Huginn

Local Rust coding agent

[![Rust MSRV](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Fdallay%2Fcortex%2Fmain%2Fapps%2Fhuginn%2FCargo.toml&query=%24.package%5B%27rust-version%27%5D&label=Rust%20MSRV&suffix=%2B&color=DEA584&logo=rust)](https://www.rust-lang.org)
[![SQLite](https://img.shields.io/badge/sqlite-3-003B57?logo=sqlite)](https://www.sqlite.org)
[![MCP](https://img.shields.io/badge/MCP-ready-7C3AED?logo=modelcontextprotocol)](https://modelcontextprotocol.io)
[![License](https://img.shields.io/github/license/dallay/cortex)](../../LICENSE)

**A local-first coding assistant for working with your codebase.**

[Overview](#overview) · [Quick Start](#quick-start) · [Capabilities](#capabilities) · [Security](#security) · [Documentation](#documentation)

</div>

---

## Overview

Huginn is a Rust coding agent that runs on your machine. It can work with a selected workspace through native tools, keep conversation history in SQLite, and connect to model providers and optional MCP servers.

Your workspace and local tool execution stay on your machine. Prompts, selected context, and model requests are sent to the provider you configure. Review that provider's data-handling terms before use.

## Quick Start

### Requirements

- Rust 1.89 or later, as declared by `apps/huginn/Cargo.toml`.
- A configured model provider. Provider credentials are optional only for providers that do not require authentication.

### Build

Run these commands from the repository root:

```bash
cargo build --release -p huginn
```

### Configure and start a chat

Configure provider credentials in the interactive, masked login form, then start Huginn with the project you want to work in:

```bash
cargo run --release -p huginn -- auth login
cargo run --release -p huginn -- --workspace /path/to/project chat
```

On the next TUI startup, select a model with `/model`. The login form stores credentials locally; it does not confirm remote authorization or model availability.

See the [configuration example](config.example.toml) and the [complete usage guide](../../docs/huginn/README.md) for provider setup, environment variables, authentication, and configuration precedence. Keep credentials out of source control and logs.

Run one prompt without opening an interactive chat:

```bash
cargo run --release -p huginn -- --workspace /path/to/project run "Explain this codebase"
```

Non-interactive actions are denied unless you explicitly grant them for that invocation with `--allow`. Review the [permissions and guardrails](../../docs/huginn/guardrails.md) before granting access.

## Capabilities

- **Terminal interface:** Ratatui chat, line mode, and JSON output for automation.
- **Workspace tools:** Native file, shell, and Git operations governed by Huginn's host policy and approval flow.
- **Persistent sessions:** Conversation history stored locally in SQLite.
- **Scoped instructions:** `AGENTS.md` files provide project context along the workspace's project chain.
- **MCP integrations:** Connect to configured stdio servers; MCP capabilities use their own permission controls.

## Security

Huginn's tools can affect files and run commands. Review each requested action and grant only the permissions needed for the task.

On macOS, `run_shell` commands use a deny-by-default Seatbelt profile. This shell isolation is not available on Linux or Windows through this feature, and it does not cover native tools or MCP servers. Apple deprecates the underlying `sandbox-exec` and Seatbelt interfaces, so the shell operation fails if Huginn cannot apply the profile. See [shell command isolation](../../docs/huginn/README.md#shell-command-isolation) for the exact boundary and limitations.

## Documentation

- [Complete Huginn guide](../../docs/huginn/README.md) — installation, usage, TUI, and configuration.
- [Architecture](../../docs/huginn/architecture.md) — crate boundaries and runtime lifecycle.
- [Permissions and guardrails](../../docs/huginn/guardrails.md) — action authorization and tool policies.
- [Interactive architecture diagram](../../docs/architecture/huginn-architecture.html).
- [Cortex overview](../../README.md) — the full product suite and development workflow.

## Development

```bash
cargo test -p huginn
cargo clippy -p huginn --all-targets -- -D warnings
```

See the [Huginn architecture guide](../../docs/huginn/architecture.md) for crate ownership and the [development guide](../../docs/development.md) for repository-wide workflows.
