import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createI18n } from "vue-i18n";
import en from "@/locales/en.json";
import ResilienceView from "./ResilienceView.vue";

const getResilience = vi.fn();
const resetProviderResilience = vi.fn();
vi.mock("@/lib/api", () => ({
	useApi: () => ({ getResilience, resetProviderResilience }),
}));
const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });
describe("ResilienceView", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		getResilience.mockResolvedValue({
			circuit_states: [
				{
					provider: "openai",
					failures: 3,
					state: "open",
					last_failure: "2026-10-24T13:00:00Z",
					cooldown_until: "2026-10-24T14:00:00Z",
					rate_limit_reset: 1792850400,
				},
			],
		});
		resetProviderResilience.mockResolvedValue({ reset: true });
	});
	it("shows breaker state, failures, cooldown and asks before reset", async () => {
		vi.stubGlobal(
			"confirm",
			vi.fn(() => false),
		);
		const wrapper = mount(ResilienceView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("openai");
		expect(wrapper.text()).toContain("open");
		expect(wrapper.text()).toContain("3");
		expect(wrapper.text()).toContain(
			new Date("2026-10-24T14:00:00Z").toLocaleString(),
		);
		await wrapper.find("button").trigger("click");
		expect(resetProviderResilience).not.toHaveBeenCalled();
	});
	it("resets and reloads when confirmation is accepted", async () => {
		vi.stubGlobal(
			"confirm",
			vi.fn(() => true),
		);
		const wrapper = mount(ResilienceView, { global: { plugins: [i18n] } });
		await flushPromises();
		await wrapper.find("button").trigger("click");
		await flushPromises();
		expect(resetProviderResilience).toHaveBeenCalledWith("openai");
		expect(getResilience).toHaveBeenCalledTimes(2);
	});
});
