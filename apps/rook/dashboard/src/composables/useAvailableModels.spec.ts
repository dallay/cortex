import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick, ref } from "vue";
import type { ProviderConnectionResponse } from "@/lib/api";

// We mock the dependencies the composable relies on so the test can
// drive the inputs deterministically.
const mockProviders = ref<ProviderConnectionResponse[]>([]);
const mockGetAvailableModels = vi.fn();

function makeProvider(
	id: string,
	name: string,
	providerKind: ProviderConnectionResponse["providerKind"],
	isActive: boolean,
): ProviderConnectionResponse {
	return {
		id,
		name,
		providerKind,
		providerRuntimeId: id,
		authType: "apiKey",
		priority: 0,
		isActive,
		config: {
			maxConcurrent: 1,
			quotaWindowThresholds: { warning: 0.5, error: 0.9 },
			defaultModel: null,
			baseUrl: null,
		},
		testStatus: {
			status: "neverTested",
			lastTestAt: null,
			latencyMs: null,
			error: null,
		},
		createdAt: "2026-01-01T00:00:00Z",
		updatedAt: "2026-01-01T00:00:00Z",
	};
}

vi.mock("@/composables/useProviders", () => ({
	useProviders: () => ({
		providers: mockProviders,
		fetch: vi.fn(),
	}),
}));

vi.mock("@/lib/api", () => ({
	useApi: () => ({
		getAvailableModels: mockGetAvailableModels,
	}),
}));

describe("useAvailableModels", () => {
	beforeEach(() => {
		mockProviders.value = [];
		mockGetAvailableModels.mockReset();
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("starts with empty state and no fetch", () => {
		mockGetAvailableModels.mockResolvedValue({ models: [] });
		return import("./useAvailableModels").then(
			async ({ useAvailableModels }) => {
				const c = useAvailableModels();
				expect(c.modelsByProvider.value).toEqual([]);
				expect(c.loading.value).toBe(false);
				expect(c.error.value).toBeNull();
				expect(c.fetched.value).toBe(false);
				expect(mockGetAvailableModels).not.toHaveBeenCalled();
			},
		);
	});

	it("fetch() populates groups and sets fetched=true", async () => {
		mockGetAvailableModels.mockResolvedValue({
			models: [
				{
					providerId: "p1",
					providerName: "OpenAI Primary",
					providerKind: "openai",
					models: ["gpt-4o"],
				},
				{
					providerId: "p2",
					providerName: "Anthropic",
					providerKind: "anthropic",
					models: ["claude-3-5-sonnet-latest"],
				},
			],
		});
		mockProviders.value = [
			makeProvider("p1", "OpenAI Primary", "openai", true),
			makeProvider("p2", "Anthropic", "anthropic", true),
		];

		const { useAvailableModels } = await import("./useAvailableModels");
		const c = useAvailableModels();
		await c.fetch();
		await nextTick();

		expect(c.fetched.value).toBe(true);
		expect(c.loading.value).toBe(false);
		expect(c.error.value).toBeNull();
		expect(c.modelsByProvider.value).toHaveLength(2);
		expect(c.modelsByProvider.value[0]?.provider.id).toBe("p1");
		expect(c.modelsByProvider.value[0]?.models).toEqual(["gpt-4o"]);
		expect(c.modelsByProvider.value[1]?.provider.id).toBe("p2");
	});

	it("filters out inactive providers from the cross", async () => {
		mockGetAvailableModels.mockResolvedValue({
			models: [
				{
					providerId: "p1",
					providerName: "OpenAI Primary",
					providerKind: "openai",
					models: ["gpt-4o"],
				},
				{
					providerId: "p2",
					providerName: "OpenAI Inactive",
					providerKind: "openai",
					models: ["gpt-4o"],
				},
			],
		});
		mockProviders.value = [
			makeProvider("p1", "OpenAI Primary", "openai", true),
			makeProvider("p2", "OpenAI Inactive", "openai", false),
		];

		const { useAvailableModels } = await import("./useAvailableModels");
		const c = useAvailableModels();
		await c.fetch();
		await nextTick();

		// Only the active provider is in the cross.
		expect(c.modelsByProvider.value).toHaveLength(1);
		expect(c.modelsByProvider.value[0]?.provider.id).toBe("p1");
	});

	it("filters out groups with no models", async () => {
		mockGetAvailableModels.mockResolvedValue({
			models: [
				{
					providerId: "p1",
					providerName: "OpenAI",
					providerKind: "openai",
					models: ["gpt-4o"],
				},
				{
					providerId: "p2",
					providerName: "Empty",
					providerKind: "openai",
					models: [],
				},
			],
		});
		mockProviders.value = [
			makeProvider("p1", "OpenAI", "openai", true),
			makeProvider("p2", "Empty", "openai", true),
		];

		const { useAvailableModels } = await import("./useAvailableModels");
		const c = useAvailableModels();
		await c.fetch();
		await nextTick();

		expect(c.modelsByProvider.value).toHaveLength(1);
		expect(c.modelsByProvider.value[0]?.provider.id).toBe("p1");
	});

	it("surfaces errors and keeps groups empty", async () => {
		mockGetAvailableModels.mockRejectedValue(new Error("network down"));
		mockProviders.value = [makeProvider("p1", "OpenAI", "openai", true)];

		const { useAvailableModels } = await import("./useAvailableModels");
		const c = useAvailableModels();
		await c.fetch();
		await nextTick();

		expect(c.error.value).toBe("network down");
		expect(c.fetched.value).toBe(false);
		expect(c.modelsByProvider.value).toEqual([]);
	});

	it("cross handles missing group entry for active provider", async () => {
		// Server returned no group for p1, even though p1 is active.
		// The composable should not crash and should drop the provider.
		mockGetAvailableModels.mockResolvedValue({ models: [] });
		mockProviders.value = [makeProvider("p1", "OpenAI", "openai", true)];

		const { useAvailableModels } = await import("./useAvailableModels");
		const c = useAvailableModels();
		await c.fetch();
		await nextTick();

		expect(c.modelsByProvider.value).toEqual([]);
	});
});
