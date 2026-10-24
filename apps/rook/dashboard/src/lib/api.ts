// =============================================================================
// API Key types
// =============================================================================

import type { ProviderKind } from "@/config/providerCatalog";

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

// =============================================================================
// Combo types
// =============================================================================

/**
 * Wire format for the combo strategy. The backend accepts only `priority`
 * (`ComboStrategy` in `rook-core`, validated by `parse_combo_strategy` in
 * `combo_dto.rs`).
 */
export type ComboStrategy = "priority";

/**
 * Response step. Mirrors `ComboStepResponse` in
 * `crates/infrastructure/transport-axum/src/combo_dto.rs`.
 */
export interface ComboStepResponse {
	providerId: string;
	model: string;
	connectionId: string | null;
	priority: number;
}

/**
 * Response body for a single combo. Mirrors `ComboResponse` in `combo_dto.rs`.
 */
export interface ComboResponse {
	id: string;
	name: string;
	strategy: ComboStrategy;
	steps: ComboStepResponse[];
	createdAt: string;
	updatedAt: string;
}

/** Response body for `GET /api/combos`. Mirrors `ComboListResponse`. */
export interface ComboListResponse {
	combos: ComboResponse[];
}

/**
 * Request step. The backend uses `deny_unknown_fields`, so this shape must
 * stay identical to `CreateComboStepRequest` in `combo_dto.rs`.
 */
export interface ComboStepInput {
	providerId: string;
	model: string;
	connectionId?: string | null;
	priority: number;
}

/** Request body for `POST /api/combos` and `PUT /api/combos/{id}`. */
export interface CreateComboRequest {
	name: string;
	strategy: ComboStrategy;
	steps: ComboStepInput[];
}

export type UpdateComboRequest = CreateComboRequest;

/** Dashboard representation of the combo alias response DTO. */
export interface ComboAliasResponse {
	alias: string;
	comboId: string;
}

export interface ModelAliasResponse {
	alias: string;
	canonical: string;
	providerId: string | null;
	createdAt: string;
}

export interface CreateModelAliasRequest {
	alias: string;
	canonical: string;
	providerId?: string | null;
}

export type RateLimitScope = "api-key" | "ip-address" | "global";

export interface RateLimitRuleResponse {
	id: string;
	scope: RateLimitScope;
	target: string;
	requests_per_minute: number;
	requests_per_day: number | null;
	tokens_per_minute: number | null;
}

export interface CreateRateLimitRequest {
	scope: RateLimitScope;
	target: string;
	requests_per_minute: number;
	requests_per_day?: number | null;
	tokens_per_minute?: number | null;
}

export type CircuitState = "open" | "closed" | "halfopen";

export interface CircuitStateResponse {
	provider: string;
	failures: number;
	state: CircuitState;
	last_failure: string | null;
	cooldown_until: string | null;
	rate_limit_reset: number | null;
}

export interface ResilienceResponse {
	circuit_states: CircuitStateResponse[];
}

export interface PercentileStatsResponse {
	p50: number;
	p95: number;
	p99: number;
	avg: number;
	min: number;
	max: number;
	count: number;
}

export interface ProviderTelemetryResponse {
	provider_id: string;
	total_requests: number;
	success_count: number;
	failure_count: number;
	rate_limited_count: number;
	latency_stats?: PercentileStatsResponse;
	ttft_stats?: PercentileStatsResponse;
}

export interface TelemetrySummaryResponse {
	providers: ProviderTelemetryResponse[];
}

export interface CacheStatsResponse {
	hits: number;
	misses: number;
	evictions: number;
	entries: number;
	max_entries: number;
	token_cache?: {
		hits: number;
		misses: number;
		tokens_saved: number;
		estimated_cost_saved_usd: number;
	};
}

export interface CacheSignatureResponse {
	signature: string;
	request_id: string;
	model: string;
	provider: string;
	cached_at: string;
	last_accessed: string;
	request_metadata: {
		message_count: number;
		has_tools: boolean;
		max_tokens: number | null;
		temperature: number | null;
	};
}

export interface CachedCompletionResponse {
	id: string;
	model: string;
	choices: unknown[];
	usage?: unknown;
	[key: string]: unknown;
}

