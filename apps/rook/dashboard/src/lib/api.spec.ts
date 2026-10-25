import { beforeEach, describe, expect, it, vi } from "vitest";

describe("Rook auth API client", () => {
	beforeEach(() => {
		vi.restoreAllMocks();
		vi.unstubAllGlobals();
		localStorage.clear();
		Object.defineProperty(document, "cookie", {
			configurable: true,
			value: "",
		});
	});

	it("fetches a CSRF token before logging in and echoes it in the login header", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ csrf_token: "csrf-from-login" }), {
					status: 200,
					headers: { "Content-Type": "application/json" },
				}),
			)
			.mockResolvedValueOnce(
				new Response(
					JSON.stringify({
						session_id: "session-1",
						expires_at: "2026-01-01T00:00:00Z",
					}),
					{
						status: 200,
						headers: { "Content-Type": "application/json" },
					},
				),
			);

		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		const { useApi } = await import("./api");

		const result = await useApi().login({
			username: "admin",
			password: "test-fixture-password",
		});

		expect(result.sessionId).toBe("session-1");
		expect(fetchMock).toHaveBeenNthCalledWith(
			1,
			"/login",
			expect.objectContaining({
				credentials: "include",
			}),
		);
		expect(fetchMock).toHaveBeenNthCalledWith(
			2,
			"/login",
			expect.objectContaining({
				method: "POST",
				credentials: "include",
				headers: expect.objectContaining({
					"Content-Type": "application/json",
					"X-CSRF-Token": "csrf-from-login",
				}),
				body: JSON.stringify({
					username: "admin",
					password: "test-fixture-password",
				}),
			}),
		);
	});

	it("fetches bootstrap status from the public bootstrap endpoint", async () => {
		const fetchMock = vi.fn().mockResolvedValueOnce(
			new Response(
				JSON.stringify({
					is_initialized: false,
					admin_user_exists: true,
					// SECURITY: setup_token must NEVER appear in the status response body.
					// The token is out-of-band only (printed to server logs at startup).
				}),
				{
					status: 200,
					headers: { "Content-Type": "application/json" },
				},
			),
		);

		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		const { useApi } = await import("./api");

		const status = await useApi().getBootstrapStatus();

		// setup_token must not be present in the response — it is an out-of-band
		// secret only printed to server logs. Exposing it via HTTP would allow
		// unauthenticated remote takeover of fresh installations.
		expect(status).toEqual({
			isInitialized: false,
			adminUserExists: true,
		});
		expect(status).not.toHaveProperty("setupToken");
		expect(fetchMock).toHaveBeenCalledWith(
			"/api/bootstrap/status",
			expect.objectContaining({
				credentials: "include",
			}),
		);
	});

	it("uses a CSRF token when submitting the first-time admin password", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ csrf_token: "csrf-for-setup" }), {
					status: 200,
					headers: { "Content-Type": "application/json" },
				}),
			)
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ api_key: "rk_admin_initial" }), {
					status: 200,
					headers: { "Content-Type": "application/json" },
				}),
			);

		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		const { useApi } = await import("./api");

		const result = await useApi().setupBootstrap({
			setupToken: "setup-token",
			password: "test-fixture-password",
		});

		expect(result.apiKey).toBe("rk_admin_initial");
		expect(fetchMock).toHaveBeenNthCalledWith(
			2,
			"/api/bootstrap/setup",
			expect.objectContaining({
				method: "POST",
				credentials: "include",
				headers: expect.objectContaining({
					"Content-Type": "application/json",
					"X-CSRF-Token": "csrf-for-setup",
				}),
				body: JSON.stringify({
					setup_token: "setup-token",
					password: "test-fixture-password",
				}),
			}),
		);
	});

	it("fetches available model groups from /api/models with session credentials", async () => {
		const fetchMock = vi.fn().mockResolvedValueOnce(
			new Response(
				JSON.stringify({
					models: [
						{
							providerId: "00000000-0000-0000-0000-000000000001",
							providerName: "OpenAI Primary",
							providerKind: "openai",
							models: ["gpt-4o", "gpt-4-turbo"],
						},
						{
							providerId: "00000000-0000-0000-0000-000000000002",
							providerName: "Anthropic Primary",
							providerKind: "anthropic",
							models: ["claude-3-5-sonnet-latest"],
						},
					],
				}),
				{
					status: 200,
					headers: { "Content-Type": "application/json" },
				},
			),
		);

		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		const { useApi } = await import("./api");

		const result = await useApi().getAvailableModels();

		expect(result.models).toHaveLength(2);
		expect(result.models[0]?.providerId).toBe(
			"00000000-0000-0000-0000-000000000001",
		);
		expect(result.models[0]?.providerName).toBe("OpenAI Primary");
		expect(result.models[0]?.providerKind).toBe("openai");
		expect(result.models[0]?.models).toEqual(["gpt-4o", "gpt-4-turbo"]);
		expect(result.models[1]?.models).toEqual(["claude-3-5-sonnet-latest"]);
		expect(fetchMock).toHaveBeenCalledWith(
			"/api/models",
			expect.objectContaining({
				credentials: "include",
			}),
		);
	});

	it("returns an empty models list when no providers are configured", async () => {
		const fetchMock = vi.fn().mockResolvedValueOnce(
			new Response(JSON.stringify({ models: [] }), {
				status: 200,
				headers: { "Content-Type": "application/json" },
			}),
		);

		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		const { useApi } = await import("./api");

		const result = await useApi().getAvailableModels();

		expect(result.models).toEqual([]);
	});

	// Regression: WebKit (Safari) cookie-jar race. Each state-changing request
	// re-fetched GET /login, and Safari sometimes registered the new csrf_token
	// cookie after the body had already been read but before the next fetch
	// fired. Caching the token in memory collapses the per-request round-trip
	// and removes the race entirely. See issue #82.
	it("caches the CSRF token across state-changing requests", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ csrf_token: "csrf-cached" }), {
					status: 200,
					headers: { "Content-Type": "application/json" },
				}),
			)
			.mockResolvedValueOnce(
				new Response(
					JSON.stringify({
						session_id: "s1",
						expires_at: "2026-01-01T00:00:00Z",
					}),
					{
						status: 200,
						headers: { "Content-Type": "application/json" },
					},
				),
			)
			.mockResolvedValueOnce(
				new Response(
					JSON.stringify({
						session_id: "s2",
						expires_at: "2026-01-01T00:00:00Z",
					}),
					{
						status: 200,
						headers: { "Content-Type": "application/json" },
					},
				),
			);

		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		const { useApi } = await import("./api");

		await useApi().login({
			username: "admin",
			password: "test-fixture-password",
		});
		await useApi().login({
			username: "admin",
			password: "test-fixture-password",
		});

		// Exactly one GET /login across the two state-changing calls
		const getLoginCalls = fetchMock.mock.calls.filter(([url, init]) => {
			const method = (
				(init as RequestInit | undefined)?.method ?? "GET"
			).toUpperCase();
			return url === "/login" && method === "GET";
		});
		expect(getLoginCalls).toHaveLength(1);

		// Both POSTs must carry the same cached token
		const loginPost1 = fetchMock.mock.calls[1]?.[1] as RequestInit | undefined;
		const loginPost2 = fetchMock.mock.calls[2]?.[1] as RequestInit | undefined;
		expect(
			(loginPost1?.headers as Record<string, string> | undefined)?.[
				"X-CSRF-Token"
			],
		).toBe("csrf-cached");
		expect(
			(loginPost2?.headers as Record<string, string> | undefined)?.[
				"X-CSRF-Token"
			],
		).toBe("csrf-cached");
	});

	// Regression: even with the in-memory cache, the first request after
	// server-side token rotation or after a cookie-jar failure must retry with
	// a fresh token instead of propagating csrf_missing to the caller.
	it("refetches the CSRF token and retries when the backend returns csrf_missing", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ csrf_token: "csrf-stale" }), {
					status: 200,
					headers: { "Content-Type": "application/json" },
				}),
			)
			.mockResolvedValueOnce(
				new Response(
					JSON.stringify({
						error: "csrf_missing",
						message: "CSRF token required",
					}),
					{
						status: 403,
						headers: { "Content-Type": "application/json" },
					},
				),
			)
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ csrf_token: "csrf-fresh" }), {
					status: 200,
					headers: { "Content-Type": "application/json" },
				}),
			)
			.mockResolvedValueOnce(
				new Response(
					JSON.stringify({
						session_id: "s1",
						expires_at: "2026-01-01T00:00:00Z",
					}),
					{
						status: 200,
						headers: { "Content-Type": "application/json" },
					},
				),
			);

		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		const { useApi } = await import("./api");

		const result = await useApi().login({
			username: "admin",
			password: "test-fixture-password",
		});

		expect(result.sessionId).toBe("s1");
		// 1 CSRF fetch + 1 failing POST + 1 CSRF refetch + 1 retry POST = 4
		expect(fetchMock).toHaveBeenCalledTimes(4);

		// The retry must use the freshly fetched token, not the stale one
		const retryPost = fetchMock.mock.calls[3]?.[1] as RequestInit | undefined;
		expect(
			(retryPost?.headers as Record<string, string> | undefined)?.[
				"X-CSRF-Token"
			],
		).toBe("csrf-fresh");
	});

	// Regression: when the backend refuses to issue a CSRF token (network down,
	// 5xx, etc.) the request fails with csrf_missing. Previously the failure
	// was swallowed silently and surfaced only as a generic 403 in the dialog
	// — issue #82 lists "stop silently swallowing errors" as fix #4. We now
	// surface a console.warn so future regressions are visible in DevTools.
	it("logs a warning when the CSRF token fetch fails", async () => {
		const warnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});

		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				new Response("backend exploded", {
					status: 500,
					headers: { "Content-Type": "text/plain" },
				}),
			)
			.mockResolvedValueOnce(
				new Response(
					JSON.stringify({
						error: "csrf_missing",
						message: "CSRF token required",
					}),
					{
						status: 403,
						headers: { "Content-Type": "application/json" },
					},
				),
			)
			.mockResolvedValueOnce(
				new Response("backend still down", {
					status: 500,
					headers: { "Content-Type": "text/plain" },
				}),
			);

		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		const { useApi } = await import("./api");

		await expect(
			useApi().createApiKey({
				label: "test",
				scopes: ["chat:read"],
				tier: "Free",
				expiresAt: null,
			}),
		).rejects.toThrow(/csrf_missing/);

		// console.warn must have fired at least once with a CSRF-related message
		expect(warnSpy).toHaveBeenCalled();
		const messages = warnSpy.mock.calls.map((args) => String(args[0] ?? ""));
		expect(messages.some((m) => /CSRF/i.test(m))).toBe(true);

		warnSpy.mockRestore();
	});

	// Regression: after a successful login, the CSRF token from the login
	// response body must seed the cache so the very first state-changing
	// request after login has a cached token and never needs GET /login.
	// Without this, the page had to make GET /login after every login to
	// fetch the CSRF token, which races with the Set-Cookie landing in
	// WebKit's cookie jar and caused the 403 csrf_missing on the first
	// state-changing request. See issue #82 (suggested fix #2).
	it("seeds the CSRF cache from the login response body", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ csrf_token: "csrf-before-login" }), {
					status: 200,
					headers: { "Content-Type": "application/json" },
				}),
			)
			.mockResolvedValueOnce(
				new Response(
					JSON.stringify({
						session_id: "session-after-login",
						expires_at: "2026-01-01T00:00:00Z",
						csrf_token: "csrf-from-login-body", // ← the login response ALSO carries a token
					}),
					{
						status: 200,
						headers: { "Content-Type": "application/json" },
					},
				),
			)
			// Subsequent POST (createApiKey) — must NOT call GET /login again
			.mockResolvedValueOnce(
				new Response(
					JSON.stringify({
						key: { id: "k1", label: "test" },
						plaintextKey: "rk_test_abc",
					}),
					{
						status: 201,
						headers: { "Content-Type": "application/json" },
					},
				),
			);

		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		const { useApi } = await import("./api");

		await useApi().login({
			username: "admin",
			password: "test-fixture-password",
		});

		// Now make a state-changing request (no GET /login should fire)
		await useApi().createApiKey({
			label: "test",
			scopes: ["chat:read"],
			tier: "Free",
			expiresAt: null,
		});

		const getLoginCalls = fetchMock.mock.calls.filter(([url, init]) => {
			const method = (
				(init as RequestInit | undefined)?.method ?? "GET"
			).toUpperCase();
			return url === "/login" && method === "GET";
		});
		// Only the token-fetch for the login POST itself — NOT for the createApiKey
		expect(getLoginCalls).toHaveLength(1);
	});

	// The login response body carries a fresh csrf_token (the same one the
	// backend sets as the cookie). The cache must be seeded from the login
	// response body, NOT from the pre-login GET /login. This matters because
	// in WebKit, the cookie from POST /login's Set-Cookie may not be visible
	// to the next request — but if the cache holds the matching token from the
	// response body, the double-submit succeeds on the retry path. The cache
	// must therefore be OVERWRITTEN after a successful login, not just set
	// by the pre-login GET. See issue #82 (suggested fix #2).
	it("overwrites the cached CSRF token with the one from the login response body", async () => {
		const fetchMock = vi
			.fn()
			// Pre-login GET /login — returns token "csrf-before"
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ csrf_token: "csrf-before" }), {
					status: 200,
					headers: { "Content-Type": "application/json" },
				}),
			)
			// POST /login — returns a DIFFERENT token "csrf-from-login-body"
			.mockResolvedValueOnce(
				new Response(
					JSON.stringify({
						session_id: "s1",
						expires_at: "2026-01-01T00:00:00Z",
						csrf_token: "csrf-from-login-body", // ← backend sets THIS as the real cookie
					}),
					{
						status: 200,
						headers: { "Content-Type": "application/json" },
					},
				),
			)
			// Next state-changing call — must use "csrf-from-login-body", NOT "csrf-before"
			.mockResolvedValueOnce(
				new Response(JSON.stringify({ ok: true }), {
					status: 200,
					headers: { "Content-Type": "application/json" },
				}),
			);

		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		const { useApi } = await import("./api");

		await useApi().login({
			username: "admin",
			password: "test-fixture-password",
		});
		await useApi().logout();

		const postCall = fetchMock.mock.calls[2]?.[1] as RequestInit | undefined;
		expect(
			(postCall?.headers as Record<string, string> | undefined)?.[
				"X-CSRF-Token"
			],
		).toBe("csrf-from-login-body");
	});
});

