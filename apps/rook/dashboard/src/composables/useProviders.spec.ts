import {beforeEach, describe, expect, it, vi} from "vitest";
import type {ProviderConnectionResponse} from "@/lib/api";

const mockDisconnectOAuth = vi.fn();
const mockDeleteProvider = vi.fn();
const mockGetProviders = vi.fn();
const mockStartOAuthFlow = vi.fn();
const mockGetOAuthStatus = vi.fn();
const mockRefreshOAuth = vi.fn();

vi.mock("@/lib/api", () => ({
  useApi: () => ({
    disconnectOAuth: mockDisconnectOAuth,
    deleteProvider: mockDeleteProvider,
    getProviders: mockGetProviders,
    startOAuthFlow: mockStartOAuthFlow,
    getOAuthStatus: mockGetOAuthStatus,
    refreshOAuth: mockRefreshOAuth,
  }),
}));

function oauthConnection(): ProviderConnectionResponse {
  return {
    id: "conn-oauth",
    providerKind: "azure-openai",
    providerRuntimeId: "azure-prod",
    authType: "oauth",
    name: "Azure prod",
    priority: 10,
    isActive: true,
    config: {maxConcurrent: 10},
    testStatus: {status: "connected"},
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  } as unknown as ProviderConnectionResponse;
}

function apiKeyConnection(): ProviderConnectionResponse {
  return {...oauthConnection(), id: "conn-key", authType: "apiKey"};
}

describe("useProviders OAuth paths", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockDisconnectOAuth.mockResolvedValue(undefined);
    mockDeleteProvider.mockResolvedValue(undefined);
    mockGetProviders.mockResolvedValue([]);
    mockStartOAuthFlow.mockResolvedValue({flowId: "flow-1"});
    mockGetOAuthStatus.mockResolvedValue({status: "pending"});
    mockRefreshOAuth.mockResolvedValue(undefined);
  });

  it("disconnects OAuth connections via disconnectOAuth on remove", async () => {
    const {useProviders} = await import("./useProviders");
    const store = useProviders();
    store.providers.value = [oauthConnection()];

    expect(await store.remove("conn-oauth")).toBe(true);
    expect(mockDisconnectOAuth).toHaveBeenCalledWith(
      "azure-openai",
      "conn-oauth",
    );
    expect(mockDeleteProvider).not.toHaveBeenCalled();
    expect(store.providers.value).toEqual([]);
  });

  it("deletes API-key connections via deleteProvider on remove", async () => {
    const {useProviders} = await import("./useProviders");
    const store = useProviders();
    store.providers.value = [apiKeyConnection()];

    expect(await store.remove("conn-key")).toBe(true);
    expect(mockDeleteProvider).toHaveBeenCalledWith("conn-key");
    expect(mockDisconnectOAuth).not.toHaveBeenCalled();
  });

  it("reports deletion errors", async () => {
    mockDisconnectOAuth.mockRejectedValueOnce(new Error("nope"));
    const {useProviders} = await import("./useProviders");
    const store = useProviders();
    store.providers.value = [oauthConnection()];

    expect(await store.remove("conn-oauth")).toBe(false);
    expect(store.error.value).toBe("nope");
  });

  it("starts an OAuth flow and surfaces failures", async () => {
    const {useProviders} = await import("./useProviders");
    const store = useProviders();
    const payload = {
      flowType: "authorizationCode",
      providerRuntimeId: "azure-prod",
      name: "Azure prod",
      priority: 10,
      isActive: true,
      config: {maxConcurrent: 10},
      clientId: "client-id",
      redirectUri: "https://rook.example.com/api/oauth/callback",
    } as never;

    expect(await store.startOAuthFlow("azure-openai", payload)).toEqual({
      flowId: "flow-1",
    });
    expect(store.error.value).toBeNull();

    mockStartOAuthFlow.mockRejectedValueOnce(new Error("denied"));
    expect(await store.startOAuthFlow("azure-openai", payload)).toBeNull();
    expect(store.error.value).toBe("denied");
  });

  it("reads OAuth status and surfaces failures", async () => {
    const {useProviders} = await import("./useProviders");
    const store = useProviders();

    expect(await store.getOAuthStatus("gemini", {flowId: "flow-1"})).toEqual({
      status: "pending",
    });

    mockGetOAuthStatus.mockRejectedValueOnce("boom");
    expect(await store.getOAuthStatus("gemini", {flowId: "flow-1"})).toBeNull();
    expect(store.error.value).toBe("Failed to read OAuth status");
  });

  it("refreshes OAuth and refetches providers", async () => {
    const {useProviders} = await import("./useProviders");
    const store = useProviders();

    expect(await store.refreshOAuth("azure-openai", "conn-oauth")).toBe(true);
    expect(mockRefreshOAuth).toHaveBeenCalledWith(
      "azure-openai",
      "conn-oauth",
    );
    expect(mockGetProviders).toHaveBeenCalled();

    mockRefreshOAuth.mockRejectedValueOnce(new Error("stale"));
    expect(await store.refreshOAuth("azure-openai", "conn-oauth")).toBe(
      false,
    );
    expect(store.error.value).toBe("stale");
  });
});
