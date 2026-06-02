// =============================================================================
// API Key types
// =============================================================================

export interface ApiKeyRecordResponse {
  id: string
  label: string
  keyPrefix: string
  scopes: string[]
  tier: string
  isActive: boolean
  revokedAt: string | null
  expiresAt: string | null
  createdAt: string
  lastUsedAt: string | null
  allowedModels: string[]
  allowedProviders: string[]
}

export interface CreateApiKeyResponse {
  key: ApiKeyRecordResponse
  plaintextKey: string
}

export interface PaginationResponse {
  total: number
  limit: number
  offset: number
}

export interface ListApiKeysResponse {
  keys: ApiKeyRecordResponse[]
  pagination: PaginationResponse
}

export interface CreateApiKeyRequest {
  label: string
  scopes: string[]
  tier: string
  expiresAt: string | null
  allowedModels?: string[]
  allowedProviders?: string[]
}

export interface UpdateApiKeyRequest {
  label?: string
  scopes?: string[]
  tier?: string
  isActive?: boolean
  expiresAt?: string | null
  allowedModels?: string[]
  allowedProviders?: string[]
}

// =============================================================================
// Rook API Client
// =============================================================================

/**
 * Rook API Client
 *
 * Base URL is auto-detected from window.location for convenience.
 * Override via window.__ROOK_API_BASE__ or setApiBaseUrl().
 */

export interface HealthResponse {
  status: 'healthy' | 'degraded' | 'no_providers_configured'
  providers: ProviderHealth[]
}

export interface ProviderHealth {
  id: string
  healthy: boolean
  latency_ms: number | null
  last_error: string | null
}

export interface ProviderConnectionResponse {
  id: string
  providerKind: string
  providerRuntimeId: string
  authType: string
  name: string
  priority: number
  isActive: boolean
  config: ConnectionConfigResponse
  testStatus: TestStatusResponse
  createdAt: string
  updatedAt: string
}

export interface ConnectionConfigResponse {
  maxConcurrent: number
  quotaWindowThresholds: { warning: number; error: number }
  defaultModel: string | null
  baseUrl: string | null
}

export interface TestStatusResponse {
  status: 'neverTested' | 'active' | 'unhealthy' | 'expired' | 'unknown'
  lastTestAt: string | null
  latencyMs: number | null
  error: string | null
}

export interface BootstrapStatusResponse {
  is_initialized: boolean
  admin_user_exists: boolean
  // NOTE: setup_token is intentionally NOT present in the wire response.
  // It is an out-of-band secret printed only to server logs. Exposing it
  // via HTTP would allow unauthenticated remote takeover of fresh installations.
}

export interface BootstrapStatus {
  isInitialized: boolean
  adminUserExists: boolean
}

export interface BootstrapSetupRequest {
  setupToken: string
  password: string
}

export interface BootstrapSetupResponse {
  api_key: string
}

export interface BootstrapSetupResult {
  apiKey: string
}

export interface LoginRequest {
  username: string
  password: string
}

export interface LoginResponse {
  session_id: string
  expires_at: string
}

export interface LoginResult {
  sessionId: string
  expiresAt: string
}

export interface CsrfTokenResponse {
  csrf_token: string
}

export interface MeResponse {
  username: string
  displayName: string
}

const STORAGE_KEY = 'rook-api-base-url'

function getBaseUrl(): string {
  // Allow override for development/CI
  if (typeof window !== 'undefined' && (window as unknown as { __ROOK_API_BASE__?: string }).__ROOK_API_BASE__) {
    return (window as unknown as { __ROOK_API_BASE__: string }).__ROOK_API_BASE__
  }
  const stored = localStorage.getItem(STORAGE_KEY)
  if (stored) return stored
  // In development with Vite proxy, use relative URLs
  // The proxy handles forwarding to the backend
  if (import.meta.env.DEV) {
    return '' // Relative URLs for dev proxy
  }
  // Auto-detect from current origin in production
  if (typeof window !== 'undefined') {
    return window.location.origin
  }
  return 'http://localhost:8080'
}