export interface RateLimitStatusResponse {
	scope: RateLimitScope;
	target: string;
	current_minute_count: number;
	current_day_count: number;
	remaining_minute: number;
	remaining_day: number;
	reset_at: string;
	active_requests_per_minute: number;
	active_requests_per_day: number | null;
}

export interface UsageQuery {
	provider?: string;
	model?: string;
	apiKeyId?: string;
	connectionId?: string;
	start?: string;
	end?: string;
	status?: string;
	offset?: number;
	limit?: number;
}

export interface UsageEntryResponse {
	request_id: string;
	provider: string;
	model: string;
	status: string;
	requested_tier: string | null;
	api_key_id: string | null;
	connection_id: string | null;
	tokens_prompt: number | null;
	tokens_completion: number | null;
	tokens_cache_read: number | null;
	tokens_cache_creation: number | null;
	tokens_reasoning: number | null;
	ttft_ms: number | null;
	latency_ms: number;
	cost_usd: number | null;
	timestamp: string;
}

export interface UsageListResponse {
	entries: UsageEntryResponse[];
	total: number;
}

export interface UsageSummaryResponse {
	total_requests: number;
	total_prompt_tokens: number;
	total_completion_tokens: number;
	total_cache_read_tokens: number;
	total_cache_creation_tokens: number;
	total_reasoning_tokens: number;
	avg_ttft_ms: number | null;
	avg_latency_ms: number;
}

export interface UsageCostResponse {
	total_cost_usd: number;
	known_cost_entries: number;
	unknown_cost_entries: number;
	by_provider: Record<string, number>;
	by_model: Record<string, number>;
	by_api_key: Record<string, number>;
}

