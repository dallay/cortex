// =============================================================================
// API Key types
// =============================================================================

import type {ProviderKind} from "@/config/providerCatalog";

/**
 * Wire format for `authType` in API requests/responses.
 *
 * Matches the Rust `AuthType` enum in
 * `crates/domain/rook-core/src/provider_connection.rs`, which serializes
 * to camelCase via `#[serde(rename_all = "camelCase")]`. This is DIFFERENT
 * from the internal `AuthType` type in `@/config/providerCatalog`, which
 * is the form-state value (`'apikey'` lowercase, no camelCase). Use
 * `WireAuthType` for the API boundary; use `AuthType` for in-component
 * state. The dialog's `wireAuthType()` helper bridges between them.
 */
export type WireAuthType = "apiKey" | "oauth";

export interface ApiKeyRecordResponse {
  id: string;
  label: string;
  keyPrefix: string;
  scopes: string[];
  tier: string;
  isActive: boolean;
  revokedAt: string | null;
  expiresAt: string | null;
  createdAt: string;
  lastUsedAt: string | null;
  allowedModels: string[];
  allowedProviders: string[];
}

// =============================================================================
// Model catalog types
// =============================================================================

/**
 * One group of model ids for a single active provider connection.
 * Returned by `GET /api/models` and consumed by the API key restriction UI.
 */
export interface ProviderModelsGroup {
  providerId: string;
  providerName: string;
  providerKind: ProviderKind;
  models: string[];
}

/**
 * Response body for `GET /api/models`.
 *
 * The shape must match the Rust DTO in
 * `crates/infrastructure/transport-axum/src/handlers/models_dto.rs`.
 */
export interface ListModelsResponse {
  models: ProviderModelsGroup[];
}

export interface CreateApiKeyResponse {
  key: ApiKeyRecordResponse;
  plaintextKey: string;
}

export interface PaginationResponse {
  total: number;
  limit: number;
  offset: number;
}

export interface ListApiKeysResponse {
  keys: ApiKeyRecordResponse[];
  pagination: PaginationResponse;
}

export interface TestCredentialsPayload {
  providerKind: ProviderKind;
  providerRuntimeId: string;
  authType: WireAuthType;
  credentials: {
    apiKey?: string;
    email?: string;
    accessToken?: string;
    refreshToken?: string;
    expiresAt?: number;
    scope?: string;
    idToken?: string;
    projectId?: string;
  };
  config: {
    maxConcurrent: number;
    quotaWindowThresholds: {
      warning: number;
      error: number;
    };
    defaultModel?: string;
    baseUrl?: string;
  };
}

export interface CreateApiKeyRequest {
  label: string;
  scopes: string[];
  tier: string;
  expiresAt: string | null;
  allowedModels?: string[];
  allowedProviders?: string[];
}

export interface UpdateApiKeyRequest {
  label?: string;
  scopes?: string[];
  tier?: string;
  isActive?: boolean;
  expiresAt?: string | null;
  allowedModels?: string[];
  allowedProviders?: string[];
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
  status: "healthy" | "degraded" | "no_providers_configured";
  providers: ProviderHealth[];
}

export interface ProviderHealth {
  id: string;
  healthy: boolean;
  latency_ms: number | null;
  last_error: string | null;
}

export interface ProviderConnectionResponse {
  id: string;
  providerKind: ProviderKind;
  providerRuntimeId: string;
  authType: WireAuthType;
  name: string;
  priority: number;
  isActive: boolean;
  config: ConnectionConfigResponse;
  testStatus: TestStatusResponse;
  createdAt: string;
  updatedAt: string;
}

export interface ConnectionConfigResponse {
  maxConcurrent: number;
  quotaWindowThresholds: { warning: number; error: number };
  defaultModel: string | null;
  baseUrl: string | null;
}

export interface TestStatusResponse {
  status: "neverTested" | "active" | "unhealthy" | "expired" | "unknown";
  lastTestAt: string | null;
  latencyMs: number | null;
  error: string | null;
}

export interface BootstrapStatusResponse {
  is_initialized: boolean;
  admin_user_exists: boolean;
  // NOTE: setup_token is intentionally NOT present in the wire response.
  // It is an out-of-band secret printed only to server logs. Exposing it
  // via HTTP would allow unauthenticated remote takeover of fresh installations.
}

export interface BootstrapStatus {
  isInitialized: boolean;
  adminUserExists: boolean;
}

export interface BootstrapSetupRequest {
  setupToken: string;
  password: string;
}

export interface BootstrapSetupResponse {
  api_key: string;
}

export interface BootstrapSetupResult {
  apiKey: string;
}

export interface LoginRequest {
  username: string;
  password: string;
}

