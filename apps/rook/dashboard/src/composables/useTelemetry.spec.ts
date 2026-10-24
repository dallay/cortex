import { beforeEach, describe, expect, it, vi } from "vitest";

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
describe("useTelemetry", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		getTelemetrySummary.mockResolvedValue({ providers: [] });
		getProviderTelemetry.mockResolvedValue({ provider_id: "openai" });
		getProviderLatency.mockResolvedValue({
			p50: 10,
			p95: 20.4,
			p99: 30,
			avg: 15,
			min: 1,
			max: 30,
			count: 8,
		});
	});
	it("loads summary and provider latency details", async () => {
		const { useTelemetry } = await import("./useTelemetry");
		const state = useTelemetry();
		await state.fetch();
		await state.inspect("openai");
		expect(state.providers.value).toEqual([]);
		expect(state.provider.value?.provider_id).toBe("openai");
		expect(state.latency.value?.p95).toBe(20.4);
	});
	it("classifies disabled telemetry as unavailable", async () => {
		getTelemetrySummary.mockRejectedValueOnce(
			new Error("API Error 501: Telemetry not enabled"),
		);
		const { useTelemetry } = await import("./useTelemetry");
		const state = useTelemetry();
		await state.fetch();
		expect(state.availability.value).toBe("unavailable");
	});
	it("formats latency with integer milliseconds and no false precision", async () => {
		const { formatMilliseconds } = await import("./useTelemetry");
		expect(formatMilliseconds(20.4)).toBe("20 ms");
		expect(formatMilliseconds(undefined)).toBe("—");
	});
});
