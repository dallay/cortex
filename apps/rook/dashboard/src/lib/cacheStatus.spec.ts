import { describe, expect, it } from "vitest";
import { cacheStatus } from "./cacheStatus";

const base = {
	hits: 10,
	misses: 5,
	evictions: 1,
	entries: 2,
	max_entries: 10,
	token_cache: {
		hits: 0,
		misses: 0,
		tokens_saved: 0,
		estimated_cost_saved_usd: 0,
	},
};
describe("cacheStatus", () => {
	it("treats zero maximum capacity as unlimited, not disabled", () => {
		expect(cacheStatus({ ...base, max_entries: 0 })).toBe("active");
		expect(cacheStatus({ ...base, entries: 0 })).toBe("empty");
		expect(cacheStatus(base)).toBe("active");
	});
});
