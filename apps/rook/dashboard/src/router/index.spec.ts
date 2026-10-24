import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RouteLocationNormalizedLoaded } from "vue-router";

type GuardResult = boolean | string | { name: string };

describe("auth router guard", () => {
	beforeEach(() => {
		setActivePinia(createPinia());
		vi.resetModules();
	});

	it("redirects protected routes to login when the session is missing", async () => {
		const { getAuthRedirect } = await import("./index");

		const result = getAuthRedirect(
			{
				name: "API Keys",
				meta: {},
				matched: [{ meta: { requiresAuth: true } }, { meta: {} }],
			} as unknown as RouteLocationNormalizedLoaded,
			false,
			false,
		);

		expect(result).toEqual({ name: "Login" });
	});

	it("redirects initialized login visits back home when already authenticated", async () => {
		const { getAuthRedirect } = await import("./index");

		const result: GuardResult = getAuthRedirect(
			{
				name: "Login",
				meta: { guestOnly: true },
				matched: [{ meta: { guestOnly: true } }],
			} as unknown as RouteLocationNormalizedLoaded,
			true,
			false,
		);

		expect(result).toEqual({ name: "Home" });
	});

	it("keeps unauthenticated users on login while bootstrap setup is required", async () => {
		const { getAuthRedirect } = await import("./index");

		const result = getAuthRedirect(
			{
				name: "Login",
				meta: { guestOnly: true },
				matched: [{ meta: { guestOnly: true } }],
			} as unknown as RouteLocationNormalizedLoaded,
			false,
			true,
		);

		expect(result).toBe(true);
	});

	it("allows access to routes that do not require auth or guestOnly", async () => {
		const { getAuthRedirect } = await import("./index");

		const result = getAuthRedirect(
			{
				name: "Public",
				meta: {},
				matched: [{ meta: {} }],
			} as unknown as RouteLocationNormalizedLoaded,
			false,
			false,
		);

		expect(result).toBe(true);
	});

	it("allows authenticated users through protected routes without redirecting", async () => {
		const { getAuthRedirect } = await import("./index");

		const result = getAuthRedirect(
			{
				name: "API Keys",
				meta: {},
				matched: [{ meta: { requiresAuth: true } }, { meta: {} }],
			} as unknown as RouteLocationNormalizedLoaded,
			true, // authenticated
			false,
		);

		expect(result).toBe(true);
	});

	it("keeps unauthenticated users on guest routes that do not require bootstrap", async () => {
		const { getAuthRedirect } = await import("./index");

		const result = getAuthRedirect(
			{
				name: "Login",
				meta: { guestOnly: true },
				matched: [{ meta: { guestOnly: true } }],
			} as unknown as RouteLocationNormalizedLoaded,
			false, // unauthenticated
			false, // bootstrap NOT required
		);

		expect(result).toBe(true);
	});

	it("validates providerKind param in route beforeEnter", async () => {
		const routerModule = await import("./index");
		const router = routerModule.default;

		const providerDetailsRoute = router
			.getRoutes()
			.find((r) => r.name === "Provider Details");

		expect(providerDetailsRoute).toBeDefined();
		expect(providerDetailsRoute?.beforeEnter).toBeDefined();

		const beforeEnter = providerDetailsRoute?.beforeEnter as (
			to: Partial<RouteLocationNormalizedLoaded>,
		) => unknown;

		// Valid provider kind
		const validResult = beforeEnter({
			params: { providerKind: "openai" },
		});
		expect(validResult).toBeUndefined();

		// Invalid provider kind
		const invalidResult = beforeEnter({
			params: { providerKind: "invalid-provider-kind" },
		});
		expect(invalidResult).toEqual({ name: "Providers" });

		// Non-string or missing provider kind
		const missingResult = beforeEnter({
			params: {},
		});
		expect(missingResult).toEqual({ name: "Providers" });
	});
});

