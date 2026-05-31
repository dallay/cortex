# API Key Dashboard Specification

## Purpose

Defines the Vue.js dashboard UI for API key management. The dashboard is a single-page application at `/api-keys` that enables admins to create, list, update, and revoke API keys.

---

## Page Structure

**Route**: `/api-keys` → `ApiKeysView.vue`

**Layout**: Full-page with header, create section, and keys table

```
┌─────────────────────────────────────────────────────────────┐
│ [Page Header]                                               │
│  Title: "API Keys"                                          │
│  Subtitle: "Manage your API keys for accessing Rook"        │
│  [Create Key Button]                                        │
├─────────────────────────────────────────────────────────────┤
│ [Create Modal] (shown when create is clicked)               │
│  - Label input │
│  - Scopes multi-select (read, write)                        │
│  - Tier dropdown (Free, Pro, Enterprise)                    │
│  - Expiration date picker (optional)                       │
│  - [Create] [Cancel]                                        │
├─────────────────────────────────────────────────────────────┤
│ [Keys Table] │
│  Columns: Name | Key Prefix | Scopes | Tier | Status |      │
│           Created | Last Used | Actions │
│  Rows: Paginated list of keys │
│  Empty state: "No API keys yet"                             │
├─────────────────────────────────────────────────────────────┤
│ [Pagination Controls]                                        │
│  Prev | Page1 of 3 | Next │
└─────────────────────────────────────────────────────────────┘
```

---

## Components

### ApiKeysView.vue (main page)

**State**:

- `apiKeys: Ref<ApiKeyRecord[]>` — current page of keys
- `pagination: Ref<PaginationState>` — `{ total, limit, offset }`
- `showCreateModal: Ref<boolean>` — create modal visibility
- `showKeyValue: Ref<string | null>` — which key's value is revealed
- `isLoading: Ref<boolean>` — loading state
- `error: Ref<string | null>` — error message

**Methods**:

- `fetchKeys()` — loads current page from `GET /api/api-keys`
- `createKey(request)` — calls `POST /api/api-keys`, shows raw key
- `updateKey(id, request)` — calls `PUT /api/api-keys/:id`
- `revokeKey(id)` — calls `DELETE /api/api-keys/:id`, refreshes list
- `copyToClipboard(text)` — copies key to clipboard
- `maskKey(key)` — shows `rook_fake_a...xxxx` format
- `toggleKeyVisibility(id)` — reveals/hides raw key value

### CreateKeyModal.vue (inline in ApiKeysView)

**Props**: `open: boolean`

**Emits**: `close`, `created(key)`

**Fields**:

- `label: string` — required, min 1 char
- `scopes: string[]` — required, at least one of `["read", "write"]`
- `tier: "free" | "pro" | "enterprise"` — default `"free"`
- `expiresAt: string | null` — ISO date string, optional

**Validation**:

- Label cannot be empty
- At least one scope must be selected
- Expiration must be in the future (if provided)

**On Create**:

1. POST to `/api/api-keys`
2. On success: show success toast with raw key displayed prominently
3. Emit `created` event with the new key
4. Close modal and refresh list

### KeyDisplayBanner.vue (shown after create)

**Shown**: When `plaintext_key` is available after create

**Content**:

```
┌─────────────────────────────────────────────────────────────┐
│ ⚠️ Save this key — it will not be shown again │
│                                                             │
│ Key: rook_fake_a3f8b2c1d0e9f2a3b4c5d6e7f8a9b0c │
│                                                             │
│ [Copy to Clipboard]                                         │
└─────────────────────────────────────────────────────────────┘
```

### EditKeyModal.vue (inline in ApiKeysView)

**Props**: `open: boolean`, `key: ApiKeyRecord | null`

**Emits**: `close`, `updated`

**Fields** (all optional, only changed fields sent):

- `label: string`
- `scopes: string[]`
- `tier: "free" | "pro" | "enterprise"`
- `expiresAt: string | null` (null = clear expiration)

**On Update**:

1. PUT to `/api/api-keys/:id`
2. On success: show success toast
3. Emit `updated` event
4. Close modal and refresh list

---

## API Interactions

### Fetch Keys

```
GET /api/api-keys?limit=20&offset=0
```

**Response**:

```json
{
  "keys": [
    {
      "id": "key_abc123",
      "label": "opencode-agent",
      "key_prefix": "rook_fake_a",
      "scopes": ["read", "write"],
      "tier": "pro",
      "is_active": true,
      "revoked_at": null,
      "expires_at": "2026-12-31T23:59:59Z",
      "created_at": "2026-05-31T12:00:00Z",
      "last_used_at": null
    }
  ],
  "pagination": { "total": 1, "limit": 20, "offset": 0 }
}
```

