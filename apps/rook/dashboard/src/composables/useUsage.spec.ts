import { beforeEach, describe, expect, it, vi } from "vitest";

const getUsage = vi.fn();
const getUsageSummary = vi.fn();
const getUsageCost = vi.fn();
vi.mock("@/lib/api", () => ({
	useApi: () => ({ getUsage, getUsageSummary, getUsageCost }),
}));

describe("useUsage", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		getUsage.mockResolvedValue({ entries: [], total: 0 });
		getUsageSummary.mockResolvedValue({ total_requests: 0 });
		getUsageCost.mockResolvedValue({
			total_cost_usd: 0,
			known_cost_entries: 0,
			unknown_cost_entries: 0,
			by_provider: {},
			by_model: {},
			by_api_key: {},
		});
	});
	it("classifies complete, partial, unknown, and no-data costs", async () => {
		const { useUsage } = await import("./useUsage");
		const state = useUsage();
		await state.fetch();
		expect(state.costStatus.value).toBe("no_data");
		getUsageCost.mockResolvedValueOnce({
			total_cost_usd: 4,
			known_cost_entries: 2,
			unknown_cost_entries: 1,
			by_provider: {},
			by_model: {},
			by_api_key: {},
		});
		await state.fetch();
		expect(state.costStatus.value).toBe("partial");
		getUsageCost.mockResolvedValueOnce({
			total_cost_usd: 0,
			known_cost_entries: 0,
			unknown_cost_entries: 3,
			by_provider: {},
			by_model: {},
			by_api_key: {},
		});
		await state.fetch();
		expect(state.costStatus.value).toBe("unknown");
		getUsageCost.mockResolvedValueOnce({
			total_cost_usd: 4,
			known_cost_entries: 2,
			unknown_cost_entries: 0,
			by_provider: {},
			by_model: {},
			by_api_key: {},
		});
		await state.fetch();
		expect(state.costStatus.value).toBe("complete");
	});
	it("marks recorder unavailability as informational state", async () => {
		getUsage.mockRejectedValueOnce(
			new Error("API Error 503: USAGE_RECORDER_UNAVAILABLE"),
		);
		const { useUsage } = await import("./useUsage");
		const state = useUsage();
		await state.fetch();
		expect(state.availability.value).toBe("unavailable");
	});
});
