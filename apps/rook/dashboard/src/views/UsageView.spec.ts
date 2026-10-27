import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createI18n } from "vue-i18n";
import en from "@/locales/en.json";
import UsageView from "./UsageView.vue";

const getUsage = vi.fn();
const getUsageSummary = vi.fn();
const getUsageCost = vi.fn();
vi.mock("@/lib/api", () => ({
	useApi: () => ({ getUsage, getUsageSummary, getUsageCost }),
}));

const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });

describe("UsageView", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		getUsage.mockResolvedValue({ entries: [], total: 0 });
		getUsageSummary.mockResolvedValue({
			total_requests: 0,
			total_prompt_tokens: 0,
			total_completion_tokens: 0,
			avg_latency_ms: 0,
		});
		getUsageCost.mockResolvedValue({
			total_cost_usd: 0,
			known_cost_entries: 0,
			unknown_cost_entries: 0,
			by_provider: {},
			by_model: {},
			by_api_key: {},
		});
	});

	it("associates labels with each usage filter input", async () => {
		const wrapper = mount(UsageView, { global: { plugins: [i18n] } });
		await flushPromises();

		for (const id of [
			"usage-provider",
			"usage-model",
			"usage-api-key",
			"usage-start",
			"usage-end",
		]) {
			expect(wrapper.get(`#${id}`).attributes("id")).toBe(id);
			expect(wrapper.find(`label[for="${id}"]`).exists()).toBe(true);
		}
	});

	it("renders usage and cost without inventing a cost when there is no data", async () => {
		getUsageCost.mockResolvedValueOnce({
			total_cost_usd: 0,
			known_cost_entries: 0,
			unknown_cost_entries: 0,
			by_provider: {},
			by_model: {},
			by_api_key: {},
		});
		const wrapper = mount(UsageView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("No usage data");
		expect(wrapper.text()).toContain("Estimated cost");
		expect(wrapper.text()).toContain("No usage data");
		expect(wrapper.text()).not.toContain("$0.00");
	});

	it("shows aggregated cost when records include known costs", async () => {
		getUsageCost.mockResolvedValueOnce({
			total_cost_usd: 3.25,
			known_cost_entries: 2,
			unknown_cost_entries: 1,
			by_provider: { openai: 3.25 },
			by_model: { "gpt-4o": 3.25 },
			by_api_key: {},
		});
		const wrapper = mount(UsageView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("$3.25");
		expect(wrapper.text()).toContain("1 cost unknown");
	});

	it("applies provider, model, API key, and date filters", async () => {
		const wrapper = mount(UsageView, { global: { plugins: [i18n] } });
		await flushPromises();
		for (const id of [
			"usage-provider",
			"usage-model",
			"usage-api-key",
			"usage-start",
			"usage-end",
		]) {
			expect(wrapper.find(`label[for="${id}"]`).exists()).toBe(true);
			expect(wrapper.find(`#${id}`).exists()).toBe(true);
		}
		await wrapper.findAll("input")[0]?.setValue("openai");
		await wrapper.findAll("input")[1]?.setValue("gpt-4o");
		await wrapper.findAll("input")[2]?.setValue("key-1");
		await wrapper.findAll("input")[3]?.setValue("2026-10-01T00:00");
		await wrapper.findAll("input")[4]?.setValue("2026-10-24T00:00");
		await wrapper.find("form").trigger("submit");
		await flushPromises();
		expect(getUsage).toHaveBeenLastCalledWith(
			expect.objectContaining({
				provider: "openai",
				model: "gpt-4o",
				apiKeyId: "key-1",
				offset: 0,
				limit: 25,
			}),
		);
	});

	it("treats unavailable usage recording as informational", async () => {
		getUsage.mockRejectedValueOnce(
			new Error("API Error 503: USAGE_RECORDER_UNAVAILABLE"),
		);
		const wrapper = mount(UsageView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("Usage recording is not available");
		expect(wrapper.find('[role="alert"]').exists()).toBe(false);
	});
});
