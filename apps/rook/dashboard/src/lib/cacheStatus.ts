import type { CacheStatsResponse } from "@/lib/api";

export type CacheStatus = "disabled" | "empty" | "active";

export function cacheStatus(stats: CacheStatsResponse): CacheStatus {
	if (stats.entries === 0) return "empty";
	return "active";
}
