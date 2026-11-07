# Rook Backend Development

This guide covers the Rook HTTP backend. Run commands from the repository root unless a step says otherwise. For shared repository setup and complete CI validation, start with the [Development Guide](../development.md).

## First-time setup

Use the repository setup recipe:

```bash
just setup
just doctor
```

`just setup` installs dashboard dependencies, checks the Rust toolchain, creates `~/.cortex/rook/config.toml` only if the file does not exist, and attempts CodeGraph initialization when the CLI is installed. It does not overwrite an existing Rook configuration. `just doctor` is read-only.

Rook reads its configuration from `ROOK_CONFIG` when set. Otherwise it reads `~/.cortex/rook/config.toml`. The default configuration created by `just setup` binds to `127.0.0.1:3773`.

For all configuration options, see the [Rook configuration guide](../rook/configuration.md).

## Run Rook locally

Start the backend:

```bash
just run
```

`just run` runs `cargo run -p rook`. Stop the server with `Ctrl+C`.

With the default generated configuration, check the health endpoint:

```bash
curl http://127.0.0.1:3773/health
```

If the active configuration uses another host or port, use that address instead. To enable debug logs:

```bash
RUST_LOG=debug cargo run -p rook
```

## Manual smoke check

The `/health` request above checks that the server starts and responds. It does not verify management authentication, provider credentials, or model availability. Configure a provider before testing provider-backed routes; see the [Rook provider guide](../rook/providers.md). For an isolated Docker smoke test, follow the [Docker smoke-test guide](../../dev/README.md#path-1--smoke-test-container).

## Focused Rust checks

Run the check that matches the code you changed:

```bash
# Example domain package
cargo test -p rook-core

# Full Rust workspace
cargo test --workspace --all-features

# Formatting and linting
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

The complete local pull-request validation is `just ci`. See the [Development Guide](../development.md#pre-pr-validation) for its prerequisites and coverage.

## Related documentation

- [Rook API reference](../rook/api.md)
- [Rook configuration](../rook/configuration.md)
- [Rook architecture](../rook/architecture.md)
- [Docker and E2E testing](../../dev/README.md)
