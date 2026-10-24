import { beforeEach, describe, expect, it, vi } from "vitest";

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

describe("useRateLimits", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		getRateLimits.mockResolvedValue([]);
		createRateLimit.mockResolvedValue({ id: "rl-1" });
		updateRateLimit.mockResolvedValue({ id: "rl-1" });
		deleteRateLimit.mockResolvedValue(undefined);
		getRateLimitStatus.mockResolvedValue({ remaining_minute: 58 });
	});
	it("loads rules, creates, updates, deletes, and reads status", async () => {
		const { useRateLimits } = await import("./useRateLimits");
		const state = useRateLimits();
		await state.fetch();
		await state.create({
			scope: "api-key",
			target: "key-1",
			requests_per_minute: 60,
		});
		await state.update("rl-1", { requests_per_minute: 90 });
		await state.fetchStatus("api-key", "key-1");
		await state.remove("rl-1");
		expect(createRateLimit).toHaveBeenCalled();
		expect(updateRateLimit).toHaveBeenCalledWith("rl-1", {
			requests_per_minute: 90,
		});
		expect(state.status.value?.remaining_minute).toBe(58);
		expect(state.rules.value).toEqual([]);
	});
	it("keeps an informational unavailable state on a missing store router", async () => {
		getRateLimits.mockRejectedValueOnce(new Error("API Error 404: Not Found"));
		const { useRateLimits } = await import("./useRateLimits");
		const state = useRateLimits();
		await state.fetch();
		expect(state.availability.value).toBe("unavailable");
	});
});
