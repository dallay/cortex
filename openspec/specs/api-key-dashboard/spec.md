# API Key Dashboard Specification

## Purpose

Defines the Vue 3 + TypeScript dashboard UI in `apps/rook/dashboard/src/views/ApiKeysView.vue` for managing API keys post-#46. Captures the 5-scope chip group, `allowedModels` and `allowedProviders` inputs, the rotate action with confirmation and copyable banner, and the restrictions badge.

This spec replaces the pre-#46 version at `openspec/changes/archive/2026-05-31-api-key-crud/specs/api-key-dashboard.md`.

---

## Components

### Create Modal

Fields:

- **Label** — text input, required, non-empty after trim. Pre-populated with placeholder `e.g., opencode-agent`.
- **Scopes** — chip group with 5 canonical options: `chat:read`, `chat:write`, `providers:read`, `providers:write`, `admin`. At least one chip MUST be selected before submit. Previously had only 2 entries.
- **Tier** — `<Select>` with `free`, `pro`, `enterprise`. Default `free`.
- **Expires at** — date picker, optional. Must be in the future if set.
- **Allowed models** — text input accepting comma- or space-separated model IDs. Empty = unrestricted. Wire format: `allowedModels: string[]`.
- **Allowed providers** — multi-select chip group populated from `GET /v1/providers` via `useProviders()`. Only registered provider IDs are selectable. Empty = unrestricted. Wire format: `allowedProviders: string[]`.

On success: response's `plaintextKey` is rendered in an amber copy-banner with a Copy button. Modal stays open with "Done" until dismissed.

### Edit Modal

Same fields as Create, pre-populated from the existing record. Submit calls `PUT /api/api-keys/{id}`. Rotating a key is NOT part of edit — it's a separate action with its own confirmation flow.

### List Display

Columns: Name | Key | Scopes | Tier | Status | Restrictions | Created | Last Used | Actions

- **Name** — `key.label` with key icon.
- **Key** — `${keyPrefix}...` (raw key never shown for existing keys).
- **Scopes** — one chip per scope from `key.scopes`.
- **Tier** — capitalized string.
- **Status** — green "Active" / red "Revoked" pill.
- **Restrictions** — badge: gray "Unrestricted" | amber "Restricted (N models)" | amber "Restricted (N providers)" | amber "Restricted (N models, M providers)".
- **Created** / **Last Used** — formatted dates or `—`.
- **Actions** — Edit, Rotate (new), Revoke (destructive).

Pagination controls (`Previous` / `Next`) below the table.

### Rotate Action

New icon button between Edit and Revoke. On click:

1. Opens confirmation dialog: "Rotate API Key" — "This will immediately invalidate the current key and generate a new one. The new key will be shown once."
2. On confirm, calls `useApiKeys().rotate(id)`.
3. On success: dialog closes, new raw key shown in amber copy-banner (same UX as Create). List row's `keyPrefix` updated in place.
4. On failure: inline error in dialog, no banner.

---

## State Management

### useApiKeys composable (`useApiKeys.ts`)

Existing methods unchanged except:

```typescript
rotate(id: string): Promise<{ key: ApiKeyRecordResponse; plaintextKey: string } | null>
```

Behavior: calls `api.rotateApiKey(id)`, on success updates local `apiKeys` entry by id with new record, on error sets `error.value` and returns `null`.

### api.ts type extensions

```typescript
interface ApiKeyRecordResponse {
  // ... existing fields
  allowedModels: string[]    // NEW — empty array = unrestricted
  allowedProviders: string[] // NEW — empty array = unrestricted
}

interface CreateApiKeyRequest {
  // ... existing fields
  allowedModels?: string[]
  allowedProviders?: string[]
}

interface UpdateApiKeyRequest {
  // ... existing fields
  allowedModels?: string[]
  allowedProviders?: string[]
}

async rotateApiKey(id: string): Promise<CreateApiKeyResponse> {
  return request<CreateApiKeyResponse>(`/api/api-keys/${id}/rotate`, { method: 'POST' })
}
```

---

## Requirements

### REQ-DASH-1: 5-Scope Chip Group

Create and Edit modals SHALL render the scopes selector as a chip group with all 5 canonical values. At least one chip MUST be selected before submit.

### REQ-DASH-2: Free-Form Allowed Models Input

Create and Edit modals SHALL provide a text input for `allowedModels` accepting comma- or space-separated model IDs. Split on `,` and whitespace, trim, filter empty. Empty = unrestricted.

### REQ-DASH-3: allowedProviders Multi-Select from Registry

Create and Edit modals SHALL provide a multi-select chip group for `allowedProviders` populated from `GET /v1/providers` via `useProviders()`. Only registered provider IDs are selectable. Empty = unrestricted.

### REQ-DASH-4: Edit Pre-Populates Restrictions

Edit modal SHALL pre-populate `allowedModels` and `allowedProviders` from the record being edited.

### REQ-DASH-5: Rotate Action with Confirmation

Rotate button opens confirmation dialog explaining immediate invalidation. On confirm, calls rotate API, shows new raw key in amber banner, updates `keyPrefix` in place.

### REQ-DASH-6: Restrictions Badge

Each list row SHALL render a Restrictions badge: gray "Unrestricted" when both empty; amber "Restricted (N models)" when only models non-empty; amber "Restricted (N providers)" when only providers non-empty; amber "Restricted (N models, M providers)" when both non-empty.

### REQ-DASH-7: Refresh After Rotate

After a successful rotate, the local `apiKeys` entry for the rotated id SHALL be updated with the new `keyPrefix` from the response, without a full list refresh.

---

## Scenarios

### Scenario: User creates key with all 5 scopes and restrictions

1. User clicks "Create API Key"
2. Create modal opens
3. User selects all 5 scope chips, types `"gpt-4o, claude-3-opus"` in Allowed Models, selects `["openai", "anthropic"]` in Allowed Providers
4. User clicks "Create"
5. POST request sent with `scopes: ["chat:read","chat:write","providers:read","providers:write","admin"]`, `allowedModels: ["gpt-4o","claude-3-opus"]`, `allowedProviders: ["openai","anthropic"]`
6. Success: amber banner shows new raw key
7. User copies key, clicks "Done"
8. List refreshes; new key shows in row with amber "Restricted (2 models, 2 providers)" badge

### Scenario: User rotates a key

1. User clicks rotate icon on a key row
2. Confirmation dialog: "This will immediately invalidate the current key and generate a new one."
3. User confirms
4. POST `/api/api-keys/{id}/rotate`
5. Success: dialog closes, amber banner shows new raw key, list row's `keyPrefix` updates to new value
6. User copies key, dismisses banner

### Scenario: User edits restrictions to clear them

1. User clicks edit on a restricted key
2. Edit modal pre-populates: `allowedModelsInput = "gpt-4o, claude-3-opus"`, `allowedProviders = ["openai"]`
3. User clears both fields to empty
4. User clicks "Save"
5. PUT request sent with `allowedModels: []`, `allowedProviders: []`
6. Modal closes, list row now shows gray "Unrestricted" badge

### Scenario: User attempts create with no scopes selected

1. User opens Create modal
2. User deselects all scope chips
3. User clicks "Create"
4. Validation error shown: "At least one scope is required"
5. No API call made