import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createI18n } from "vue-i18n";
import en from "@/locales/en.json";
import RateLimitsView from "./RateLimitsView.vue";

const getRateLimits = vi.fn();
const createRateLimit = vi.fn();
const updateRateLimit = vi.fn();
const deleteRateLimit = vi.fn();
const getRateLimitStatus = vi.fn();
vi.mock("@/lib/api", () => ({
	useApi: () => ({
		getRateLimits,
		createRateLimit,
		updateRateLimit,
		deleteRateLimit,
		getRateLimitStatus,
	}),
}));
const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });

describe("RateLimitsView", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		getRateLimits.mockResolvedValue([
			{
				id: "rl-1",
				scope: "api-key",
				target: "key-1",
				requests_per_minute: 60,
				requests_per_day: 1000,
				tokens_per_minute: null,
			},
		]);
		getRateLimitStatus.mockResolvedValue({
			current_minute_count: 2,
			remaining_minute: 58,
			reset_at: "2026-10-24T14:00:00Z",
			active_requests_per_minute: 60,
		});
		createRateLimit.mockResolvedValue({});
		updateRateLimit.mockResolvedValue({});
		deleteRateLimit.mockResolvedValue(undefined);
	});
	it("shows rules, numeric quota and the live status", async () => {
		const wrapper = mount(RateLimitsView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("key-1");
		expect(wrapper.text()).toContain("60");
		expect(wrapper.text()).toContain("58");
		expect(wrapper.find('label[for="rate-limit-status-target"]').exists()).toBe(
			true,
		);
		for (const label of wrapper.findAll("label")) {
			const controlId = label.attributes("for");
			expect(controlId).toBeTruthy();
			expect(wrapper.find(`#${controlId}`).exists()).toBe(true);
		}
	});
	it("allows clearing the optional daily limit and reports invalid limits", async () => {
		const wrapper = mount(RateLimitsView, { global: { plugins: [i18n] } });
		await flushPromises();
		const inputs = wrapper.findAll('input[type="number"]');
		await inputs[1]?.setValue(500);
		await inputs[1]?.setValue("");
		await wrapper.find("form").trigger("submit");
		await flushPromises();
		expect(createRateLimit).toHaveBeenCalledWith(
			expect.objectContaining({ requests_per_day: undefined }),
		);
		await inputs[0]?.setValue(0);
		await wrapper.find("form").trigger("submit");
		expect(wrapper.text()).toContain("Enter a positive whole-number limit");
	});
	it("shows backend router absence as informational", async () => {
		getRateLimits.mockRejectedValueOnce(new Error("API Error 404: Not Found"));
		const wrapper = mount(RateLimitsView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("Rate limit management is not available");
		expect(wrapper.find('[role="alert"]').exists()).toBe(false);
	});
});
