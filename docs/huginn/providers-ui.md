# Optional Providers UI

> Status: implementation reviewed and tested with doubles. Cancellation of `/model`
> during saving and safe form cancellation during an incomplete paste (Ctrl+C; Esc
> begins the protocol delimiter) were fixed. Remote adapters, the user's Keychain /
> Secret Service, and onboarding acceptance (P0.5 integration) were not validated;
> this is not equivalent to full CI.

`builtin.providers.ui` provides four commands over the headless
`agent:connections@1` service and catalog. It does not provide terminal, focus,
keyboard, or rendering capabilities to adapters.

## Usage

- `/login`: choose a provider and authentication method; enter the endpoint, model,
  and, when applicable, the key in a host-owned secret field.
- `/providers`: inspect connection status, select a model, update configuration or
  credentials, or disconnect.
- `/model`: choose the connection and model for the **next turn**.
- `/logout`: confirm local revocation and credential cleanup.

Tab or the vertical arrows move between fields; the horizontal arrows change
options. Enter submits; Esc cancels the form except during an incomplete paste,
when it is interpreted as part of the paste delimiter. Use Ctrl+C to cancel without
waiting for that delimiter. Ctrl+C inside the form cancels the local action, not
the process. Secret fields use a constant mask and never pass through the
conversation editor or JSON events.

Secure storage does not automatically fall back to ephemeral storage when it
fails. Ephemeral mode requires an explicit choice and loses credentials when the
process exits. After restart, the connection retains its metadata but reports
missing credentials rather than implying availability.

## Contract and limits

For current authentication support, see the
[OAuth support matrix and blocker](oauth.md). `/login` and `/providers` explicitly
report unsupported OAuth, including adapters that declare only OAuth. They do not
offer OAuth choices or token fields; API-key and local forms are unchanged.

`huginn-plugin-api::v1::Workflow` describes fields and an asynchronous operation.
The synchronous callback only returns that description. The host captures public
and secret fields separately in `FormInput` and runs the future outside the input
loop. The original registration is retained throughout the chain; a replacement
contribution is never looked up by name.

The host keeps a single active action/form. Closing, cancelling, or revoking the
contribution discards buffers and cancels the task. This does **not prove**
rollback of a durable write or remote cancellation, billing stoppage, or
revocation. After errors, check the actual state before retrying.

Changing the model does not rotate authorization. A started turn retains its
connection/model; revocation or credential updates block subsequent inference in
that turn. Explicit configuration takes precedence at startup; an interactive
choice is an intentional change for later turns.

Static catalog models are shown as options. Adapters with a connection-scoped
catalog accept a candidate identifier: this UI does not claim to have discovered
or validated remote models. Integration and acceptance with real adapters belongs
to P0.5, not these tests.

## Manual test on macOS

The TUI requires a real terminal. From the PR checkout, in a Terminal or iTerm
window:

```sh
cargo run -p huginn --
```

You should enter the conversation and see a prompt to configure a model. First
try the flow without external credentials:

1. Type `/login` and press Enter. Browse providers and authentication methods;
   cancel with Esc. You do not need to save anything to inspect the form.
2. If you have local Ollama and an already-downloaded model, start Ollama and
   continue `/login` with endpoint `http://127.0.0.1:11434`, explicitly allowing
   HTTP only for loopback. Use the identifier of an installed local model. Do not
   download anything for this test.
3. Alternatively, if you choose to use a test account, enter the key only in the
   masked secret field. Do not put it in CLI arguments, the prompt, the
   configuration file, screenshots, logs, or comments. The UI does not validate
   remote availability here; avoid production keys.
4. In `/providers`, inspect the status and change the model; `/model` should
   indicate that the selection applies to the next turn. Verify that `/logout`
   confirms local revocation without claiming to invalidate the provider key.
5. Try Ctrl+C during a form and paste incomplete text if your terminal supports
   bracketed paste. The form should cancel without displaying the pasted content
   or closing the TUI. Esc during an incomplete paste belongs to the framing; use
   Ctrl+C to cancel.
6. Exit with `/quit`. Restarting lets you inspect persisted status, but not
   ephemeral mode unless you explicitly select it and accept losing the key on
   exit.

The local Ollama test does use a real adapter, but it does not validate the user's
Keychain / Secret Service or P0.5 onboarding. Automated tests use controlled
doubles; PTY captures verify that sentinel secrets, errors, and panics do not
appear in output.

## Automated checks

```sh
cargo test -p huginn-plugin-api
cargo test -p huginn-presentation --features test-driver
cargo test -p huginn-application
just huginn-architecture
```
