import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createI18n } from "vue-i18n";
import en from "@/locales/en.json";
import TelemetryView from "./TelemetryView.vue";

const getTelemetrySummary = vi.fn();
const getProviderTelemetry = vi.fn();
const getProviderLatency = vi.fn();
vi.mock("@/lib/api", () => ({
	useApi: () => ({
		getTelemetrySummary,
		getProviderTelemetry,
		getProviderLatency,
	}),
}));
const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });
describe("TelemetryView", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		getTelemetrySummary.mockResolvedValue({
			providers: [
				{
					provider_id: "openai",
					total_requests: 12,
					success_count: 10,
					failure_count: 1,
					rate_limited_count: 1,
					latency_stats: {
						p50: 120,
						p95: 340.4,
						p99: 500,
						avg: 210,
						min: 50,
						max: 500,
						count: 12,
					},
					ttft_stats: null,
				},
			],
		});
		getProviderTelemetry.mockResolvedValue({ provider_id: "openai" });
		getProviderLatency.mockResolvedValue({
			p50: 120,
			p95: 340.4,
			p99: 500,
			avg: 210,
			min: 50,
			max: 500,
			count: 12,
		});
	});
	it("renders per-provider metrics and rounded latency units", async () => {
		const wrapper = mount(TelemetryView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("openai");
		expect(wrapper.text()).toContain("340 ms");
		expect(wrapper.text()).toContain("ms");
	});
	it("shows 501 as an informational disabled-telemetry state", async () => {
		getTelemetrySummary.mockRejectedValueOnce(
			new Error("API Error 501: Telemetry not enabled"),
		);
		const wrapper = mount(TelemetryView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("Telemetry is not enabled");
		expect(wrapper.find('[role="alert"]').exists()).toBe(false);
	});
});
