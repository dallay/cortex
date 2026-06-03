# Spec: API Key Dashboard

## Purpose

Defines the Vue 3 + TypeScript dashboard UI in
`apps/rook/dashboard/src/views/ApiKeysView.vue` for managing API keys.
This spec captures the post-#46 state: the 5-scope chip group, the
`allowedModels` and `allowedProviders` inputs, the rotate action with
confirmation and copyable banner, and the restrictions badge in the
list view. The dashboard reuses the existing `useApiKeys` composable
(`apps/rook/dashboard/src/composables/useApiKeys.ts`) and the existing
`@/lib/api` client; the data model types are extended in
`apps/rook/dashboard/src/lib/api.ts`.

This file is a **full spec**, not a delta — the dashboard has not been
specified before in `openspec/specs/api-key-dashboard/`. The previous
archive spec (`openspec/changes/archive/2026-05-31-api-key-crud/specs/api-key-dashboard.md`)
describes the pre-#46 layout and is superseded by this one.

---

## Components

### Create Modal (in `ApiKeysView.vue`)

Fields:

- **Label** — text input. Required, non-empty after trim. Pre-populated
  with placeholder text (`e.g., opencode-agent`).
- **Scopes** — chip group with the 5 canonical options:
  `chat:read`, `chat:write`, `providers:read`, `providers:write`,
  `admin`. At least one chip MUST be selected before submit. The
  pre-#46 list at `ApiKeysView.vue:172–175` had only 2 entries; this
  spec replaces that array with the full 5-entry list, rendered as a
  chip group (not raw checkboxes) for visual clarity with 5 options.
- **Tier** — `<Select>` with 3 options: `free`, `pro`, `enterprise`.
  Default `free`.
- **Expires at** — date picker (optional). Must be in the future if set;
  the transport returns 400 on past values.
- **Allowed models** — text input accepting comma- or space-separated
  model IDs. On submit, the value is split on `,` and whitespace,
  trimmed, and empty entries are dropped. Empty input = unrestricted.
  Wire format: `allowedModels: string[]`.
- **Allowed providers** — multi-select chip group populated from
  `GET /v1/providers` via the existing `useProviders()` composable
  (`apps/rook/dashboard/src/composables/useProviders.ts`). Only
  currently-registered provider IDs are selectable, so unknown IDs
  cannot enter the field at all — the UI layer enforces the same
  constraint that REQ-UC-12 will enforce at the use case. Empty
  selection = unrestricted. Wire format: `allowedProviders: string[]`.

Submit: calls `POST /api/api-keys` with the assembled request body. On
success, the response's `plaintextKey` is rendered in an amber
copy-banner (existing UX at `ApiKeysView.vue:335–351`). The banner
displays the raw key in monospace with a Copy button; the modal stays
open with a "Done" button until the user dismisses.

### Edit Modal (in `ApiKeysView.vue`)

Same fields as the Create modal, pre-populated from the existing
record. Submit calls `PUT /api/api-keys/{id}`. After save, the modal
closes and the list refreshes the affected row in place (the
`useApiKeys.update` composable method already updates the local store
entry).

Note: rotating a key is **not** part of edit. Rotating changes the
raw key and is a separate action with a different confirmation flow
(see Rotate Action below).

### List Display (in `ApiKeysView.vue`)

One row per key. Columns:

- **Name** — `key.label`, prefixed with a key icon.
- **Key** — `keyPrefix` masked as `${prefix}...`. The raw key is
  **never** shown for an existing key (only for newly created or
  rotated keys, in the amber banner).
- **Scopes** — one chip per scope, rendered from `key.scopes` array.
- **Tier** — capitalized string (`free` / `pro` / `enterprise`).
- **Status** — green "Active" pill when `isActive = true`; red
  "Revoked" pill otherwise.
- **Created** — formatted date from `key.createdAt` (or `—` if null).
- **Last Used** — formatted date from `key.lastUsedAt` (or `—`).
- **Actions** — Edit (pencil icon), Revoke (trash icon, destructive
  variant). The pre-#46 layout has only these two; this spec adds
  **Rotate** as a third action button (see below).

Pagination controls (`Previous` / `Next`) live below the table and use
the existing `useApiKeys().prevPage` / `nextPage` methods.

### Rotate Action (NEW — `POST /api/api-keys/{id}/rotate`)

A new icon button between Edit and Revoke. On click:

1. Opens a confirmation dialog with the title "Rotate API Key" and a
   body explaining: "This will immediately invalidate the current key
   and generate a new one. The new key will be shown once. This
   action cannot be undone."
