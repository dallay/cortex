import { beforeEach, describe, expect, it, vi } from "vitest";

const getApiKeyCount = vi.fn();
const getUsageSummary = vi.fn();
const getUsageCost = vi.fn();
vi.mock("@/lib/api", () => ({
	useApi: () => ({ getApiKeyCount, getUsageSummary, getUsageCost }),
}));
describe("useHomeMetrics", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});
	it("loads API key count, 24-hour usage, and seven-day cost independently", async () => {
		getApiKeyCount.mockResolvedValue(2);
		getUsageSummary.mockResolvedValue({ total_requests: 14 });
		getUsageCost.mockResolvedValue({
			total_cost_usd: 1.25,
			known_cost_entries: 1,
			unknown_cost_entries: 0,
		});
		const { useHomeMetrics } = await import("./useHomeMetrics");
		const state = useHomeMetrics();
		await state.fetch();
		expect(state.apiKeyCount.value).toBe(2);
		expect(state.usage.value?.total_requests).toBe(14);
		expect(state.cost.value?.total_cost_usd).toBe(1.25);
		expect(getUsageSummary.mock.calls[0]?.[0]).toHaveProperty("start");
		expect(getUsageCost.mock.calls[0]?.[0]).toHaveProperty("start");
	});
	it("keeps each metric unavailable independently when its API call fails", async () => {
		getApiKeyCount.mockResolvedValue(0);
		getUsageSummary.mockRejectedValue(new Error("503"));
		getUsageCost.mockResolvedValue({
			total_cost_usd: 0,
			known_cost_entries: 0,
			unknown_cost_entries: 0,
		});
		const { useHomeMetrics } = await import("./useHomeMetrics");
		const state = useHomeMetrics();
		await state.fetch();
		expect(state.apiKeyCount.value).toBe(0);
		expect(state.usage.value).toBeNull();
		expect(state.cost.value?.known_cost_entries).toBe(0);
	});
});