export function setApiBaseUrl(url: string | null): void {
  if (url) {
    localStorage.setItem(STORAGE_KEY, url)
  } else {
    localStorage.removeItem(STORAGE_KEY)
  }
}

function createApiClient() {
  const baseUrl = getBaseUrl()

  async function request<T>(
    path: string,
    options: RequestInit = {}
  ): Promise<T> {
    const url = `${baseUrl}${path}`

    // Extract CSRF token for state-changing requests.
    // The csrf_token cookie is HttpOnly (XSS protection), so it cannot be read
    // from document.cookie. Instead, fetch a fresh token from GET /login which
    // returns it in the response body. The backend validates the double-submit
    // cookie pattern: X-CSRF-Token header must match csrf_token cookie.
    const method = (options.method || 'GET').toUpperCase()
    const isStateChanging = ['POST', 'PUT', 'DELETE', 'PATCH'].includes(method)
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
      ...options.headers as Record<string, string>,
    }

    if (isStateChanging && !headers['X-CSRF-Token']) {
      try {
        const csrfRes = await fetch(`${baseUrl}/login`, { credentials: 'include' })
        if (csrfRes.ok) {
          const csrfBody = await csrfRes.json()
          headers['X-CSRF-Token'] = csrfBody.csrf_token
        }
      } catch {
        // Proceed without CSRF token — request will fail with 403 if required
      }
    }

    const response = await fetch(url, {
      ...options,
      headers,
      credentials: 'include', // Include cookies for session auth
    })

    if (!response.ok) {
      const error = await response.text().catch(() => 'Unknown error')
      throw new Error(`API Error ${response.status}: ${error}`)
    }

    // Handle 204 No Content
    if (response.status === 204) {
      return undefined as T
    }

    return response.json()
  }

  async function getCsrfToken(): Promise<string> {
    const response = await request<CsrfTokenResponse>('/login')
    return response.csrf_token
  }

  return {
    baseUrl,

    // Public endpoints
    async getHealth(): Promise<HealthResponse> {
      return request<HealthResponse>('/health')
    },

    async getBootstrapStatus(): Promise<BootstrapStatus> {
      const response = await request<BootstrapStatusResponse>('/api/bootstrap/status')
      return {
        isInitialized: response.is_initialized,
        adminUserExists: response.admin_user_exists,
      }
    },

    async setupBootstrap(data: BootstrapSetupRequest): Promise<BootstrapSetupResult> {
      const csrfToken = await getCsrfToken()
      const response = await request<BootstrapSetupResponse>('/api/bootstrap/setup', {
        method: 'POST',
        headers: {
          'X-CSRF-Token': csrfToken,
        },
        body: JSON.stringify({
          setup_token: data.setupToken,
          password: data.password,
        }),
      })
      return { apiKey: response.api_key }
    },

    async login(data: LoginRequest): Promise<LoginResult> {
      const csrfToken = await getCsrfToken()
      const response = await request<LoginResponse>('/login', {
        method: 'POST',
        headers: {
          'X-CSRF-Token': csrfToken,
        },
        body: JSON.stringify(data),
      })
      return {
        sessionId: response.session_id,
        expiresAt: response.expires_at,
      }
    },

    async logout(): Promise<void> {
      const csrfToken = await getCsrfToken()
      await request<void>('/logout', {
        method: 'POST',
        headers: { 'X-CSRF-Token': csrfToken },
      })
    },

    async getMe(): Promise<MeResponse | null> {
      try {
        return await request<MeResponse>('/api/me')
      } catch (err) {
        // Only treat HTTP 401 as "not signed in" — rethrow all other errors
        // so callers can handle them appropriately.
        const msg = err instanceof Error ? err.message : String(err)
        if (msg.startsWith('API Error 401:')) {
          return null
        }
        throw err
      }
    },

    // Provider management (requires session auth)
    async getProviders(): Promise<ProviderConnectionResponse[]> {
      return request<ProviderConnectionResponse[]>('/api/providers')
    },

    async getProvider(id: string): Promise<ProviderConnectionResponse> {
      return request<ProviderConnectionResponse>(`/api/providers/${id}`)
    },

    async createProvider(data: CreateProviderRequest): Promise<ProviderConnectionResponse> {
      return request<ProviderConnectionResponse>('/api/providers', {
        method: 'POST',
        body: JSON.stringify(data),
      })
    },

    async updateProvider(id: string, data: UpdateProviderRequest): Promise<ProviderConnectionResponse> {
      return request<ProviderConnectionResponse>(`/api/providers/${id}`, {
        method: 'PUT',
        body: JSON.stringify(data),
      })
    },

    async deleteProvider(id: string): Promise<void> {
      return request<void>(`/api/providers/${id}`, {
        method: 'DELETE',
      })
    },

    async testProvider(id: string): Promise<TestConnectionResponse> {
      return request<TestConnectionResponse>(`/api/providers/${id}/test`, {
        method: 'POST',
      })
    },

    // API Key management (requires session auth)
    async getApiKeys(limit = 20, offset = 0): Promise<ListApiKeysResponse> {
      return request<ListApiKeysResponse>(`/api/api-keys?limit=${limit}&offset=${offset}`)
    },

    async getApiKey(id: string): Promise<ApiKeyRecordResponse> {
      return request<ApiKeyRecordResponse>(`/api/api-keys/${id}`)
    },

    async createApiKey(data: CreateApiKeyRequest): Promise<CreateApiKeyResponse> {
      return request<CreateApiKeyResponse>('/api/api-keys', {
        method: 'POST',
        body: JSON.stringify(data),
      })
    },

    async updateApiKey(id: string, data: UpdateApiKeyRequest): Promise<ApiKeyRecordResponse> {
      return request<ApiKeyRecordResponse>(`/api/api-keys/${id}`, {
        method: 'PUT',
        body: JSON.stringify(data),
      })
    },

    async revokeApiKey(id: string): Promise<void> {
      return request<void>(`/api/api-keys/${id}`, {
        method: 'DELETE',
      })
    },

    async rotateApiKey(id: string): Promise<CreateApiKeyResponse> {
      return request<CreateApiKeyResponse>(`/api/api-keys/${id}/rotate`, {
        method: 'POST',
      })
    },
  }
}