/** Wire representation returned by the combo alias routes. */
interface ComboAliasWireResponse {
	alias: string;
	combo_id: string;
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
 * Base URL is auto-detected from globalThis.location for convenience.
 * Override via globalThis.__ROOK_API_BASE__ or setApiBaseUrl().
 */

export interface HealthResponse {
	status: "healthy" | "degraded" | "no_providers_configured";
	app_name?: string;
	version?: string;
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
	oauthStatus?: "connected" | "expired" | "reauthentication_required";
	oauthExpiresAt?: number;
	oauthClientId?: string;
	oauthTenantId?: string | null;
}

export const OAUTH_FLOW_TYPES = {
	AUTHORIZATION_CODE: "authorizationCode",
	DEVICE_CODE: "deviceCode",
} as const;

export type OAuthFlowType =
	(typeof OAUTH_FLOW_TYPES)[keyof typeof OAUTH_FLOW_TYPES];

export interface OAuthFlowStartRequest {
	flowType: OAuthFlowType;
	connectionId?: string;
	providerRuntimeId: string;
	name: string;
	priority: number;
	isActive: boolean;
	config: ConnectionConfigInput;
	clientId: string;
	clientSecret?: string;
	tenantId?: string;
	redirectUri: string;
	projectId?: string;
}

export interface OAuthFlowStartResponse {
	flowId: string;
	authorizationUrl: string | null;
	userCode: string | null;
	verificationUri: string | null;
	message: string | null;
	intervalSecs: number | null;
	expiresInSecs: number;
}

export interface OAuthStatusResponse {
	flowId?: string;
	connectionId?: string;
	providerKind?: ProviderKind;
	status: string;
	message?: string | null;
	expiresAt?: number;
	updatedAt?: string;
}

export interface ConnectionConfigResponse {
	maxConcurrent: number;
	quotaWindowThresholds: { warning: number; error: number };
	defaultModel: string | null;
	baseUrl: string | null;
}

export type UsageAvailability = "available" | "no_data" | "unavailable";
export type CostAvailability =
	| "complete"
	| "partial"
	| "unknown"
	| "no_data"
	| "unavailable";
export type AccountQuotaAvailability = "not_provided" | "not_applicable";
export type LocalWarningLevel =
	| "ok"
	| "warning"
	| "critical"
	| "unknown"
	| "not_configured";

export interface ProviderQuotaWindowResponse {
	availability: UsageAvailability;
	rateLimitAvailability: "available" | "unavailable";
	requests: number | null;
	rateLimitedRequests: number | null;
	promptTokens: number | null;
	completionTokens: number | null;
	cacheReadTokens: number | null;
	cacheCreationTokens: number | null;
	reasoningTokens: number | null;
	totalTokens: number | null;
	costUsd: number | null;
	costAvailability: CostAvailability;
}

export interface ProviderQuotaTrendPointResponse {
	date: string;
	usageAvailability: UsageAvailability;
	requests: number | null;
	rateLimitedRequests: number | null;
	totalTokens: number | null;
	costUsd: number | null;
	costAvailability: CostAvailability;
}

export interface ProviderQuotaSummaryResponse {
	providerKind: ProviderKind;
	connectionCount: number;
	activeConnectionCount: number;
	coverage: "cortex_routed";
	usageAvailability: UsageAvailability;
	accountQuota: { availability: AccountQuotaAvailability };
	localThresholds: Array<{
		warning: number;
		error: number;
		connectionCount: number;
	}>;
	rateLimitObservation: {
		source: "recorded_routed_request_status";
		availability: "available" | "unavailable";
		window: "last_7d";
		rateLimitedRequests: number | null;
		requests: number | null;
		ratio: number | null;
	};
	localWarningLevel: LocalWarningLevel;
	allTime: ProviderQuotaWindowResponse;
	last24h: ProviderQuotaWindowResponse;
	last7d: ProviderQuotaWindowResponse;
	trend: ProviderQuotaTrendPointResponse[];
}

export interface ProvidersQuotaResponse {
	generatedAt: string;
	items: ProviderQuotaSummaryResponse[];
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
	const globalScope = globalThis as unknown as { __ROOK_API_BASE__?: string };
	if (globalScope.__ROOK_API_BASE__) {
		return globalScope.__ROOK_API_BASE__;
	}
	const stored =
		globalThis.localStorage !== undefined
			? globalThis.localStorage.getItem(STORAGE_KEY)
			: null;
	if (stored) return stored;
	// In development with Vite proxy, use relative URLs
	// The proxy handles forwarding to the backend
	if (import.meta.env.DEV) {
		return ""; // Relative URLs for dev proxy
	}
	// Auto-detect from current origin in production
	return globalThis.location?.origin ?? "http://127.0.0.1:3773";
}

export function setApiBaseUrl(url: string | null): void {
	if (url) {
		localStorage.setItem(STORAGE_KEY, url);
	} else {
		localStorage.removeItem(STORAGE_KEY);
	}
}

function queryString(query: UsageQuery): string {
	const params = new URLSearchParams();
	for (const [key, value] of Object.entries(query)) {
		if (value !== undefined && value !== null && value !== "") {
			params.set(key, String(value));
		}
	}
	const serialized = params.toString();
	return serialized ? `?${serialized}` : "";
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

		// Retry on CSRF failure with backoff: the cached token may be stale or
		// the cookie may not have landed in the jar yet (WebKit delayed
		// Set-Cookie described in issue #82). GET /login now reuses a valid
		// incoming cookie, so waiting briefly before refetching lets the jar
		// sync and avoids rotating to a new token. On a real authz error
		// (e.g. 403 from a route's permission check) the body will not mention
		// csrf_missing or csrf_mismatch, so we leave the response alone.
		for (let attempt = 0; attempt < 2; attempt++) {
			if (!isStateChanging || response.status !== 403) break;
			const probe = await response
				.clone()
				.text()
				.catch(() => "");
			if (!probe.includes("csrf_missing") && !probe.includes("csrf_mismatch"))
				break;
			cachedCsrfToken = null;
			try {
				await new Promise((resolve) =>
					setTimeout(resolve, 150 * (attempt + 1)),
				);
				headers["X-CSRF-Token"] = await getCsrfToken();
				response = await fetch(url, {
					...options,
					headers,
					credentials: "include",
				});
			} catch (err) {
				console.warn("[Rook API] CSRF token refresh failed:", err);
				// Fall through to the error path with the 403 response
				break;
			}
		}

		if (!response.ok) {
			const error = await response.text().catch(() => "Unknown error");
			throw new Error(`API Error ${response.status}: ${error}`);
		}

		// Handle successful responses without a body. The model-alias create
		// endpoint returns 201 Created with no body; DELETE endpoints return 204.
		if (
			response.status === 204 ||
			(response.status === 201 && response.body === null)
		) {
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
		async getApiKeyCount(): Promise<number> {
			const response = await request<ListApiKeysResponse>(
				"/api/api-keys?limit=1&offset=0",
			);
			return response.pagination.total;
		},

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
			return { apiKey: response.api_key };
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
				headers: { "X-CSRF-Token": csrfToken },
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

		async startOAuthFlow(
			provider: ProviderKind,
			data: OAuthFlowStartRequest,
		): Promise<OAuthFlowStartResponse> {
			return request<OAuthFlowStartResponse>(
				`/api/oauth/${provider}/authorize`,
				{
					method: "POST",
					body: JSON.stringify(data),
				},
			);
		},

		async getOAuthStatus(
			provider: ProviderKind,
			params: { flowId?: string; connectionId?: string },
		): Promise<OAuthStatusResponse> {
			const query = new URLSearchParams();
			if (params.flowId) query.set("flowId", params.flowId);
			if (params.connectionId) query.set("connectionId", params.connectionId);
			return request<OAuthStatusResponse>(
				`/api/oauth/${provider}/status?${query.toString()}`,
			);
		},

		async refreshOAuth(
			provider: ProviderKind,
			connectionId: string,
		): Promise<void> {
			return request<void>(`/api/oauth/${provider}/refresh`, {
				method: "POST",
				body: JSON.stringify({ connectionId }),
			});
		},

		async disconnectOAuth(
			provider: ProviderKind,
			connectionId: string,
		): Promise<void> {
			return request<void>(`/api/oauth/${provider}`, {
				method: "DELETE",
				body: JSON.stringify({ connectionId }),
			});
		},

		async getProvidersQuota(): Promise<ProvidersQuotaResponse> {
			return request<ProvidersQuotaResponse>("/api/providers/quota");
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

		// Combo management (requires session auth)
		async getCombos(): Promise<ComboListResponse> {
			return request<ComboListResponse>("/api/combos");
		},

		async getCombo(id: string): Promise<ComboResponse> {
			return request<ComboResponse>(`/api/combos/${id}`);
		},

		async createCombo(data: CreateComboRequest): Promise<ComboResponse> {
			return request<ComboResponse>("/api/combos", {
				method: "POST",
				body: JSON.stringify(data),
			});
		},

		async updateCombo(
			id: string,
			data: UpdateComboRequest,
		): Promise<ComboResponse> {
			return request<ComboResponse>(`/api/combos/${id}`, {
				method: "PUT",
				body: JSON.stringify(data),
			});
		},

		async deleteCombo(id: string): Promise<void> {
			return request<void>(`/api/combos/${encodeURIComponent(id)}`, {
				method: "DELETE",
			});
		},

		async getResilience(): Promise<ResilienceResponse> {
			return request<ResilienceResponse>("/api/resilience");
		},

		async resetProviderResilience(
			provider: string,
		): Promise<{ reset: boolean }> {
			return request<{ reset: boolean }>(
				`/api/resilience/${encodeURIComponent(provider)}/reset`,
				{ method: "POST" },
			);
		},

		async getTelemetrySummary(): Promise<TelemetrySummaryResponse> {
			return request<TelemetrySummaryResponse>("/api/telemetry/summary");
		},

		async getProviderTelemetry(
			provider: string,
		): Promise<ProviderTelemetryResponse> {
			return request<ProviderTelemetryResponse>(
				`/api/telemetry/${encodeURIComponent(provider)}`,
			);
		},

		async getProviderLatency(
			provider: string,
		): Promise<PercentileStatsResponse> {
			return request<PercentileStatsResponse>(
				`/api/telemetry/${encodeURIComponent(provider)}/latency`,
			);
		},

		async getCacheStats(): Promise<CacheStatsResponse> {
			return request<CacheStatsResponse>("/api/cache/stats");
		},

		async getCacheSignatures(): Promise<CacheSignatureResponse[]> {
			return request<CacheSignatureResponse[]>("/api/cache/signatures");
		},

		async getCacheSignature(
			signature: string,
		): Promise<CachedCompletionResponse> {
			return request<CachedCompletionResponse>(
				`/api/cache/signature/${encodeURIComponent(signature)}`,
			);
		},

		async clearCache(): Promise<void> {
			return request<void>("/api/cache", { method: "DELETE" });
		},

		async deleteCacheSignature(signature: string): Promise<void> {
			return request<void>(`/api/cache/${encodeURIComponent(signature)}`, {
				method: "DELETE",
			});
		},

		async getRateLimits(): Promise<RateLimitRuleResponse[]> {
			return request<RateLimitRuleResponse[]>("/api/rate-limits");
		},

		async createRateLimit(
			data: CreateRateLimitRequest,
		): Promise<RateLimitRuleResponse> {
			return request<RateLimitRuleResponse>("/api/rate-limits", {
				method: "POST",
				body: JSON.stringify(data),
			});
		},

		async updateRateLimit(
			id: string,
			data: Partial<
				Pick<CreateRateLimitRequest, "requests_per_minute" | "requests_per_day">
			>,
		): Promise<RateLimitRuleResponse> {
			return request<RateLimitRuleResponse>(
				`/api/rate-limits/${encodeURIComponent(id)}`,
				{ method: "PUT", body: JSON.stringify(data) },
			);
		},

		async deleteRateLimit(id: string): Promise<void> {
			return request<void>(`/api/rate-limits/${encodeURIComponent(id)}`, {
				method: "DELETE",
			});
		},

		async getRateLimitStatus(
			scope: RateLimitScope,
			target: string,
		): Promise<RateLimitStatusResponse> {
			const pathScope =
				scope === "api-key"
					? "api_key"
					: scope === "ip-address"
						? "ip_address"
						: scope;
			return request<RateLimitStatusResponse>(
				`/api/rate-limits/${pathScope}/${encodeURIComponent(target)}/status`,
			);
		},

		async getUsage(query: UsageQuery = {}): Promise<UsageListResponse> {
			return request<UsageListResponse>(`/api/usage${queryString(query)}`);
		},

		async getUsageSummary(
			query: UsageQuery = {},
		): Promise<UsageSummaryResponse> {
			return request<UsageSummaryResponse>(
				`/api/usage/summary${queryString(query)}`,
			);
		},

		async getUsageCost(query: UsageQuery = {}): Promise<UsageCostResponse> {
			return request<UsageCostResponse>(`/api/usage/cost${queryString(query)}`);
		},

		async getModelAliases(): Promise<ModelAliasResponse[]> {
			return request<ModelAliasResponse[]>("/api/models/aliases");
		},

		async createModelAlias(data: CreateModelAliasRequest): Promise<void> {
			return request<void>("/api/models/aliases", {
				method: "POST",
				body: JSON.stringify(data),
			});
		},

		async deleteModelAlias(alias: string): Promise<void> {
			return request<void>(`/api/models/aliases/${encodeURIComponent(alias)}`, {
				method: "DELETE",
			});
		},

		async getComboAliases(comboId: string): Promise<ComboAliasResponse[]> {
			const response = await request<ComboAliasWireResponse[]>(
				`/api/combos/${encodeURIComponent(comboId)}/aliases`,
			);
			return response.map(({ alias, combo_id }) => ({
				alias,
				comboId: combo_id,
			}));
		},

		async createComboAlias(
			comboId: string,
			alias: string,
		): Promise<ComboAliasResponse> {
			const response = await request<ComboAliasWireResponse>(
				`/api/combos/${encodeURIComponent(comboId)}/aliases`,
				{
					method: "POST",
					body: JSON.stringify({ alias }),
				},
			);
			return { alias: response.alias, comboId: response.combo_id };
		},

		async deleteComboAlias(comboId: string, alias: string): Promise<void> {
			return request<void>(
				`/api/combos/${encodeURIComponent(comboId)}/aliases/${encodeURIComponent(alias)}`,
				{ method: "DELETE" },
			);
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
	oauthClientId?: string;
	oauthClientSecret?: string;
	oauthTenantId?: string;
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
	apiClient ??= createApiClient();
	return apiClient;
}