export interface LoginResponse {
  session_id: string;
  expires_at: string;
  /** Present when the session was freshly authenticated — carries the same
   *  csrf_token the backend set as the HttpOnly cookie. Used to seed the
   *  client-side CSRF cache so the first state-changing request after login
   *  has a pre-warmed token. */
  csrf_token?: string;
}

export interface LoginResult {
  sessionId: string;
  expiresAt: string;
}

export interface CsrfTokenResponse {
  csrf_token: string;
}

export interface MeResponse {
  username: string;
  displayName: string;
}

const STORAGE_KEY = "rook-api-base-url";

function getBaseUrl(): string {
  // Allow override for development/CI
  if (
    typeof window !== "undefined" &&
    (window as unknown as { __ROOK_API_BASE__?: string }).__ROOK_API_BASE__
  ) {
    return (window as unknown as { __ROOK_API_BASE__: string })
      .__ROOK_API_BASE__;
  }
  const stored = typeof window !== "undefined" ? localStorage.getItem(STORAGE_KEY) : null;
  if (stored) return stored;
  // In development with Vite proxy, use relative URLs
  // The proxy handles forwarding to the backend
  if (import.meta.env.DEV) {
    return ""; // Relative URLs for dev proxy
  }
  // Auto-detect from current origin in production
  if (typeof window !== "undefined") {
    return window.location.origin;
  }
  return "http://127.0.0.1:3773";
}

export function setApiBaseUrl(url: string | null): void {
  if (url) {
    localStorage.setItem(STORAGE_KEY, url);
  } else {
    localStorage.removeItem(STORAGE_KEY);
  }
}

