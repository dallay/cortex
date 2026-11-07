import { flushPromises, shallowMount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { createI18n } from "vue-i18n";
import { Button } from "@/components/ui/button";
import en from "@/locales/en.json";
import ProvidersView from "./ProvidersView.vue";

const state = vi.hoisted(() => ({
	fetch: vi.fn(),
	loading: { value: false },
	error: { value: null as string | null },
	providersFetch: vi.fn(),
	providersLoading: { value: false },
	providersError: { value: null as string | null },
	modelsFetch: vi.fn(),
}));

const mockItems = [
	{
		kind: "openai",
		category: "api-key",
		displayNameKey: "providers.kinds.openai",
		label: "OpenAI",
		descriptionKey: "providers.kinds.openaiDesc",
		capabilities: ["chat"],
	},
	{
		kind: "google",
		category: "oauth",
		displayNameKey: "providers.kinds.google",
		label: "Google",
		descriptionKey: "providers.kinds.googleDesc",
		capabilities: ["chat"],
	},
	{
		kind: "ollama",
		category: "local",
		displayNameKey: "providers.kinds.ollama",
		label: "Ollama",
		descriptionKey: "providers.kinds.ollamaDesc",
		capabilities: ["chat"],
	},
];

vi.mock("@/composables/useProviders", () => ({
	useProviders: () => ({
		fetch: state.providersFetch,
		loading: ref(state.providersLoading.value),
		error: ref(state.providersError.value),
	}),
}));
vi.mock("@/composables/useAvailableModels", () => ({
	useAvailableModels: () => ({ fetch: state.modelsFetch }),
}));
vi.mock("@/composables/useProviderCatalog", () => ({
	useProviderCatalog: () => ({
		items: ref(mockItems),
		byCategory: (cat: string) =>
			ref(mockItems.filter((i) => i.category === cat)),
		fetch: state.fetch,
		loading: ref(state.loading.value),
		error: ref(state.error.value),
	}),
}));

function mountView(options = {}) {
	return shallowMount(ProvidersView, {
		global: {
			plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })],
			stubs: {
				EmptyState: false,
				Empty: false,
				EmptyContent: false,
				EmptyHeader: false,
				EmptyTitle: false,
				EmptyDescription: false,
				EmptyMedia: false,
			},
		},
		...options,
	});
}
describe("ProvidersView descriptor requests", () => {
	beforeEach(() => {
		state.loading.value = false;
		state.error.value = null;
		state.providersLoading.value = false;
		state.providersError.value = null;
		state.fetch.mockReset();
		state.providersFetch.mockReset();
		state.modelsFetch.mockReset();
	});

	it("fetches providers, models, and descriptors on mount", () => {
		mountView();
		expect(state.providersFetch).toHaveBeenCalledOnce();
		expect(state.modelsFetch).toHaveBeenCalledOnce();
		expect(state.fetch).toHaveBeenCalledOnce();
	});

	it("shows loading while the descriptor request is pending", () => {
		state.loading.value = true;
		const wrapper = mountView();
		expect(wrapper.findComponent({ name: "LoadingState" }).exists()).toBe(true);
		expect(wrapper.findComponent({ name: "EmptyState" }).exists()).toBe(false);
	});

	it("shows the descriptor error and retries the descriptor request", async () => {
		state.error.value = "Descriptor request failed";
		const wrapper = mountView();
		const banner = wrapper.findComponent({ name: "ErrorBanner" });
		expect(banner.props("message")).toBe(state.error.value);
		expect(wrapper.findComponent({ name: "EmptyState" }).exists()).toBe(false);
		state.fetch.mockClear();
		banner.vm.$emit("retry");
		await flushPromises();
		expect(state.fetch).toHaveBeenCalledOnce();
	});

	it("shows provider error banner and retries provider fetch", async () => {
		state.providersError.value = "Failed to load providers";
		const wrapper = mountView();
		const banners = wrapper.findAllComponents({ name: "ErrorBanner" });
		expect(banners.length).toBeGreaterThan(0);
		banners[0].vm.$emit("retry");
		await flushPromises();
		expect(state.providersFetch).toHaveBeenCalledTimes(2); // once on mount, once on retry
	});

	it("renders sections and filters by category", async () => {
		const wrapper = mountView();
		const sections = wrapper.findAllComponents({
			name: "ProviderCategorySection",
		});
		expect(sections.length).toBe(3);

		const filter = wrapper.findComponent({ name: "ProviderCatalogFilter" });
		filter.vm.$emit("update:activeCategory", "api-key");
		await flushPromises();

		const filteredSections = wrapper.findAllComponents({
			name: "ProviderCategorySection",
		});
		expect(filteredSections.length).toBe(1);
	});

	it("filters items by search query and shows empty state when no matches found", async () => {
		const wrapper = mountView();
		const filter = wrapper.findComponent({ name: "ProviderCatalogFilter" });

		// Filter matching one item
		filter.vm.$emit("update:searchQuery", "openai");
		await flushPromises();
		expect(wrapper.findComponent({ name: "EmptyState" }).exists()).toBe(false);

		// Filter with no match
		filter.vm.$emit("update:searchQuery", "nonexistent-query");
		await flushPromises();
		const emptyState = wrapper.findComponent({ name: "EmptyState" });
		expect(emptyState.exists()).toBe(true);

		// Reset filter via button in empty state
		const resetButton = wrapper.findComponent(Button);
		expect(resetButton.exists()).toBe(true);
		await resetButton.trigger("click");
		await flushPromises();
		expect(wrapper.findComponent({ name: "EmptyState" }).exists()).toBe(false);
	});

	it("opens and closes AddProviderDialog on section add, saved, and deleted events", async () => {
		const wrapper = mountView();
		const section = wrapper.findComponent({ name: "ProviderCategorySection" });
		const dialog = wrapper.findComponent({ name: "AddProviderDialog" });

		expect(dialog.props("open")).toBe(false);

		// Open dialog
		section.vm.$emit("add", "openai");
		await flushPromises();
		expect(dialog.props("open")).toBe(true);
		expect(dialog.props("providerKind")).toBe("openai");

		// Close via saved
		dialog.vm.$emit("saved");
		await flushPromises();
		expect(dialog.props("open")).toBe(false);

		// Open and close via deleted
		section.vm.$emit("add", "google");
		await flushPromises();
		expect(dialog.props("open")).toBe(true);
		dialog.vm.$emit("deleted");
		await flushPromises();
		expect(dialog.props("open")).toBe(false);
	});
});
