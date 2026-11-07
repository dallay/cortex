# Huginn Development

This guide covers local development and focused validation for Huginn. Run commands from the repository root.

## Run Huginn with the mock provider

The mock provider supports a local workflow without a real provider endpoint or API key:

```bash
just huginn-run --provider mock --workspace . run "list files"
```

To start the interactive terminal interface with the mock provider:

```bash
just huginn-run --provider mock --workspace . chat
```

Huginn also supports real providers. Configure the endpoint and model, and provide credentials through the required environment variable. Do not commit credentials. See the [Huginn usage and configuration guide](../huginn/README.md#-configuration) for configuration details.

## Focused validation

Run the product-level validation recipe:

```bash
just huginn-test
```

This runs the Huginn architecture guard before the Huginn Rust test suite. To run one package's tests, use its Cargo package name, for example:

```bash
cargo test -p huginn
```

If `cargo-tarpaulin` is installed, generate package coverage with:

```bash
cargo tarpaulin -p huginn
```

The workspace architecture guard can be run on its own:

```bash
just huginn-architecture
```

## Manual checks

Use the mock-backed TUI command above for local interaction checks that do not depend on a remote model. Verify the affected keyboard flow and relevant success, error, and denied states. For real-provider checks, identify the provider and host prerequisites separately; a mock test does not establish remote compatibility.

## Related documentation

- [Huginn product guide](../huginn/README.md)
- [Huginn architecture](../huginn/architecture.md)
- [Huginn architecture decisions](../huginn/adr/README.md)
- [Repository-wide Development Guide](../development.md)