function createApiClient() {
  const baseUrl = getBaseUrl();

  // CSRF token cache + retry-on-403 + login-seeded cache.
  // The double-submit cookie pattern requires that every state-changing
  // request carries the same csrf_token in the cookie and the X-CSRF-Token
  // header. Previously the client refetched GET /login on every state-changing
  // call, which exposed a cookie-jar race in WebKit (Safari): the response
  // body returned the token before the Set-Cookie landed in the jar, so the
  // next request went out with a valid header but no cookie and the backend
  // rejected it as csrf_missing.
  //
  // The fix (issue #82, suggested fix #2) is two-pronged:
  // 1. The POST /login response body now carries a fresh csrf_token (the same
  //    value set as the HttpOnly cookie). The login() call seeds cachedCsrfToken
  //    from this response body so the very first state-changing request AFTER
  //    login has a pre-warmed token and never needs GET /login at all.
  // 2. The retry-on-403 guard protects against any remaining token staleness
  //    (server-side rotation, or edge cases where the login cookie wasn't
  //    visible to the first state-changing request even with pre-warming).
  //
  // Result: WebKit, Chromium, and Firefox all work correctly. The cache also
  // eliminates a per-request round-trip for all browsers, reducing latency.
  let cachedCsrfToken: string | null = null;

  async function request<T>(
    path: string,
    options: RequestInit = {},
  ): Promise<T> {
    const url = `${baseUrl}${path}`;

    // Extract CSRF token for state-changing requests.
    // The csrf_token cookie is HttpOnly (XSS protection), so it cannot be read
    // from document.cookie. Instead, fetch a fresh token from GET /login which
    // returns it in the response body. The backend validates the double-submit
    // cookie pattern: X-CSRF-Token header must match csrf_token cookie.
    const method = (options.method || "GET").toUpperCase();
    const isStateChanging = ["POST", "PUT", "DELETE", "PATCH"].includes(method);
    const headers: Record<string, string> = {
      "Content-Type": "application/json",
      ...(options.headers as Record<string, string>),
    };

    if (isStateChanging && !headers["X-CSRF-Token"]) {
      try {
        const token = await getCsrfToken();
        headers["X-CSRF-Token"] = token;
      } catch (err) {
        // Surface the failure so it shows up in DevTools — the request will
        // likely fail with csrf_missing next, but operators need the root
        // cause (e.g. backend 5xx) to diagnose the real issue. See issue #82.
        console.warn("[Rook API] Failed to fetch CSRF token:", err);
        // Proceed without CSRF token — request will fail with 403 if required
      }
    }

    let response = await fetch(url, {
      ...options,
      headers,
      credentials: "include", // Include cookies for session auth
    });

    // Retry once on CSRF failure: the cached token may be stale (server
    // rotation) or the cookie may not have landed in the jar yet (WebKit
    // race described in issue #82). On a real authz error (e.g. 403 from a
    // route's permission check) the body will not mention csrf_missing or
    // csrf_mismatch, so we leave the response alone and let it throw below.
    if (isStateChanging && response.status === 403) {
      const probe = await response
        .clone()
        .text()
        .catch(() => "");
      if (probe.includes("csrf_missing") || probe.includes("csrf_mismatch")) {
        cachedCsrfToken = null;
        try {
          headers["X-CSRF-Token"] = await getCsrfToken();
          response = await fetch(url, {
            ...options,
            headers,
            credentials: "include",
          });
        } catch (err) {
          console.warn("[Rook API] CSRF token refresh failed:", err);
          // Fall through to the error path with the 403 response
        }
      }
    }

    if (!response.ok) {
      const error = await response.text().catch(() => "Unknown error");
      throw new Error(`API Error ${response.status}: ${error}`);
    }

    // Handle 204 No Content
    if (response.status === 204) {
      return undefined as T;
    }

    return response.json();
  }

  async function getCsrfToken(): Promise<string> {
    if (cachedCsrfToken) return cachedCsrfToken;
    const response = await fetch(`${baseUrl}/login`, {
      credentials: "include",
    });
    if (!response.ok) {
      throw new Error(`Failed to fetch CSRF token: HTTP ${response.status}`);
    }
    const body = (await response.json()) as CsrfTokenResponse;
    cachedCsrfToken = body.csrf_token;
    return cachedCsrfToken;
  }

  /** Overwrite the cached CSRF token directly. Used by login() to
   *  seed the cache from the POST response body so that the very first
   *  state-changing request after login has a pre-warmed token. */
  function seedCsrfCache(token: string): void {
    cachedCsrfToken = token;
  }

  return {
    baseUrl,

    // Public endpoints
    async getHealth(): Promise<HealthResponse> {
      return request<HealthResponse>("/health");
    },

    async getBootstrapStatus(): Promise<BootstrapStatus> {
      const response = await request<BootstrapStatusResponse>(
        "/api/bootstrap/status",
      );
      return {
        isInitialized: response.is_initialized,
        adminUserExists: response.admin_user_exists,
      };
    },

    async setupBootstrap(
      data: BootstrapSetupRequest,
    ): Promise<BootstrapSetupResult> {
      const csrfToken = await getCsrfToken();
      const response = await request<BootstrapSetupResponse>(
        "/api/bootstrap/setup",
        {
          method: "POST",
          headers: {
            "X-CSRF-Token": csrfToken,
          },
          body: JSON.stringify({
            setup_token: data.setupToken,
            password: data.password,
          }),
        },
      );
      return {apiKey: response.api_key};
    },

    async login(data: LoginRequest): Promise<LoginResult> {
      const csrfToken = await getCsrfToken();
      const response = await request<LoginResponse>("/login", {
        method: "POST",
        headers: {
          "X-CSRF-Token": csrfToken,
        },
        body: JSON.stringify(data),
      });
      // Seed the cache from the POST response body so the very first
      // state-changing request after login has a pre-warmed token.
      // This eliminates the GET /login round-trip that caused the WebKit
      // cookie-jar race. See issue #82 (suggested fix #2).
      if (response.csrf_token) {
        seedCsrfCache(response.csrf_token);
      }
      return {
        sessionId: response.session_id,
        expiresAt: response.expires_at,
      };
    },

    async logout(): Promise<void> {
      const csrfToken = await getCsrfToken();
      await request<void>("/logout", {
        method: "POST",
        headers: {"X-CSRF-Token": csrfToken},
      });
    },

    async getMe(): Promise<MeResponse | null> {
      try {
        return await request<MeResponse>("/api/me");
      } catch (err) {
        // Only treat HTTP 401 as "not signed in" — rethrow all other errors
        // so callers can handle them appropriately.
        const msg = err instanceof Error ? err.message : String(err);
        if (msg.startsWith("API Error 401:")) {
          return null;
        }
        throw err;
      }
    },

    // Provider management (requires session auth)
    async getProviders(): Promise<ProviderConnectionResponse[]> {
      return request<ProviderConnectionResponse[]>("/api/providers");
    },

    async getProvider(id: string): Promise<ProviderConnectionResponse> {
      return request<ProviderConnectionResponse>(`/api/providers/${id}`);
    },

    async createProvider(
      data: CreateProviderRequest,
    ): Promise<ProviderConnectionResponse> {
      return request<ProviderConnectionResponse>("/api/providers", {
        method: "POST",
        body: JSON.stringify(data),
      });
    },

    async updateProvider(
      id: string,
      data: UpdateProviderRequest,
    ): Promise<ProviderConnectionResponse> {
      return request<ProviderConnectionResponse>(`/api/providers/${id}`, {
        method: "PUT",
        body: JSON.stringify(data),
      });
    },

    async deleteProvider(id: string): Promise<void> {
      return request<void>(`/api/providers/${id}`, {
        method: "DELETE",
      });
    },

    async testProvider(id: string): Promise<TestConnectionResponse> {
      return request<TestConnectionResponse>(`/api/providers/${id}/test`, {
        method: "POST",
      });
    },

    async testCredentials(
      payload: TestCredentialsPayload,
    ): Promise<TestConnectionResponse> {
      return request<TestConnectionResponse>(
        "/api/providers/test-credentials",
        {
          method: "POST",
          body: JSON.stringify(payload),
        },
      );
    },

    // API Key management (requires session auth)
    async getApiKeys(limit = 20, offset = 0): Promise<ListApiKeysResponse> {
      return request<ListApiKeysResponse>(
        `/api/api-keys?limit=${limit}&offset=${offset}`,
      );
    },

    async getApiKey(id: string): Promise<ApiKeyRecordResponse> {
      return request<ApiKeyRecordResponse>(`/api/api-keys/${id}`);
    },

    async createApiKey(
      data: CreateApiKeyRequest,
    ): Promise<CreateApiKeyResponse> {
      return request<CreateApiKeyResponse>("/api/api-keys", {
        method: "POST",
        body: JSON.stringify(data),
      });
    },

    async updateApiKey(
      id: string,
      data: UpdateApiKeyRequest,
    ): Promise<ApiKeyRecordResponse> {
      return request<ApiKeyRecordResponse>(`/api/api-keys/${id}`, {
        method: "PUT",
        body: JSON.stringify(data),
      });
    },

    async revokeApiKey(id: string): Promise<void> {
      return request<void>(`/api/api-keys/${id}`, {
        method: "DELETE",
      });
    },

    async rotateApiKey(id: string): Promise<CreateApiKeyResponse> {
      return request<CreateApiKeyResponse>(`/api/api-keys/${id}/rotate`, {
        method: "POST",
      });
    },

    // Model catalog (requires session auth)
    /**
     * Returns the model ids available to the API key restriction UI,
     * grouped by active provider connection.
     */
    async getAvailableModels(): Promise<ListModelsResponse> {
      return request<ListModelsResponse>("/api/models");
    },
  };
}

