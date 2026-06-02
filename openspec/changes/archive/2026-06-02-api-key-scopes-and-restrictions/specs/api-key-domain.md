# Spec Delta: API Key Domain

This delta updates `openspec/specs/api-key-domain/spec.md` to match the
implemented system after #46. The original REQ-DOM-1 through REQ-DOM-8 remain
valid; REQ-DOM-2 is rewritten in place because the scope allowlist is no longer
the binary `read`/`write` pair. REQ-DOM-9, REQ-DOM-10, and REQ-DOM-11 are
new requirements added by this change. **REQ-DOM-3 (Tier Representation) is
UNCHANGED** — `ApiKeyTier` is still in the domain and used by the
`/api/api-keys` REST surface; the proposal considered removing it but the
code does not.

---

## MODIFIED Requirements

### REQ-DOM-2: Scope Allowlist (REPLACES the old REQ-DOM-2)

The system SHALL support exactly five canonical scope values, defined by the
`KnownScope` enum in `crates/domain/rook-core/src/api_key.rs:29`:

| Variant             | Wire string       | Purpose                                                    |
|---------------------|-------------------|------------------------------------------------------------|
| `ChatRead`          | `chat:read`       | Read-only access to chat operations (e.g. listing models)  |
| `ChatWrite`         | `chat:write`      | Write access to chat operations (e.g. POST chat completions) |
| `ProvidersRead`     | `providers:read`  | Read-only access to provider configuration                 |
| `ProvidersWrite`    | `providers:write` | Write access to provider configuration                     |
| `Admin`             | `admin`           | Full administrative access (including API key management)   |

The `KnownScope::as_str` method SHALL return the wire string for each variant,
and `FromStr for KnownScope` SHALL accept the wire string and reject any other
input (case-sensitive, lowercase only).

The `ApiKeyScope::parse(&str)` function SHALL:

- Reject empty or whitespace-only input with `ApiKeyValidationError::EmptyScope`.
- Reject any string not in the canonical set with
  `ApiKeyValidationError::UnknownScope(String)`.
- Return `Ok(ApiKeyScope)` for any of the five canonical values.

The `ApiKeyScope::parse_lenient(&str)` function SHALL accept any non-empty
string (after trimming) and return an `ApiKeyScope` without erroring. Unknown
scope strings SHALL be logged at `WARN` level with the field `scope=<value>`
via the `tracing` crate. This is the **only** path that does not error, and
it is used exclusively when hydrating rows from the `scopes_json` column so
that legacy pre-#46 records remain readable.

(Previously: the allowlist was the binary pair `read`/`write`.)

### REQ-DOM-9: Model Allowlist (NEW)

The system SHALL represent an API key's model allowlist as
`allowed_models: Vec<ModelId>` on both `ApiKeyRecord` and `ApiKeySubject`.
The semantic is **empty vec = unrestricted, non-empty vec = strict
allowlist**:

- An empty `allowed_models` SHALL be interpreted as "the key may invoke any
  model". The request-time enforcement layer (`route_request.rs:59`) MUST
  permit the request without inspecting the requested model.
- A non-empty `allowed_models` SHALL be interpreted as a strict allowlist. The
  request MUST be rejected if `requested_model` is not in the list. See
  `route_request.rs:59–66` and the corresponding use-case spec for the 403
  envelope.

The invariant MUST be encoded by the type system (an empty `Vec<ModelId>`
is the only valid "unrestricted" state). The use case MUST NOT introduce a
separate `Option<Vec<ModelId>>` field on the domain type.

### REQ-DOM-10: Provider Allowlist (NEW)

The system SHALL represent an API key's provider allowlist as
`allowed_providers: Vec<ProviderId>` on both `ApiKeyRecord` and
`ApiKeySubject`. The semantic mirrors REQ-DOM-9:

- Empty `allowed_providers` SHALL mean "any provider".
- Non-empty `allowed_providers` SHALL be a strict allowlist enforced by
  `route_request.rs:81` after `FallbackRouter::select()` returns. See the
  use-case spec for the 403 envelope and the note on the cost trade-off of
  checking **after** selection.

### REQ-DOM-11: Restriction Semantics in Auth Subject (NEW)

The runtime auth principal `ApiKeySubject` (returned by
`ApiKeyRepositoryPort::find_active_by_hash`) SHALL expose the same
`allowed_models: Vec<ModelId>` and `allowed_providers: Vec<ProviderId>` fields
as the persisted `ApiKeyRecord`. This guarantees the authz middleware
(`authz.rs:579–597`) and the use case (`route_request.rs:54`) see the same
shape without an additional mapping step.

The transport-layer `Subject` struct in `authz.rs:264` carries these fields
as `Vec<String>` for header propagation (`x-authz-allowed-models`,
`x-authz-allowed-providers`) so downstream middleware can introspect
restrictions without re-querying the database.

---

## REMOVED Requirements

(none)

---

## Scenarios

### Scenario: ApiKeyScope::parse accepts the 5 known values

- GIVEN the strings `"chat:read"`, `"chat:write"`, `"providers:read"`,
  `"providers:write"`, and `"admin"`
- WHEN `ApiKeyScope::parse` is called on each
- THEN each call returns `Ok(ApiKeyScope(_))` whose `as_str()` round-trips to
  the input string

### Scenario: ApiKeyScope::parse rejects legacy "read" and "write"

- GIVEN the string `"read"` (the pre-#46 binary scope)
- WHEN `ApiKeyScope::parse` is called
- THEN it returns `Err(ApiKeyValidationError::UnknownScope("read"))`

(And the same for `"write"`, `"Chat:Read"`, `"ADMIN"`, `""`, and `"   "`.)

### Scenario: parse_lenient accepts an unknown scope without erroring

- GIVEN a row in `api_keys.scopes_json` containing the legacy string `"read"`
- WHEN the repository hydrates an `ApiKeySubject` via `scopes_from_json`
- THEN the subject is returned with `scopes[0].as_str() == "read"`
- AND a tracing WARN line is emitted with the unknown value

(Implementation: `auth-sqlite/src/lib.rs:428` calls `ApiKeyScope::parse_lenient`
inside `scopes_from_json`. This is the only path that does not error.)

### Scenario: Empty allowed_models means unrestricted

- GIVEN an `ApiKey` with `allowed_models = vec![]`
- WHEN the auth middleware extracts the subject and stamps
  `x-authz-allowed-models`
- THEN the header value is the empty string
- AND a request to any model is permitted by the model-restriction check in
  `route_request.rs:59`

### Scenario: Empty allowed_providers means unrestricted

- GIVEN an `ApiKey` with `allowed_providers = vec![]`
- WHEN the request is routed
- THEN the provider-restriction check in `route_request.rs:81` is a no-op and
  any provider selected by `FallbackRouter::select()` is accepted

### Scenario: KnownScope preserves case sensitivity

- GIVEN the string `"CHAT:READ"` (uppercase)
- WHEN `ApiKeyScope::parse` is called
- THEN it returns `Err(ApiKeyValidationError::UnknownScope("CHAT:READ"))`

(Verified by `api_key.rs:212`: uppercase variants are unknown. The wire
contract is lowercase-only; clients sending uppercase MUST be rejected.)
