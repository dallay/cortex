import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createI18n } from "vue-i18n";
import en from "@/locales/en.json";
import HomeView from "./HomeView.vue";

const getHealth = vi.fn();
const getUsageSummary = vi.fn();
const getUsageCost = vi.fn();
const getApiKeyCount = vi.fn();
vi.mock("@/lib/api", () => ({
	useApi: () => ({ getHealth, getUsageSummary, getUsageCost, getApiKeyCount }),
}));
const homeMetrics = vi.hoisted(() => ({ unavailable: false }));
vi.mock("@/composables/useHomeMetrics", async () => {
	const { ref } = await import("vue");
	return {
		useHomeMetrics: () => ({
			apiKeyCount: ref(homeMetrics.unavailable ? null : 2),
			usage: ref(
				homeMetrics.unavailable
					? null
					: {
							total_requests: 14,
							total_prompt_tokens: 100,
							total_completion_tokens: 50,
						},
			),
			cost: ref(
				homeMetrics.unavailable
					? null
					: {
							total_cost_usd: 1.25,
							known_cost_entries: 2,
							unknown_cost_entries: 0,
						},
			),
			loading: ref(false),
			fetch: vi.fn(),
		}),
	};
});
const healthState = vi.hoisted(() => ({ unavailable: false }));
vi.mock("@/composables/useHealth", () => ({
	useHealth: () => ({
		data: healthState.unavailable
			? null
			: {
					status: "healthy",
					providers: [
						{ id: "openai", healthy: true, latency_ms: 82, last_error: null },
					],
				},
		loading: false,
		error: null,
		averageLatency: healthState.unavailable ? null : 82,
		healthyProviders: healthState.unavailable ? [] : [{ id: "openai" }],
	}),
}));
vi.mock("vue-router", () => ({ RouterLink: { template: "<a><slot /></a>" } }));
const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });
describe("HomeView", () => {
	beforeEach(() => {
		homeMetrics.unavailable = false;
	});
	it("renders backend health and usage summaries as real metrics", async () => {
		const wrapper = mount(HomeView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("2");
		expect(wrapper.text()).toContain("14");
		expect(wrapper.text()).toContain("$1.25");
		expect(wrapper.text()).toContain("82");
	});
	it("renders explicit unavailable text instead of ambiguous dashes", async () => {
		homeMetrics.unavailable = true;
		healthState.unavailable = true;
		const wrapper = mount(HomeView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("Unavailable");
		expect(wrapper.text()).not.toContain("/ 0");
	});
});
