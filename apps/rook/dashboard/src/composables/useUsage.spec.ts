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
	it("keeps results from the latest overlapping query only", async () => {
		const { useUsage } = await import("./useUsage");
		const state = useUsage();
		let resolveOld!: (value: {
			entries: { id: string }[];
			total: number;
		}) => void;
		let resolveOldSummary!: (value: { total_requests: number }) => void;
		let resolveOldCost!: (value: {
			total_cost_usd: number;
			known_cost_entries: number;
			unknown_cost_entries: number;
			by_provider: Record<string, number>;
			by_model: Record<string, number>;
			by_api_key: Record<string, number>;
		}) => void;
		getUsage.mockImplementationOnce(
			() =>
				new Promise<{ entries: { id: string }[]; total: number }>((resolve) => {
					resolveOld = resolve;
				}),
		);
		getUsageSummary.mockImplementationOnce(
			() =>
				new Promise<{ total_requests: number }>((resolve) => {
					resolveOldSummary = resolve;
				}),
		);
		getUsageCost.mockImplementationOnce(
			() =>
				new Promise<{
					total_cost_usd: number;
					known_cost_entries: number;
					unknown_cost_entries: number;
					by_provider: Record<string, number>;
					by_model: Record<string, number>;
					by_api_key: Record<string, number>;
				}>((resolve) => {
					resolveOldCost = resolve;
				}),
		);
		const oldFetch = state.fetch({ offset: 0, limit: 25 });
		getUsage.mockResolvedValueOnce({ entries: [{ id: "new" }], total: 1 });
		getUsageSummary.mockResolvedValueOnce({ total_requests: 1 });
		getUsageCost.mockResolvedValueOnce({
			total_cost_usd: 1,
			known_cost_entries: 1,
			unknown_cost_entries: 0,
			by_provider: {},
			by_model: {},
			by_api_key: {},
		});
		await state.fetch({ offset: 25, limit: 25 });
		resolveOld({ entries: [{ id: "old" }], total: 1 });
		resolveOldSummary({ total_requests: 0 });
		resolveOldCost({
			total_cost_usd: 0,
			known_cost_entries: 0,
			unknown_cost_entries: 0,
			by_provider: {},
			by_model: {},
			by_api_key: {},
		});
		await oldFetch;
		expect(state.entries.value).toEqual([{ id: "new" }]);
		expect(state.loading.value).toBe(false);
	});
	it("does not classify unrelated error digits as recorder unavailability", async () => {
		const { useUsage } = await import("./useUsage");
		getUsage.mockRejectedValueOnce(new Error("request 503 data format error"));
		const state = useUsage();
		await state.fetch();
		expect(state.availability.value).toBe("error");
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
