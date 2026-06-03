/**
 * Scope registry — the canonical list of API key scopes the dashboard
 * exposes in the create/edit form.
 *
 * Adding a new scope:
 *   1. Add a backend scope to `rook-core`'s `ApiKeyScope` enum.
 *   2. Add a matching entry to `SCOPES` below (with `defaultChecked: true`
 *      if the new scope should be enabled by default for new keys).
 *   3. The form renders it automatically — no template changes needed.
 *
 * The `group` field is used to render scopes under section headers in
 * the form (e.g. "Chat", "Providers", "Administrative"). The `danger`
 * flag marks scopes that should render with a visual warning (red badge
 * + shield icon) because they grant privileged access.
 */
export type ScopeGroup = 'chat' | 'providers' | 'admin'

export interface ScopeDef {
  /** The scope value sent to the backend (e.g. `'chat:read'`). */
  value: string
  /** Human-readable label rendered in the UI. */
  label: string
  /** Short description of what the scope grants. */
  description: string
  /** Group used to organize scopes in the form. */
  group: ScopeGroup
  /** When true, the scope is rendered with a danger visual indicator. */
  danger?: boolean
  /** Whether the scope should be checked by default in the create form. */
  defaultChecked: boolean
}

export const SCOPES: readonly ScopeDef[] = [
  {
    value: 'chat:read',
    label: 'Chat Read',
    description: 'Read chat completions and conversation history.',
    group: 'chat',
    defaultChecked: true,
  },
  {
    value: 'chat:write',
    label: 'Chat Write',
    description: 'Send chat completion requests through the proxy.',
    group: 'chat',
    defaultChecked: true,
  },
  {
    value: 'providers:read',
    label: 'Providers Read',
    description: 'List configured providers and their health status.',
    group: 'providers',
    defaultChecked: true,
  },
  {
    value: 'providers:write',
    label: 'Providers Write',
    description: 'Create, update, and delete provider connections.',
    group: 'providers',
    defaultChecked: true,
  },
  {
    value: 'admin',
    label: 'Admin',
    description: 'Full administrative access — manage API keys and settings.',
    group: 'admin',
    danger: true,
    defaultChecked: false,
  },
] as const

/** Scope values that are checked by default in the create form. */
export const DEFAULT_SCOPES: readonly string[] = SCOPES.filter((s) => s.defaultChecked).map(
  (s) => s.value,
)
