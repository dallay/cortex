# Secure headless connections

P0.2 builds on the [P0.1 router](providers.md) and
[ADR-0014](adr/0014-builtin-providers-and-turn-scoped-model-routing.md).
It adds Rust headless use cases, not terminal forms, OAuth, new HTTP adapters,
or a replacement for the executable's legacy configuration path. P0.3/P0.4
consume these APIs; P0.5 composes them into the product workflow.

## Composition

1. Explicitly construct `OsSecrets` or `EphemeralSecrets`; there is no backend
   fallback. Keep its `Arc<dyn SecretStore>` for the connection manager lifetime.
2. Construct `ConnectionAccess` with that store, then activate
   `ProvidersPlugin::with_credentials(access.clone())` and native adapters.
3. Open `SqliteConnections` at a dedicated path, such as `connections.db`.
   Do not use `sessions.db`: the schemas and ownership are independent.
4. Open `ConnectionManager` with the metadata store, access handle,
   `ProviderManagement` and `ProviderCatalog` from that activation.

`create`, `list`, `status`, `cleanup_status`, `select`, `update`, `revoke`, `reconcile` and
`resolve` are terminal-independent async use cases. Connection IDs are generated
UUIDs. Metadata contains provider, endpoint, model, revision, persistence mode
and an opaque UUID secret reference. Selection contains only a connection ID.
The adapter receives `ConnectionMaterial` through `create_connection_model`;
its default implementation rejects all connection material, including no-auth
endpoint/settings. Adapters must explicitly implement this connection-aware factory
to honor routing configuration; the legacy factory is never an implicit fallback.

Credentials are nonserializable `Secret` values with redacted `Debug` and a
zeroed owned buffer on drop. Only trusted stores/adapters should call `expose`.
They must not put that value into session messages, errors, telemetry or events.
Native adapters remain trusted code, not a sandbox.

## Secure backend requirements

- **macOS:** the native user Keychain, explicitly constructed through
  `keyring::macos::MacCredential`, not a global/default backend.
- **Linux:** Secret Service through session D-Bus, explicitly constructed through
  `keyring::secret_service::SsCredential`. Enable a compatible service such as
  GNOME Keyring or KWallet's Secret Service interface, and unlock its collection.
  Building uses `pkg-config` and libdbus development headers/libraries.
- Other platforms return `BackendUnavailable`; they never write plaintext.

The pinned `keyring` 3.6.3 API supports the workspace's Rust baseline. Features
are target-specific: `apple-native` on macOS and `sync-secret-service` plus
`crypto-rust` on Linux. The kernel-keyutils and mock backends are not selected.
OS calls run on blocking workers, serialized across instances before submission.
A dropped async waiter cannot let cleanup overtake its credential write.

Errors distinguish missing entries, inaccessible/locked storage and unavailable
services. Unlock or enable the native service and retry; errors deliberately do
not expose vendor error strings. There is no automatic ephemeral downgrade.

`EphemeralSecrets::default()` is an explicit choice of in-memory storage. Its
contents are lost when the store is dropped. Metadata can survive; a recovered
connection with a missing ephemeral secret remains listed but is not connected
to the router or ready for inference. `status` explains the missing credential.

## Coherent input precedence

`ConnectionSources` resolves atomic configured choices in this order:

1. Explicit CLI choice.
2. Explicit TOML choice, otherwise a chosen environment profile.
3. Persisted selection, resolved by `ConnectionManager`.
4. `Unconfigured`.

Each choice is either an existing connection ID or one complete configuration.
No endpoint, model or credential is completed from a lower tier. A missing
credential or invalid ID in the winning tier fails, even if persistence contains
a working connection. Lower tiers are not evaluated. Provenance reports the
winning tier without credentials.

`ExplicitConnectionConfig::parse_toml` accepts a provider, endpoint, model and
optional `credential_env` name, never a secret field. The chosen environment
map resolves exactly that name; it does not fall back to `AGENT_API_KEY` or any
other connection. URLs with userinfo, query parameters or fragments are rejected
as metadata. Endpoint TLS policy and transport adapters belong to P0.4.

These source adapters are headless API inputs. Existing `huginn` flags and legacy
TOML/environment behavior have not yet been migrated to this API; callers must
not infer new executable commands from these type names.

## Failure and restart semantics

All manager operations and reopening share a serialization lock on `ConnectionAccess`.
Reopening retires the previous manager generation before loading authoritative metadata.
Every subsequent operation on the old manager returns `Revoked` before touching runtime
or metadata state. Old pinned authorizations remain revoked even if the same connection
is reinstalled. Cancellation guards invalidate only their owning generation, under
the shared lock. SQLite stores one metadata document using
atomic versioned compare-and-swap, full synchronous durability and a process
ownership lease. A stale writer fails rather than merging credentials.

Creation persists an orphan-cleanup intent before writing a new secret, then
commits the connection and removes its intent. Rotation revokes the old runtime
identity first, removes the old durable connection/selection and journals both
retired and staged secret references before writing the replacement. It never
overwrites a secret reference. The replacement requires explicit selection;
an existing turn cannot adopt it.

Revocation immediately unbinds the runtime identity and cancels its admitted
requests, then durably removes metadata and selection before deleting the secret.
Failed deletion leaves a cleanup tombstone. `reconcile` and restart retry cleanup
without deleting live or unrelated references. A failed rotation may remove the
old connection rather than pretending rollback restored its authorization.

Startup retains records with missing adapters or inaccessible credentials for `list`
and `status`, and installs independently usable records. `status` reports
`AdapterUnavailable` for an absent adapter. No-auth connections do not depend on the
secret backend's persistence mode. Locked/unavailable cleanup does not abort startup
or block unrelated connections: tombstones remain durable, and `cleanup_status`
returns their opaque references and actionable errors. `reconcile` retries deletion
and returns outstanding cleanup errors; `revoke` likewise reports failed cleanup.
Successful creation/rotation remains usable with unrelated cleanup pending. After
unlocking/enabling the backend, call `reconcile` and recheck `cleanup_status`.
Fatal startup errors (including invalid metadata/selection or uncertain saves) retire
any partially installed runtime authorizations; retry `open` after correcting the cause.

A metadata write error may mean that the write committed. The manager therefore
fails closed, revokes its installed authorizations and requires reopening from
authoritative storage. Dropping a metadata mutation during I/O also blocks further
manager mutations until reopening. Failed cleanup is visible; it is not reported
as successful revocation cleanup. A failure before any durable change can restore
the old metadata on restart, but never substitutes replacement credentials into
the old authorization.

## Verification

```sh
cargo test -p huginn-application -p huginn-runtime
cargo test -p huginn-runtime --test connections os_secret_backend_contract -- --ignored
```

The ordinary suites use secure-store doubles and the real SQLite implementation.
They cover isolated connections, restart, explicit ephemeral loss, locked/missing
credentials, source precedence, staged writes, failed cleanup, and pinned-turn
invalidation. Sentinel credentials are absent from metadata and display/errors;
the credential store is intentionally the only persistence boundary for them.

The ignored OS contract writes two unique sentinel test entries in the Huginn
namespace, tests read/isolation/idempotent deletion and attempts cleanup on
operation failure. Run it only on an accessible compatible native backend.
macOS and Linux runs must be reported separately; doubles are not OS evidence.
The onboarding QA issues remain independent and are not closed by these tests.