describe("handleAuthNavigation", () => {
	it("calls loadBootstrapStatus on first navigation when not initialized", async () => {
		const { handleAuthNavigation } = await import("./index");

		const loadBootstrapStatus = vi.fn<() => Promise<void>>(() =>
			Promise.resolve(),
		);
		const to = {
			name: "Home",
			meta: {},
			matched: [{ meta: {} }],
		} as unknown as RouteLocationNormalizedLoaded;

		await handleAuthNavigation(to, {
			initialized: false,
			isAuthenticated: false,
			bootstrapRequired: true,
			loadBootstrapStatus,
		});

		expect(loadBootstrapStatus).toHaveBeenCalledOnce();
	});

	it("skips loadBootstrapStatus when auth store is already initialized", async () => {
		const { handleAuthNavigation } = await import("./index");

		const loadBootstrapStatus = vi.fn<() => Promise<void>>();
		const to = {
			name: "Home",
			meta: {},
			matched: [{ meta: {} }],
		} as unknown as RouteLocationNormalizedLoaded;

		await handleAuthNavigation(to, {
			initialized: true,
			isAuthenticated: false,
			bootstrapRequired: true,
			loadBootstrapStatus,
		});

		expect(loadBootstrapStatus).not.toHaveBeenCalled();
	});

	it("allows navigation when loadBootstrapStatus fails", async () => {
		const { handleAuthNavigation } = await import("./index");

		const warnSpy = vi.spyOn(console, "error").mockReturnValue();
		const loadBootstrapStatus = vi.fn<() => Promise<void>>(() =>
			Promise.reject(new Error("network error")),
		);
		const to = {
			name: "Login",
			meta: { guestOnly: true },
			matched: [{ meta: { guestOnly: true } }],
		} as unknown as RouteLocationNormalizedLoaded;

		const result = await handleAuthNavigation(to, {
			initialized: false,
			isAuthenticated: false,
			bootstrapRequired: false,
			loadBootstrapStatus,
		});

		// Navigation must proceed even after bootstrap failure
		expect(result).toBe(true);
		expect(warnSpy).toHaveBeenCalledOnce();
		warnSpy.mockRestore();
	});

	it("marks auth store as initialized after a failed loadBootstrapStatus", async () => {
		const { handleAuthNavigation } = await import("./index");

		vi.spyOn(console, "error").mockReturnValue();
		const loadBootstrapStatus = vi.fn<() => Promise<void>>(() =>
			Promise.reject(new Error("network error")),
		);
		const to = {
			name: "Home",
			meta: {},
			matched: [{ meta: {} }],
		} as unknown as RouteLocationNormalizedLoaded;
		const authStore = {
			initialized: false,
			isAuthenticated: false,
			bootstrapRequired: true,
			loadBootstrapStatus,
		};

		await handleAuthNavigation(to, authStore);

		expect(authStore.initialized).toBe(true);
	});
});

describe("router route definitions", () => {
	it("instantiates the full route tree and exercises all beforeEnter guards", async () => {
		// Accessing router.getRoutes() forces the router to evaluate the entire
		// route tree, which executes the lazy-loaded component imports and
		// registers the beforeEnter callbacks. This ensures those route-definition
		// lines are counted as executed by the coverage tool.
		const routerModule = await import("./index");
		const router = routerModule.default;

		const allRoutes = router.getRoutes();
		expect(allRoutes.length).toBeGreaterThan(0);

		// Provider Details — validates :providerKind param
		const providerDetails = allRoutes.find(
			(r) => r.name === "Provider Details",
		);
		expect(providerDetails?.beforeEnter).toBeDefined();
		const beforeEnter = providerDetails?.beforeEnter as (to: {
			params: Record<string, unknown>;
		}) => unknown;

		expect(beforeEnter({ params: { providerKind: "openai" } })).toBeUndefined();
		expect(beforeEnter({ params: { providerKind: "invalid-kind" } })).toEqual({
			name: "Providers",
		});
		expect(beforeEnter({ params: {} })).toEqual({ name: "Providers" });
	});
});
