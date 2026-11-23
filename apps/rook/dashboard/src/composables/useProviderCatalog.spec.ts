/**
 * useProviderCatalog — joins the static catalog with the live connection
 * and model lists.
 *
 * The composable is read-only: it derives one item per provider kind with
 * the connection count, the "has active connections" flag and the models
 * configured for that kind. These tests pin that derivation, including
 * the model de-duplication and the category / kind lookups.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import type { ProviderConnectionResponse } from "@/lib/api";
import { useProviderCatalog } from "./useProviderCatalog";

const providers = ref<ProviderConnectionResponse[]>([]);
const modelsByProvider = ref<
	Array<{ provider: { providerKind: string }; models: string[] }>
>([]);
const descriptorCatalog = ref<Record<string, unknown>[]>([]);
const descriptorFetch = vi.fn();

vi.mock("@/composables/useProviders", () => ({
	useProviders: () => ({ providers }),
}));

vi.mock("@/composables/useAvailableModels", () => ({
	useAvailableModels: () => ({ modelsByProvider }),
}));

vi.mock("@/composables/useProviderDescriptors", () => ({
	useProviderDescriptors: () => ({
		catalog: descriptorCatalog,
		fetch: descriptorFetch,
		loading: ref(false),
		error: ref<string | null>(null),
	}),
}));

function makeDescriptor(overrides: Record<string, unknown> = {}) {
	return {
		kind: "openai",
		category: "api-key",
		displayNameKey: "providers.kind.openai.name",
		descriptionKey: "providers.kind.openai.description",
		defaultBaseUrl: "https://api.openai.com/v1",
		authTypes: ["apikey"],
		label: "OpenAI",
		iconKey: "openai",
		...overrides,
	};
}

function makeConnection(
	overrides: Partial<ProviderConnectionResponse> = {},
): ProviderConnectionResponse {
	return {
		id: "conn-1",
		providerKind: "openai",
		providerRuntimeId: "openai",
		authType: "apiKey",
		name: "Prod",
		priority: 0,
		isActive: true,
		config: {},
		testStatus: { status: "active" },
		createdAt: "2026-01-01T00:00:00Z",
		updatedAt: "2026-01-01T00:00:00Z",
		...overrides,
	} as ProviderConnectionResponse;
}

beforeEach(() => {
	providers.value = [];
	modelsByProvider.value = [];
	descriptorCatalog.value = [makeDescriptor()];
	descriptorFetch.mockReset().mockResolvedValue(undefined);
});

describe("useProviderCatalog items", () => {
	it("returns one item per descriptor", () => {
		const catalog = useProviderCatalog();

		expect(catalog.items.value).toHaveLength(1);
		expect(catalog.items.value[0].kind).toBe("openai");
		expect(catalog.items.value[0].label).toBe("OpenAI");
	});

	it("counts the connections and flags the active ones", () => {
		providers.value = [
			makeConnection({ id: "a", isActive: true }),
			makeConnection({ id: "b", isActive: false }),
		];
		const catalog = useProviderCatalog();

		const item = catalog.items.value[0];
		expect(item.connectionCount).toBe(2);
		expect(item.hasActiveConnections).toBe(true);
	});

	it("flags a kind without active connections", () => {
		providers.value = [makeConnection({ isActive: false })];
		const catalog = useProviderCatalog();

		expect(catalog.items.value[0].hasActiveConnections).toBe(false);
	});

	it("collects the configured models per kind, de-duplicated and sorted", () => {
		modelsByProvider.value = [
			{ provider: { providerKind: "openai" }, models: ["gpt-5", "gpt-4"] },
			{ provider: { providerKind: "openai" }, models: ["gpt-5"] },
		];
		const catalog = useProviderCatalog();

		expect(catalog.items.value[0].configuredModels).toEqual(["gpt-4", "gpt-5"]);
	});

	it("leaves the model list empty for kinds without models", () => {
		const catalog = useProviderCatalog();

		expect(catalog.items.value[0].configuredModels).toEqual([]);
	});
});

describe("useProviderCatalog lookups", () => {
	beforeEach(() => {
		descriptorCatalog.value = [
			makeDescriptor(),
			makeDescriptor({
				kind: "ollama",
				category: "local",
				label: "Ollama",
			}),
		];
	});

	it("filters the items by category", () => {
		const catalog = useProviderCatalog();

		const local = catalog.byCategory("local").value;

		expect(local).toHaveLength(1);
		expect(local[0].kind).toBe("ollama");
	});

	it("returns an empty list for a category without items", () => {
		const catalog = useProviderCatalog();

		expect(catalog.byCategory("oauth").value).toHaveLength(0);
	});

	it("finds a single item by kind", () => {
		const catalog = useProviderCatalog();

		expect(catalog.byKind("ollama").value?.label).toBe("Ollama");
	});

	it("returns null for an unknown kind", () => {
		const catalog = useProviderCatalog();

		expect(catalog.byKind("anthropic" as never).value).toBeNull();
	});
});

describe("useProviderCatalog state", () => {
	it("delegates the fetch to the descriptor catalog", async () => {
		const catalog = useProviderCatalog();

		await catalog.fetch();

		expect(descriptorFetch).toHaveBeenCalledOnce();
	});
});
