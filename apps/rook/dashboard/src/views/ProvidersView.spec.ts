import { flushPromises, shallowMount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { createI18n } from "vue-i18n";
import en from "@/locales/en.json";
import ProvidersView from "./ProvidersView.vue";

const state = vi.hoisted(() => ({
	fetch: vi.fn(),
	loading: { value: false },
	error: { value: null as string | null },
}));
vi.mock("@/composables/useProviders", () => ({
	useProviders: () => ({
		fetch: vi.fn(),
		loading: ref(false),
		error: ref(null),
	}),
}));
vi.mock("@/composables/useAvailableModels", () => ({
	useAvailableModels: () => ({ fetch: vi.fn() }),
}));
vi.mock("@/composables/useProviderCatalog", () => ({
	useProviderCatalog: () => ({
		items: ref([]),
		byCategory: () => ref([]),
		fetch: state.fetch,
		loading: ref(state.loading.value),
		error: ref(state.error.value),
	}),
}));

function mountView() {
	return shallowMount(ProvidersView, {
		global: {
			plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })],
		},
	});
}
describe("ProvidersView descriptor requests", () => {
	beforeEach(() => {
		state.loading.value = false;
		state.error.value = null;
		state.fetch.mockReset();
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
});