export interface CreateProviderRequest {
  providerKind: ProviderKind;
  providerRuntimeId: string;
  authType: WireAuthType;
  name: string;
  priority: number;
  isActive: boolean;
  credentials: ApiKeyCredentialsInput | OAuthCredentialsInput;
  config: ConnectionConfigInput;
}

export interface UpdateProviderRequest {
  expectedUpdatedAt: string;
  providerKind?: ProviderKind;
  providerRuntimeId?: string;
  authType?: WireAuthType;
  name?: string;
  priority?: number;
  isActive?: boolean;
  credentials?: ApiKeyCredentialsInput | OAuthCredentialsInput;
  config?: ConnectionConfigInput;
}

export interface ApiKeyCredentialsInput {
  apiKey: string;
}

export interface OAuthCredentialsInput {
  email: string;
  accessToken: string;
  refreshToken: string;
  expiresAt: number;
  scope: string;
  idToken: string;
  projectId: string;
}

export interface ConnectionConfigInput {
  maxConcurrent: number;
  quotaWindowThresholds: { warning: number; error: number };
  defaultModel?: string;
  baseUrl?: string;
}

export type TestConnectionStatus =
  | "ok"
  | "warning"
  | "unhealthy"
  | "unknown"
  | "expired";

export interface TestConnectionResponse {
  /**
   * Whether the credentials are usable. `true` for `Healthy`,
   * `Warning`, and `Unknown`; `false` for `Unhealthy` and `Expired`.
   * The dashboard's Save button is enabled iff `valid === true`,
   * regardless of `status` or `warning`.
   */
  valid: boolean;
  /** One of `"ok" | "warning" | "unhealthy" | "unknown" | "expired"`. */
  status: TestConnectionStatus;
  latencyMs: number | null;
  error: string | null;
  /**
   * Soft signal surfaced as a yellow alert. Set when credentials are
   * valid but the probe saw a non-fatal condition (HTTP 429, no API
   * key configured, etc.). A warning does not block Save.
   */
  warning: string | null;
  /**
   * Probe method used to derive this result. Free-form string
   * (`"models_list"`, `"v1beta_models"`, `"tags_reachability"`,
   * `"chat_probe"`, `"not_supported"`, `"oauth_expired"`, ...).
   * Optional: providers that don't probe return `null`.
   */
  method: string | null;
}

// Singleton instance
let apiClient: ReturnType<typeof createApiClient> | null = null;

export function useApi(): ReturnType<typeof createApiClient> {
  if (!apiClient) {
    apiClient = createApiClient();
  }
  return apiClient;
}