export interface CreateProviderRequest {
  providerKind: string
  providerRuntimeId: string
  authType: string
  name: string
  priority: number
  isActive: boolean
  credentials: ApiKeyCredentialsInput | OAuthCredentialsInput
  config: ConnectionConfigInput
}

export interface UpdateProviderRequest {
  expectedUpdatedAt: string
  providerKind?: string
  providerRuntimeId?: string
  authType?: string
  name?: string
  priority?: number
  isActive?: boolean
  credentials?: ApiKeyCredentialsInput | OAuthCredentialsInput
  config?: ConnectionConfigInput
}

export interface ApiKeyCredentialsInput {
  apiKey: string
}

export interface OAuthCredentialsInput {
  email: string
  accessToken: string
  refreshToken: string
  expiresAt: number
  scope: string
  idToken: string
  projectId: string
}

export interface ConnectionConfigInput {
  maxConcurrent: number
  quotaWindowThresholds: { warning: number; error: number }
  defaultModel?: string
  baseUrl?: string
}

export interface TestConnectionResponse {
  ok: boolean | null
  status: string
  latencyMs: number | null
  error: string | null
}

// Singleton instance
let apiClient: ReturnType<typeof createApiClient> | null = null

export function useApi(): ReturnType<typeof createApiClient> {
  if (!apiClient) {
    apiClient = createApiClient()
  }
  return apiClient
}