### Create Key

```
POST /api/api-keys
{
  "label": "opencode-agent",
  "scopes": ["read", "write"],
  "tier": "pro",
  "expires_at": "2026-12-31T23:59:59Z"
}
```

**Response** (201):

```json
{
  "key": { ... },
  "plaintext_key": "rook_fake_a3f8b2c1d0e9f2a3b4c5d6e7f8a9b0c"
}
```

### Update Key

```
PUT /api/api-keys/:id
{
  "label": "updated-label",
  "scopes": ["read"],
  "tier": "enterprise",
  "expires_at": null
}
```

### Revoke Key

```
DELETE /api/api-keys/:id
```

**Response**: `204 No Content`

---

## UI States

### Empty State

When `keys.length === 0`:

```
┌─────────────────────────────────────────────────────────────┐
│                                                             │
│                    🔑 No API keys yet                       │
│                                                             │
│     Create your first API key to start managing AI agents    │
│                                                             │
│                  [Create API Key]                            │
│                                                             │
└─────────────────────────────────────────────────────────────┘
```

### Loading State

- Skeleton rows in table
- Disabled buttons
- `isLoading = true`

### Error State

- Error banner below header
- `error` message displayed
- Retry button

### Key Status Badge

| Status                                  | Badge            |
|-----------------------------------------|------------------|
| `is_active = true`, no `revoked_at`     | Green "Active"   |
| `is_active = false` OR `revoked_at` set | Red "Revoked"    |
| `expires_at` in past                    | Orange "Expired" |

---

## Requirements

### REQ-UI-1: Create Modal

The dashboard SHALL show a modal dialog for creating new API keys with fields for label, scopes, tier, and optional expiration date.

### REQ-UI-2: Raw Key Display

The dashboard SHALL display the raw key prominently after creation with a warning that it will not be shown again.

### REQ-UI-3: Copy to Clipboard

The dashboard SHALL provide a one-click copy button for the raw key after creation.

### REQ-UI-4: List Pagination

The dashboard SHALL display pagination controls and fetch pages from the API with `limit` and `offset` parameters.

### REQ-UI-5: Key Prefix Display

The dashboard SHALL display the `key_prefix` (first 8 chars) for each key in the list view.

### REQ-UI-6: Last Used Timestamp

The dashboard SHALL display `last_used_at` for each key, showing "—" if never used.

### REQ-UI-7: Revoke Action

The dashboard SHALL provide a revoke button that calls `DELETE /api/api-keys/:id` and refreshes the list.

### REQ-UI-8: Edit Modal

The dashboard SHALL provide an edit modal for updating key label, scopes, tier, and expiration.

### REQ-UI-9: Status Badges

The dashboard SHALL show status badges for Active, Revoked, and Expired states.

### REQ-UI-10: Refresh After Mutation

The dashboard SHALL refresh the key list after every create, update, or revoke operation.

---

## Scenarios

### Scenario: Create first API key

1. User clicks "Create API Key" button
2. Create modal opens
3. User fills in label "opencode-agent", selects scopes ["read", "write"], tier "Pro"
4. User clicks "Create"
5. POST request sent
6. Success: modal closes, raw key banner shown with copy button
7. User clicks "Copy to Clipboard"
8. List refreshes and shows the new key

### Scenario: Revoke a key

1. User clicks revoke icon on a key row
2. Confirmation dialog: "Are you sure you want to revoke this key?"
3. User confirms
4. DELETE request sent
5. List refreshes; key shows "Revoked" badge
6. Toast: "Key revoked successfully"

### Scenario: Edit key expiration

1. User clicks edit icon on a key row
2. Edit modal opens with current values
3. User clears expiration date field
4. User clicks "Save"
5. PUT request sent with `expires_at: null`
6. List refreshes; key shows no expiration

### Scenario: Pagination navigation

1. User has 50 keys
2. Page 1 shows keys 1-20
3. User clicks "Next"
4. API called with `offset=20`
5. Page 2 shows keys 21-40
6. User clicks "Previous"
7. API called with `offset=0`
8. Page 1 shown

### Scenario: Empty state

1. User navigates to API keys page
2. No keys exist
3. Empty state shown with "No API keys yet" message
4. User clicks "Create API Key"
5. Create modal opens
