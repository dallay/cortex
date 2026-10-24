import { beforeEach, describe, expect, it, vi } from "vitest";

const getResilience = vi.fn();
const resetProviderResilience = vi.fn();
vi.mock("@/lib/api", () => ({
	useApi: () => ({ getResilience, resetProviderResilience }),
}));
describe("useResilience", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		getResilience.mockResolvedValue({
			circuit_states: [
				{
					provider: "openai",
					failures: 2,
					state: "open",
					last_failure: null,
					cooldown_until: null,
					rate_limit_reset: null,
				},
			],
		});
		resetProviderResilience.mockResolvedValue({ reset: true });
	});
	it("loads circuit states and refreshes after reset", async () => {
		const { useResilience } = await import("./useResilience");
		const state = useResilience();
		await state.fetch();
		await state.reset("openai");
		expect(resetProviderResilience).toHaveBeenCalledWith("openai");
		expect(getResilience).toHaveBeenCalledTimes(2);
		expect(state.states.value[0]?.state).toBe("open");
	});
});