2. On confirm, calls `useApiKeys().rotate(id)` (new composable method
   described below).
3. On success, the dialog closes and the new raw key is shown in the
   **same** amber copy-banner used by Create. The list row's
   `keyPrefix` is updated in place to the new prefix returned in the
   response.
4. On failure, an inline error message is shown in the dialog (the
   banner is not displayed).

### List Display — Restrictions Badge (NEW)

Each row SHALL also render a `Restrictions` indicator. The rendering
rule (computed from `allowedModels` and `allowedProviders`):

- Both empty → gray `Unrestricted` badge.
- Only `allowedModels` non-empty → amber `Restricted (N models)`
  badge, where N is `allowedModels.length`.
- Only `allowedProviders` non-empty → amber
  `Restricted (N providers)` badge.
- Both non-empty → amber `Restricted (N models, M providers)` badge.

The dashboard MAY render this as a separate column or as a
"Restrictions" sub-line under the Name column. The spec does not
constrain the exact placement; the values it MUST display are
specified by the rendering rule above.

---

## State Management

### Existing pieces (unchanged)

- `apiKeys: Ref<ApiKeyRecordResponse[]>` — current page of keys.
- `pagination: Ref<PaginationState>` — `{ total, limit, offset }`.
- `loading: Ref<boolean>`, `error: Ref<string | null>`.
- `fetch(limit?, offset?)`, `create(req)`, `update(id, req)`,
  `revoke(id)`, `nextPage()`, `prevPage()` — all already in
  `useApiKeys.ts`.

### New `rotate` method (added to `useApiKeys.ts`)

```typescript
async function rotate(id: string): Promise<{ key: ApiKeyRecordResponse; plaintextKey: string } | null>
```

Behavior:

1. Call `api.rotateApiKey(id)` (the new client method, see below).
2. On success, update the local `apiKeys` entry whose `id` matches
   `id` with the new record (new `keyPrefix`, same other fields). The
   caller (the rotate dialog) is responsible for displaying
   `plaintextKey` in the banner.
3. On error, set `error.value` and return `null`.

The composable's return type extends to expose `rotate`.

### New `rotateApiKey` method (added to `@/lib/api.ts`)

```typescript
async rotateApiKey(id: string): Promise<CreateApiKeyResponse> {
  return request<CreateApiKeyResponse>(`/api/api-keys/${id}/rotate`, {
    method: 'POST',
  })
}
```

Reuses the existing `CreateApiKeyResponse` type, which is
`{ key: ApiKeyRecordResponse, plaintextKey: string }` — the rotate
handler returns the same shape as create.

### Type extensions in `@/lib/api.ts`

`ApiKeyRecordResponse` and `CreateApiKeyRequest` / `UpdateApiKeyRequest`
SHALL be extended with the new fields (camelCase wire format):

```typescript
interface ApiKeyRecordResponse {
  // ... existing fields
  allowedModels: string[]     // NEW — empty array means unrestricted
  allowedProviders: string[]  // NEW — empty array means unrestricted
}

interface CreateApiKeyRequest {
  // ... existing fields
  allowedModels?: string[]    // NEW — defaults to [] in DTO
  allowedProviders?: string[] // NEW — defaults to [] in DTO
}

interface UpdateApiKeyRequest {
  // ... existing fields
  allowedModels?: string[]    // NEW — Some([]) clears; absent preserves
  allowedProviders?: string[] // NEW — same semantics
}
```

The dashboard's `useApiKeys` composable carries the same shapes
through.

---

## Requirements

### REQ-DASH-1: 5-Scope Chip Group in Create and Edit

The Create and Edit modals SHALL render the scopes selector as a chip
group with all 5 canonical values: `chat:read`, `chat:write`,
`providers:read`, `providers:write`, `admin`. At least one chip MUST
be selected before submit. The pre-#46 array of 2 entries
(`ApiKeysView.vue:172–175`) SHALL be replaced.

### REQ-DASH-2: Free-Form Allowed Models Input

The Create and Edit modals SHALL provide a text input for `allowedModels`
that accepts a comma- or space-separated list of model IDs. On submit,
the value SHALL be split on `,` and whitespace, each entry trimmed,
and empty entries discarded. An empty input value SHALL be sent as
`allowedModels: []` in the request body.

### REQ-DASH-3: Allowed Providers from GET /v1/providers

