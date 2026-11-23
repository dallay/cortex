/**
 * Router tests — lazy route loaders and the provider-details guard.
 *
 * The router module owns two pieces of behaviour that are easy to break
 * silently:
 *
 *   1. every route component is loaded lazily, so a broken import path
 *      only fails when the user opens that page;
 *   2. the `:providerKind` route validates the kind against the live
 *      descriptor catalog and bounces unknown kinds back to the catalog.
 *
 * These tests drive the real router with real navigations, so the lazy
 * loaders are exercised exactly as the application exercises them. Each
 * test gets a fresh router instance (the module registry is reset) so
 * that every navigation is a real one — a repeated navigation to the
 * current location is a no-op in vue-router and would silently skip
 * both the lazy loader and the route guard.
 *
 * The parent shell is stubbed because loading its UI tree would make this
 * router-focused suite exercise unrelated sidebar components. The route
 * record and its lazy loader remain real. Only the HTTP boundary and shell
 * view are replaced.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProviderDescriptorResponse } from "@/lib/api";

const apiMock = vi.hoisted(() => ({
	getBootstrapStatus: vi.fn(),
	getMe: vi.fn(),
	getProviderDescriptors: vi.fn(),
}));

vi.mock("@/views/sidebar/index.vue", () => ({
	default: { name: "SidebarShellStub", template: "<div />" },
}));

vi.mock("@/lib/api", async (importOriginal) => {
	const actual = await importOriginal<typeof import("@/lib/api")>();
	return { ...actual, useApi: () => apiMock };
});

const openaiDescriptor: ProviderDescriptorResponse = {
	id: "openai",
	label: "OpenAI",
	category: "api-key",
	protocol: "openai",
	authKinds: ["apiKey"],
	capabilityCeiling: {
		tools: "yes",
		vision: "yes",
		reasoning: "yes",
		streaming: "yes",
		contextTokens: 128000,
	},
	iconKey: "openai",
	supportsOAuth: false,
};

/** Route table under test: path → expected route name. */
const LAZY_ROUTES: ReadonlyArray<readonly [string, string]> = [
	["/", "Home"],
	["/endpoints", "Endpoints"],
	["/api-keys", "API Keys"],
	["/api-keys/new", "API Keys Create"],
	["/providers", "Providers"],
	["/providers/quota", "Providers Quota"],
	["/combos", "Combos"],
	["/model-aliases", "Model Aliases"],
	["/usage", "Usage"],
	["/rate-limits", "Rate Limits"],
	["/cache", "Cache"],
	["/resilience", "Resilience"],
	["/telemetry", "Telemetry"],
	["/settings", "Settings"],
];

type RouterInstance = typeof import("./index")["default"];
type AuthStore = ReturnType<typeof import("@/stores/auth")["useAuthStore"]>;
type DescriptorStore = ReturnType<
	typeof import("@/composables/useProviderDescriptors")["useProviderDescriptors"]
>;

let router: RouterInstance;
let auth: AuthStore;
let descriptors: DescriptorStore;

async function bootRouter() {
	// A fresh module registry gives every test a fresh router, a fresh
	// Pinia instance and a fresh descriptor cache.
	vi.resetModules();
	const pinia = await import("pinia");
	const routerModule = await import("./index");
	const authModule = await import("@/stores/auth");
	const descriptorModule = await import("@/composables/useProviderDescriptors");

	pinia.setActivePinia(pinia.createPinia());
	router = routerModule.default;
	auth = authModule.useAuthStore();
	descriptors = descriptorModule.useProviderDescriptors();

	apiMock.getBootstrapStatus
		.mockReset()
		.mockResolvedValue({ isInitialized: true, adminUserExists: true });
	apiMock.getMe.mockReset().mockResolvedValue(null);
	apiMock.getProviderDescriptors
		.mockReset()
		.mockResolvedValue([openaiDescriptor]);

	descriptors.descriptors.value = [];
	descriptors.error.value = null;
}

function signIn() {
	auth.initialized = true;
	auth.currentUser = {
		username: "admin",
		displayName: "Rook Admin",
		email: "admin@rook.local",
	};
}

function signOut() {
	auth.initialized = true;
	auth.currentUser = null;
}

/** The deepest matched record of the current route. */
function activeRecord() {
	const matched = router.currentRoute.value.matched;
	return matched[matched.length - 1];
}

beforeEach(async () => {
	await bootRouter();
});

describe("router lazy loaders", () => {
	it("loads the login view for the guest-only route", async () => {
		signOut();
		await router.push("/login");

		expect(router.currentRoute.value.name).toBe("Login");
		expect(activeRecord().mods.default).toBeDefined();
	});

	it("redirects an authenticated visitor away from the login route", async () => {
		signIn();
		await router.push("/login");

		expect(router.currentRoute.value.name).toBe("Home");
	});

	it("loads the sidebar shell and the home view for the root route", async () => {
		signIn();
		await router.push("/");

		const matched = router.currentRoute.value.matched;
		expect(router.currentRoute.value.name).toBe("Home");
		// The shell route (unnamed) plus the home child route.
		expect(matched).toHaveLength(2);
		for (const record of matched) {
			expect(record.mods.default).toBeDefined();
		}
	});

	for (const [path, name] of LAZY_ROUTES) {
		it(`loads the view behind "${path}"`, async () => {
			signIn();
			await router.push(path);

			expect(router.currentRoute.value.name).toBe(name);
			expect(activeRecord().mods.default).toBeDefined();
		});
	}
});

describe("provider details guard", () => {
	it("loads the details view for a known provider kind", async () => {
		signIn();
		await router.push("/providers/openai");

		expect(router.currentRoute.value.name).toBe("Provider Details");
		expect(router.currentRoute.value.params.providerKind).toBe("openai");
		expect(activeRecord().mods.default).toBeDefined();
	});

	it("fetches the descriptor catalog when it is still empty", async () => {
		signIn();
		await router.push("/providers/openai");

		expect(apiMock.getProviderDescriptors).toHaveBeenCalledOnce();
	});

	it("does not refetch the catalog when it is already populated", async () => {
		descriptors.descriptors.value = [openaiDescriptor];
		signIn();
		await router.push("/providers/openai");

		expect(apiMock.getProviderDescriptors).not.toHaveBeenCalled();
	});

	it("redirects an unknown provider kind back to the catalog", async () => {
		signIn();
		await router.push("/providers/not-a-provider");

		expect(router.currentRoute.value.name).toBe("Providers");
		expect(router.currentRoute.value.path).toBe("/providers");
	});

	it("still renders the details view when the catalog request fails", async () => {
		apiMock.getProviderDescriptors.mockRejectedValue(new Error("boom"));
		signIn();
		await router.push("/providers/openai");

		expect(router.currentRoute.value.name).toBe("Provider Details");
		expect(descriptors.error.value).toBe("boom");
	});
});
