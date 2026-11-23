/**
 * useApiKeys — CRUD contract for the external agent API keys.
 *
 * The composable owns the client state (list, pagination, error) and
 * wraps the management API. These tests pin the observable behaviour:
 * successful calls update the local state, failed calls surface the
 * error message and never throw, and pagination helpers move the
 * offset without losing the current page size.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiKeyRecordResponse } from "@/lib/api";
import { useApiKeys } from "./useApiKeys";

const apiMock = vi.hoisted(() => ({
	getApiKeys: vi.fn(),
	createApiKey: vi.fn(),
	updateApiKey: vi.fn(),
	revokeApiKey: vi.fn(),
	rotateApiKey: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
	const actual = await importOriginal<typeof import("@/lib/api")>();
	return { ...actual, useApi: () => apiMock };
});

function makeKey(
	overrides: Partial<ApiKeyRecordResponse> = {},
): ApiKeyRecordResponse {
	return {
		id: "key-1",
		label: "CI key",
		keyPrefix: "rk_live_ab",
		scopes: ["chat"],
		tier: "standard",
		isActive: true,
		revokedAt: null,
		expiresAt: null,
		createdAt: "2026-01-01T00:00:00Z",
		lastUsedAt: null,
		allowedModels: [],
		allowedProviders: [],
		spendingBudget: null,
		...overrides,
	};
}

beforeEach(() => {
	apiMock.getApiKeys.mockReset().mockResolvedValue({
		keys: [makeKey()],
		pagination: { total: 1, limit: 20, offset: 0 },
	});
	apiMock.createApiKey.mockReset();
	apiMock.updateApiKey.mockReset();
	apiMock.revokeApiKey.mockReset().mockResolvedValue(undefined);
	apiMock.rotateApiKey.mockReset();
});

describe("useApiKeys.fetch", () => {
	it("stores the keys, the total and the requested window", async () => {
		const keys = useApiKeys();

		await keys.fetch(10, 30);

		expect(apiMock.getApiKeys).toHaveBeenCalledWith(10, 30);
		expect(keys.apiKeys.value).toHaveLength(1);
		expect(keys.total.value).toBe(1);
		expect(keys.limit.value).toBe(10);
		expect(keys.offset.value).toBe(30);
		expect(keys.loading.value).toBe(false);
		expect(keys.error.value).toBeNull();
	});

	it("stores the error message when the request fails", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		apiMock.getApiKeys.mockRejectedValue(new Error("boom"));
		const keys = useApiKeys();

		await keys.fetch();

		expect(keys.error.value).toBe("boom");
		expect(keys.loading.value).toBe(false);
	});
});

describe("useApiKeys.create", () => {
	it("returns the created key and refreshes the list", async () => {
		const created = {
			key: makeKey({ id: "key-2" }),
			plaintextKey: "rk_live_secret",
		};
		apiMock.createApiKey.mockResolvedValue(created);
		const keys = useApiKeys();

		const result = await keys.create({
			label: "CI key",
			scopes: ["chat"],
			tier: "standard",
			expiresAt: null,
		});

		expect(result).toEqual(created);
		expect(apiMock.getApiKeys).toHaveBeenCalledOnce();
	});

	it("returns null and stores the error when the request fails", async () => {
		apiMock.createApiKey.mockRejectedValue(new Error("nope"));
		const keys = useApiKeys();

		const result = await keys.create({
			label: "CI key",
			scopes: [],
			tier: "standard",
			expiresAt: null,
		});

		expect(result).toBeNull();
		expect(keys.error.value).toBe("nope");
	});
});

describe("useApiKeys.update", () => {
	it("replaces the updated key in the local list", async () => {
		const keys = useApiKeys();
		await keys.fetch();
		apiMock.updateApiKey.mockResolvedValue(makeKey({ label: "renamed" }));

		const result = await keys.update("key-1", { label: "renamed" });

		expect(result?.label).toBe("renamed");
		expect(keys.apiKeys.value[0].label).toBe("renamed");
	});

	it("returns null and stores the error when the request fails", async () => {
		apiMock.updateApiKey.mockRejectedValue(new Error("nope"));
		const keys = useApiKeys();

		const result = await keys.update("key-1", { label: "renamed" });

		expect(result).toBeNull();
		expect(keys.error.value).toBe("nope");
	});
});

describe("useApiKeys.revoke", () => {
	it("marks the key as revoked in the local list", async () => {
		const keys = useApiKeys();
		await keys.fetch();

		const ok = await keys.revoke("key-1");

		expect(ok).toBe(true);
		expect(apiMock.revokeApiKey).toHaveBeenCalledWith("key-1");
		expect(keys.apiKeys.value[0].isActive).toBe(false);
		expect(keys.apiKeys.value[0].revokedAt).not.toBeNull();
	});

	it("returns false and stores the error when the request fails", async () => {
		apiMock.revokeApiKey.mockRejectedValue(new Error("nope"));
		const keys = useApiKeys();

		const ok = await keys.revoke("key-1");

		expect(ok).toBe(false);
		expect(keys.error.value).toBe("nope");
	});
});

describe("useApiKeys.remove", () => {
	it("delegates to the revoke call", async () => {
		const keys = useApiKeys();
		await keys.fetch();

		const ok = await keys.remove("key-1");

		expect(ok).toBe(true);
		expect(apiMock.revokeApiKey).toHaveBeenCalledWith("key-1");
	});
});

describe("useApiKeys.rotate", () => {
	it("updates the key prefix in the local list", async () => {
		const keys = useApiKeys();
		await keys.fetch();
		apiMock.rotateApiKey.mockResolvedValue({
			key: makeKey({ keyPrefix: "rk_live_cd" }),
			plaintextKey: "rk_live_new",
		});

		const result = await keys.rotate("key-1");

		expect(result?.plaintextKey).toBe("rk_live_new");
		expect(keys.apiKeys.value[0].keyPrefix).toBe("rk_live_cd");
	});

	it("returns null and stores the error when the request fails", async () => {
		apiMock.rotateApiKey.mockRejectedValue(new Error("nope"));
		const keys = useApiKeys();

		const result = await keys.rotate("key-1");

		expect(result).toBeNull();
		expect(keys.error.value).toBe("nope");
	});
});

describe("useApiKeys pagination", () => {
	it("advances and rewinds the offset by one page", async () => {
		const keys = useApiKeys();
		await keys.fetch(20, 20);

		keys.nextPage();
		await vi.waitFor(() => {
			expect(apiMock.getApiKeys).toHaveBeenLastCalledWith(20, 40);
		});

		keys.prevPage();
		await vi.waitFor(() => {
			expect(apiMock.getApiKeys).toHaveBeenLastCalledWith(20, 20);
		});
	});

	it("never rewinds below the first page", async () => {
		const keys = useApiKeys();
		await keys.fetch(20, 0);

		keys.prevPage();
		await vi.waitFor(() => {
			expect(apiMock.getApiKeys).toHaveBeenLastCalledWith(20, 0);
		});
	});

	it("jumps to the requested page", async () => {
		const keys = useApiKeys();
		await keys.fetch(20, 0);

		keys.setPage(2);
		await vi.waitFor(() => {
			expect(apiMock.getApiKeys).toHaveBeenLastCalledWith(20, 40);
		});
	});

	it("derives the page count and the current page", async () => {
		const keys = useApiKeys();
		await keys.fetch(20, 40);

		expect(keys.totalPages()).toBe(1);
		expect(keys.currentPage()).toBe(2);
	});
});