The Create and Edit modals SHALL populate the `allowedProviders` chip
group from the result of `GET /v1/providers` via the existing
`useProviders()` composable. Only provider IDs currently registered in
the provider registry SHALL be selectable. An empty selection SHALL be
sent as `allowedProviders: []`.

If the providers list fails to load, the dashboard SHALL show an
inline error in the modal and the field SHALL be disabled until the
load succeeds.

### REQ-DASH-4: Edit Modal Pre-Populates Restrictions

When the Edit modal opens for a key, the `allowedModels` text input
SHALL be pre-populated with the key's `allowedModels` joined by
commas, and the `allowedProviders` chip group SHALL be pre-populated
with the key's `allowedProviders` (selecting matching chips). Submitting
the form SHALL call `PUT /api/api-keys/{id}` with the assembled
request body.

### REQ-DASH-5: Rotate Action with Confirmation and Banner

Each list row SHALL expose a Rotate action (icon button) that opens a
confirmation dialog. On confirm, the dashboard SHALL call
`useApiKeys().rotate(id)`. On success, the new raw key SHALL be
displayed in the same amber copy-banner used for Create. On error, an
inline message SHALL be shown in the dialog and the banner SHALL NOT
appear.

The confirmation dialog body SHALL include the text: "Rotate this key?
The old key will stop working immediately."

### REQ-DASH-6: List Display Shows Scopes and Restrictions

Each list row SHALL render the key's `scopes` as one chip per scope.
The row SHALL also render a restrictions indicator according to the
rendering rule in the "List Display — Restrictions Badge" section
above.

### REQ-DASH-7: Refresh After Rotate

After a successful rotate, the list row SHALL be updated in place with
the new `keyPrefix` from the response. A full `fetch()` reload is
permitted but not required (the composable's optimistic local update
is the preferred path).

---

## Scenarios

### Scenario: User creates a chat:write-only key restricted to gpt-4 and openai

- GIVEN the user is on the `/api-keys` page
- WHEN they open the Create modal, select only `chat:write`, type
  `gpt-4` in the allowed-models input, select `openai` in the
  allowed-providers chip group, and submit
- THEN `POST /api/api-keys` is called with body
  `{"label": "...", "scopes": ["chat:write"], "tier": "...",
    "allowedModels": ["gpt-4"], "allowedProviders": ["openai"], ...}`
- AND the new raw key is shown in an amber banner with a Copy button
- AND the new row appears in the list with `Scopes: [chat:write]`
  and a `Restricted (1 model, 1 provider)` badge

### Scenario: User rotates an existing key

- GIVEN the user is on the `/api-keys` page and clicks the Rotate icon
  on a row for key K
- WHEN they confirm in the dialog
- THEN `POST /api/api-keys/{K.id}/rotate` is called
- AND the response's `plaintextKey` is shown in the amber banner
- AND the row's `keyPrefix` updates to the first 8 chars of the new
  raw key
- AND the row's other fields (label, scopes, tier, restrictions,
  `isActive = true`) are unchanged

### Scenario: User edits a key to clear all restrictions

- GIVEN a key K with `allowedModels = ["gpt-4"]` and
  `allowedProviders = ["openai"]`
- WHEN the user opens the Edit modal, clears both restriction fields,
  and submits
- THEN `PUT /api/api-keys/{K.id}` is called with
  `{"allowedModels": [], "allowedProviders": [], ...}`
- AND the row updates to show `Unrestricted` badge

### Scenario: User attempts to create a key with no scopes selected

- GIVEN the Create modal is open
- WHEN the user submits with an empty scopes array
- THEN the dashboard shows an inline error: "At least one scope is
  required"
- AND no HTTP request is sent

### Scenario: Backend rejects create with unknown provider

- GIVEN a user typed an unknown provider ID in the allowed-providers
  field (only possible if `useProviders()` failed to load and the
  dashboard permitted free-form input as a fallback)
- WHEN the backend returns `400 VALIDATION_ERROR` with message
  `"unknown provider(s): <id>"`
- THEN the Create modal displays the message in `createError`
- AND no row is added to the list

(Note: under normal operation, REQ-DASH-3 prevents this scenario by
sourcing options from `GET /v1/providers`. The scenario documents the
fallback behavior.)

### Scenario: User rotates a revoked key

- GIVEN a revoked key K with `isActive = false`
- WHEN the user opens the Rotate confirmation dialog and confirms
- THEN the backend returns `409 CONFLICT` with code `KEY_REVOKED`
- AND the dialog shows the error message
- AND the amber banner is NOT shown
- AND no row in the list is mutated