describe("Rook OAuth API client", () => {
	beforeEach(() => {
		vi.restoreAllMocks();
		vi.unstubAllGlobals();
		localStorage.clear();
		Object.defineProperty(document, "cookie", {
			configurable: true,
			value: "",
		});
	});

	async function apiWithFetch(fetchMock: ReturnType<typeof vi.fn>) {
		vi.unstubAllGlobals();
		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		return (await import("./api")).useApi();
	}

	function okJson(payload: unknown) {
		return new Response(JSON.stringify(payload), {
			status: 200,
			headers: { "Content-Type": "application/json" },
		});
	}

	it("starts an OAuth flow against the provider authorize endpoint", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-1" }))
			.mockResolvedValueOnce(okJson({ flowId: "flow-1" }));
		const api = await apiWithFetch(fetchMock);
		const payload = {
			flowType: "authorizationCode",
			providerRuntimeId: "azure-prod",
			name: "Azure prod",
			priority: 10,
			isActive: true,
			config: { maxConcurrent: 10 },
			clientId: "client-id",
			redirectUri: "https://rook.example.com/api/oauth/callback",
		} as never;

		await api.startOAuthFlow("azure-openai", payload);

		expect(fetchMock).toHaveBeenCalledWith(
			"/api/oauth/azure-openai/authorize",
			expect.objectContaining({ method: "POST" }),
		);
	});

	it("stores and clears the API base URL override", async () => {
		vi.unstubAllGlobals();
		vi.resetModules();
		const { setApiBaseUrl } = await import("./api");

		setApiBaseUrl("https://stored.example.test");
		expect(localStorage.getItem("rook-api-base-url")).toBe(
			"https://stored.example.test",
		);

		setApiBaseUrl(null);
		expect(localStorage.getItem("rook-api-base-url")).toBeNull();
	});

	it("uses an explicit API base URL override for requests", async () => {
		vi.unstubAllGlobals();
		vi.stubGlobal("__ROOK_API_BASE__", "https://rook.example.test");
		const fetchMock = vi.fn().mockResolvedValueOnce(okJson({ healthy: true }));
		vi.stubGlobal("fetch", fetchMock);
		vi.resetModules();
		const api = (await import("./api")).useApi();

		await api.getHealth();

		expect(fetchMock).toHaveBeenCalledWith(
			"https://rook.example.test/health",
			expect.objectContaining({ credentials: "include" }),
		);
	});

	it("uses the stored API base URL when no runtime override is set", async () => {
		localStorage.setItem("rook-api-base-url", "https://stored.example.test");
		const fetchMock = vi.fn().mockResolvedValueOnce(okJson({ healthy: true }));
		const api = await apiWithFetch(fetchMock);

		await api.getHealth();

		expect(fetchMock).toHaveBeenCalledWith(
			"https://stored.example.test/health",
			expect.anything(),
		);
	});

	it("treats unauthorized current-user requests as a signed-out session", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(new Response("unauthorized", { status: 401 }));
		const api = await apiWithFetch(fetchMock);

		await expect(api.getMe()).resolves.toBeNull();
	});

	it("rethrows non-authentication errors from current-user requests", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				new Response("backend unavailable", { status: 503 }),
			);
		const api = await apiWithFetch(fetchMock);

		await expect(api.getMe()).rejects.toThrow(
			"API Error 503: backend unavailable",
		);
	});

	it("queries OAuth status by flow id only", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ status: "pending" }));
		const api = await apiWithFetch(fetchMock);

		await api.getOAuthStatus("gemini", { flowId: "flow-1" });

		expect(fetchMock).toHaveBeenCalledWith(
			"/api/oauth/gemini/status?flowId=flow-1",
			expect.anything(),
		);
	});

	it("queries OAuth status by connection id only", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ status: "connected" }));
		const api = await apiWithFetch(fetchMock);

		await api.getOAuthStatus("gemini", { connectionId: "conn-1" });

		expect(fetchMock).toHaveBeenCalledWith(
			"/api/oauth/gemini/status?connectionId=conn-1",
			expect.anything(),
		);
	});

	it("refreshes and disconnects OAuth connections", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-1" }))
			.mockResolvedValueOnce(new Response(null, { status: 204 }))
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		const api = await apiWithFetch(fetchMock);

		await api.refreshOAuth("azure-openai", "conn-1");
		await api.disconnectOAuth("azure-openai", "conn-1");

		expect(fetchMock).toHaveBeenNthCalledWith(
			2,
			"/api/oauth/azure-openai/refresh",
			expect.objectContaining({ method: "POST" }),
		);
		expect(fetchMock).toHaveBeenNthCalledWith(
			3,
			"/api/oauth/azure-openai",
			expect.objectContaining({ method: "DELETE" }),
		);
	});

	it("lists combos from /api/combos and parses the camelCase payload", async () => {
		const fetchMock = vi.fn().mockResolvedValueOnce(
			okJson({
				combos: [
					{
						id: "combo-1",
						name: "Production Fallback",
						strategy: "priority",
						steps: [
							{
								providerId: "openai",
								model: "gpt-4o",
								connectionId: null,
								priority: 1,
							},
						],
						createdAt: "2026-10-24T00:00:00Z",
						updatedAt: "2026-10-24T00:00:00Z",
					},
				],
			}),
		);
		const api = await apiWithFetch(fetchMock);

		const result = await api.getCombos();

		expect(result.combos).toHaveLength(1);
		const [combo] = result.combos;
		expect(combo?.strategy).toBe("priority");
		expect(combo?.steps[0]?.connectionId).toBeNull();
		expect(fetchMock).toHaveBeenCalledWith(
			"/api/combos",
			expect.objectContaining({ credentials: "include" }),
		);
	});

	it("fetches a single combo by id", async () => {
		const fetchMock = vi.fn().mockResolvedValueOnce(
			okJson({
				id: "combo-1",
				name: "Production Fallback",
				strategy: "priority",
				steps: [],
				createdAt: "2026-10-24T00:00:00Z",
				updatedAt: "2026-10-24T00:00:00Z",
			}),
		);
		const api = await apiWithFetch(fetchMock);

		const combo = await api.getCombo("combo-1");

		expect(combo.id).toBe("combo-1");
		expect(fetchMock).toHaveBeenCalledWith(
			"/api/combos/combo-1",
			expect.objectContaining({ credentials: "include" }),
		);
	});

	it("creates a combo with CSRF and the backend contract fields", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-combo" }))
			.mockResolvedValueOnce(
				okJson({
					id: "combo-2",
					name: "Fast Responses",
					strategy: "priority",
					steps: [],
					createdAt: "2026-10-24T00:00:00Z",
					updatedAt: "2026-10-24T00:00:00Z",
				}),
			);
		const api = await apiWithFetch(fetchMock);

		const payload = {
			name: "Fast Responses",
			strategy: "priority" as const,
			steps: [
				{
					providerId: "openai",
					model: "gpt-4o",
					connectionId: "conn-1",
					priority: 1,
				},
			],
		};
		const created = await api.createCombo(payload);

		expect(created.id).toBe("combo-2");
		expect(fetchMock).toHaveBeenNthCalledWith(
			2,
			"/api/combos",
			expect.objectContaining({
				method: "POST",
				credentials: "include",
				headers: expect.objectContaining({
					"X-CSRF-Token": "csrf-combo",
				}),
				body: JSON.stringify(payload),
			}),
		);
	});

	it("fetches usage summary and cost breakdown with query filters", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ total_requests: 42 }))
			.mockResolvedValueOnce(okJson({ total_cost_usd: 12.5 }));
		const api = await apiWithFetch(fetchMock);

		await api.getUsageSummary({
			start: "2026-10-01T00:00:00Z",
			provider: "openai",
		});
		await api.getUsageCost({ end: "2026-10-24T00:00:00Z" });

		expect(fetchMock).toHaveBeenNthCalledWith(
			1,
			"/api/usage/summary?start=2026-10-01T00%3A00%3A00Z&provider=openai",
			expect.anything(),
		);
		expect(fetchMock).toHaveBeenNthCalledWith(
			2,
			"/api/usage/cost?end=2026-10-24T00%3A00%3A00Z",
			expect.anything(),
		);
	});

	it("loads resilience state and resets an encoded provider with CSRF", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				okJson({
					circuit_states: [
						{
							provider: "openai",
							failures: 3,
							state: "open",
							last_failure: null,
							cooldown_until: null,
							rate_limit_reset: null,
						},
					],
				}),
			)
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-reset" }))
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		const api = await apiWithFetch(fetchMock);
		expect(await api.getResilience()).toEqual({
			circuit_states: [
				{
					provider: "openai",
					failures: 3,
					state: "open",
					last_failure: null,
					cooldown_until: null,
					rate_limit_reset: null,
				},
			],
		});
		await api.resetProviderResilience("provider/with space");
		expect(fetchMock).toHaveBeenNthCalledWith(
			1,
			"/api/resilience",
			expect.anything(),
		);
		expect(fetchMock).toHaveBeenNthCalledWith(
			3,
			"/api/resilience/provider%2Fwith%20space/reset",
			expect.objectContaining({
				method: "POST",
				headers: expect.objectContaining({ "X-CSRF-Token": "csrf-reset" }),
			}),
		);
	});

	it("loads provider telemetry and latency distributions", async () => {
		const summary = {
			providers: [
				{
					provider_id: "openai",
					total_requests: 12,
					success_count: 10,
					failure_count: 1,
					rate_limited_count: 1,
					latency_stats: {
						p50: 120,
						p95: 340.4,
						p99: 500,
						avg: 210,
						min: 50,
						max: 500,
						count: 12,
					},
					ttft_stats: null,
				},
			],
		};
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson(summary))
			.mockResolvedValueOnce(okJson(summary.providers[0]))
			.mockResolvedValueOnce(okJson(summary.providers[0]?.latency_stats));
		const api = await apiWithFetch(fetchMock);
		expect(await api.getTelemetrySummary()).toEqual(summary);
		await api.getProviderTelemetry("openai");
		await api.getProviderLatency("openai");
		expect(fetchMock).toHaveBeenNthCalledWith(
			1,
			"/api/telemetry/summary",
			expect.anything(),
		);
		expect(fetchMock).toHaveBeenNthCalledWith(
			2,
			"/api/telemetry/openai",
			expect.anything(),
		);
		expect(fetchMock).toHaveBeenNthCalledWith(
			3,
			"/api/telemetry/openai/latency",
			expect.anything(),
		);
	});

	it("loads cache statistics and signatures, clears cache, and deletes a signature", async () => {
		const signature = "a".repeat(64);
		const stats = {
			hits: 20,
			misses: 5,
			evictions: 1,
			entries: 1,
			max_entries: 100,
			token_cache: {
				hits: 2,
				misses: 1,
				tokens_saved: 300,
				estimated_cost_saved_usd: 0.05,
			},
		};
		const entry = {
			signature,
			request_id: "req-1",
			model: "gpt-4o",
			provider: "openai",
			cached_at: "2026-10-24T00:00:00Z",
			last_accessed: "2026-10-24T00:01:00Z",
			request_metadata: {
				message_count: 2,
				has_tools: false,
				max_tokens: null,
				temperature: null,
			},
		};
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson(stats))
			.mockResolvedValueOnce(okJson([entry]))
			.mockResolvedValueOnce(okJson(entry))
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-cache" }))
			.mockResolvedValueOnce(new Response(null, { status: 204 }))
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		const api = await apiWithFetch(fetchMock);
		expect(await api.getCacheStats()).toEqual(stats);
		expect(await api.getCacheSignatures()).toEqual([entry]);
		expect(await api.getCacheSignature(signature)).toEqual(entry);
		await api.clearCache();
		await api.deleteCacheSignature(signature);
		expect(fetchMock).toHaveBeenNthCalledWith(
			5,
			"/api/cache",
			expect.objectContaining({ method: "DELETE" }),
		);
		expect(fetchMock).toHaveBeenNthCalledWith(
			6,
			`/api/cache/${signature}`,
			expect.objectContaining({ method: "DELETE" }),
		);
	});

	it("lists, creates, updates, deletes, and queries rate limit rules", async () => {
		const rule = {
			id: "rl-1",
			scope: "api-key",
			target: "key-1",
			requests_per_minute: 60,
			requests_per_day: 1000,
			tokens_per_minute: null,
		};
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson([rule]))
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-rate-limit" }))
			.mockResolvedValueOnce(okJson(rule))
			.mockResolvedValueOnce(okJson({ ...rule, requests_per_minute: 90 }))
			.mockResolvedValueOnce(
				okJson({
					scope: "api-key",
					target: "key-1",
					current_minute_count: 2,
					current_day_count: 12,
					remaining_minute: 58,
					remaining_day: 988,
					reset_at: "2026-10-24T14:00:00Z",
					active_requests_per_minute: 60,
					active_requests_per_day: 1000,
				}),
			)
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		const api = await apiWithFetch(fetchMock);
		expect(await api.getRateLimits()).toEqual([rule]);
		await api.createRateLimit({
			scope: "api-key",
			target: "key-1",
			requests_per_minute: 60,
			requests_per_day: 1000,
		});
		await api.updateRateLimit("rl-1", { requests_per_minute: 90 });
		await api.getRateLimitStatus("api-key", "key-1");
		await api.deleteRateLimit("rl-1");
		expect(fetchMock).toHaveBeenNthCalledWith(
			3,
			"/api/rate-limits",
			expect.objectContaining({
				method: "POST",
				body: JSON.stringify({
					scope: "api-key",
					target: "key-1",
					requests_per_minute: 60,
					requests_per_day: 1000,
				}),
			}),
		);
		expect(fetchMock).toHaveBeenNthCalledWith(
			5,
			"/api/rate-limits/api_key/key-1/status",
			expect.anything(),
		);
		expect(fetchMock).toHaveBeenNthCalledWith(
			6,
			"/api/rate-limits/rl-1",
			expect.objectContaining({ method: "DELETE" }),
		);
	});

	it("classifies usage cost as unknown when no records have known costs", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ entries: [], total: 0 }))
			.mockResolvedValueOnce(
				okJson({
					total_requests: 0,
					total_prompt_tokens: 0,
					total_completion_tokens: 0,
					total_cache_read_tokens: 0,
					total_cache_creation_tokens: 0,
					total_reasoning_tokens: 0,
					avg_ttft_ms: null,
					avg_latency_ms: 0,
				}),
			)
			.mockResolvedValueOnce(
				okJson({
					total_cost_usd: 0,
					known_cost_entries: 0,
					unknown_cost_entries: 1,
					by_provider: {},
					by_model: {},
					by_api_key: {},
				}),
			);
		const api = await apiWithFetch(fetchMock);
		const [list, summary, cost] = await Promise.all([
			api.getUsage(),
			api.getUsageSummary(),
			api.getUsageCost(),
		]);
		expect(list.total).toBe(0);
		expect(summary.total_requests).toBe(0);
		expect(cost.unknown_cost_entries).toBe(1);
	});

	it("lists usage with filters and pagination", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ entries: [], total: 7 }));
		const api = await apiWithFetch(fetchMock);
		await api.getUsage({
			apiKeyId: "key-1",
			model: "gpt-4o",
			offset: 25,
			limit: 25,
		});
		expect(fetchMock).toHaveBeenCalledWith(
			"/api/usage?apiKeyId=key-1&model=gpt-4o&offset=25&limit=25",
			expect.anything(),
		);
	});

	it("lists model aliases using the camelCase backend DTO", async () => {
		const fetchMock = vi.fn().mockResolvedValueOnce(
			okJson([
				{
					alias: "chat-fast",
					canonical: "gpt-4o-mini",
					providerId: "openai",
					createdAt: "2026-10-24T00:00:00Z",
				},
			]),
		);
		const api = await apiWithFetch(fetchMock);

		const aliases = await api.getModelAliases();

		expect(aliases[0]?.providerId).toBe("openai");
		expect(fetchMock).toHaveBeenCalledWith(
			"/api/models/aliases",
			expect.objectContaining({ credentials: "include" }),
		);
	});

	it("creates a model alias with providerId and CSRF", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-model-alias" }))
			.mockResolvedValueOnce(new Response(null, { status: 201 }));
		const api = await apiWithFetch(fetchMock);

		await api.createModelAlias({
			alias: "chat-fast",
			canonical: "gpt-4o-mini",
			providerId: "openai",
		});

		expect(fetchMock).toHaveBeenNthCalledWith(
			2,
			"/api/models/aliases",
			expect.objectContaining({
				method: "POST",
				headers: expect.objectContaining({
					"X-CSRF-Token": "csrf-model-alias",
				}),
				body: JSON.stringify({
					alias: "chat-fast",
					canonical: "gpt-4o-mini",
					providerId: "openai",
				}),
			}),
		);
	});

	it("accepts an empty 201 response body backed by a stream", async () => {
		const emptyResponse = new Response(null, { status: 201 });
		Object.defineProperty(emptyResponse, "body", {
			value: new ReadableStream<Uint8Array>({
				start(controller) {
					controller.close();
				},
			}),
		});
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-model-alias" }))
			.mockResolvedValueOnce(emptyResponse);
		const api = await apiWithFetch(fetchMock);
		await expect(
			api.createModelAlias({
				alias: "chat-fast",
				canonical: "gpt-4o-mini",
				providerId: "openai",
			}),
		).resolves.toBeUndefined();
	});

	it("deletes an encoded model alias with CSRF", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-model-alias" }))
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		const api = await apiWithFetch(fetchMock);

		await api.deleteModelAlias("model/alias");

		expect(fetchMock).toHaveBeenNthCalledWith(
			2,
			"/api/models/aliases/model%2Falias",
			expect.objectContaining({
				method: "DELETE",
				headers: expect.objectContaining({
					"X-CSRF-Token": "csrf-model-alias",
				}),
			}),
		);
	});

	it("lists combo aliases as a raw array with snake_case combo_id", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				okJson([{ alias: "rook/coding", combo_id: "combo-1" }]),
			);
		const api = await apiWithFetch(fetchMock);

		const aliases = await api.getComboAliases("combo/1");

		expect(aliases).toEqual([{ alias: "rook/coding", comboId: "combo-1" }]);
		expect(fetchMock).toHaveBeenCalledWith(
			"/api/combos/combo%2F1/aliases",
			expect.objectContaining({ credentials: "include" }),
		);
	});

	it("creates a combo alias with CSRF and maps the backend response", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-alias" }))
			.mockResolvedValueOnce(
				okJson({ alias: "rook/coding", combo_id: "combo-1" }),
			);
		const api = await apiWithFetch(fetchMock);

		const created = await api.createComboAlias("combo-1", "rook/coding");

		expect(created).toEqual({ alias: "rook/coding", comboId: "combo-1" });
		expect(fetchMock).toHaveBeenNthCalledWith(
			2,
			"/api/combos/combo-1/aliases",
			expect.objectContaining({
				method: "POST",
				credentials: "include",
				headers: expect.objectContaining({ "X-CSRF-Token": "csrf-alias" }),
				body: JSON.stringify({ alias: "rook/coding" }),
			}),
		);
	});

	it("deletes an encoded combo alias with DELETE and CSRF", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-alias" }))
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		const api = await apiWithFetch(fetchMock);

		await api.deleteComboAlias("combo/1", "rook/coding");

		expect(fetchMock).toHaveBeenNthCalledWith(
			2,
			"/api/combos/combo%2F1/aliases/rook%2Fcoding",
			expect.objectContaining({
				method: "DELETE",
				headers: expect.objectContaining({ "X-CSRF-Token": "csrf-alias" }),
			}),
		);
	});

	it("updates and deletes a combo with the expected methods and paths", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(okJson({ csrf_token: "csrf-combo" }))
			.mockResolvedValueOnce(
				okJson({
					id: "combo-1",
					name: "Renamed",
					strategy: "priority",
					steps: [],
					createdAt: "2026-10-24T00:00:00Z",
					updatedAt: "2026-10-24T01:00:00Z",
				}),
			)
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		const api = await apiWithFetch(fetchMock);

		await api.updateCombo("combo-1", {
			name: "Renamed",
			strategy: "priority",
			steps: [],
		});
		await api.deleteCombo("combo-1");

		expect(fetchMock).toHaveBeenNthCalledWith(
			2,
			"/api/combos/combo-1",
			expect.objectContaining({ method: "PUT" }),
		);
		expect(fetchMock).toHaveBeenNthCalledWith(
			3,
			"/api/combos/combo-1",
			expect.objectContaining({ method: "DELETE" }),
		);
	});
});
