import {beforeEach, describe, expect, it, vi} from "vitest";
import type {ProviderConnectionResponse} from "@/lib/api";

const mockDisconnectOAuth = vi.fn();
const mockDeleteProvider = vi.fn();
const mockGetProviders = vi.fn();
const mockCreateProvider = vi.fn();
const mockUpdateProvider = vi.fn();
const mockTestProvider = vi.fn();
const mockTestCredentials = vi.fn();
const mockStartOAuthFlow = vi.fn();
const mockGetOAuthStatus = vi.fn();
const mockRefreshOAuth = vi.fn();

vi.mock("@/lib/api", () => ({
  useApi: () => ({
    disconnectOAuth: mockDisconnectOAuth,
    deleteProvider: mockDeleteProvider,
    getProviders: mockGetProviders,
    createProvider: mockCreateProvider,
    updateProvider: mockUpdateProvider,
    testProvider: mockTestProvider,
    testCredentials: mockTestCredentials,
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

  it("handles fetch errors and preserves existing providers", async () => {
    const {useProviders} = await import("./useProviders");
    const store = useProviders();
    store.providers.value = [apiKeyConnection()];

    mockGetProviders.mockRejectedValueOnce(new Error("Network failed"));

    await store.fetch();

    expect(store.error.value).toBe("Network failed");
    expect(store.providers.value.length).toBe(1);
    expect(store.loading.value).toBe(false);
  });

  it("creates a provider successfully and pushes to providers list", async () => {
    const newConn = apiKeyConnection();
    mockCreateProvider.mockResolvedValueOnce(newConn);

    const {useProviders} = await import("./useProviders");
    const store = useProviders();

    const created = await store.create({
      providerKind: "openai",
      providerRuntimeId: "openai-main",
      authType: "apiKey",
      name: "New Connection",
      priority: 1,
      isActive: true,
      credentials: {apiKey: "sk-test"},
      config: {maxConcurrent: 10},
    } as never);

    expect(created).toEqual(newConn);
    expect(store.providers.value).toContainEqual(newConn);
  });

  it("handles create provider error and sets error state", async () => {
    mockCreateProvider.mockRejectedValueOnce(new Error("Name conflict"));

    const {useProviders} = await import("./useProviders");
    const store = useProviders();

    const created = await store.create({} as never);

    expect(created).toBeNull();
    expect(store.error.value).toBe("Name conflict");
  });

  it("updates an existing provider and replaces it in list", async () => {
    const original = apiKeyConnection();
    const updatedConn = {...original, name: "Updated Name"};
    mockUpdateProvider.mockResolvedValueOnce(updatedConn);

    const {useProviders} = await import("./useProviders");
    const store = useProviders();
    store.providers.value = [original];

    const result = await store.update("conn-key", {
      name: "Updated Name",
    } as never);

    expect(result).toEqual(updatedConn);
    expect(store.providers.value[0].name).toBe("Updated Name");
  });

  it("handles update provider error", async () => {
    mockUpdateProvider.mockRejectedValueOnce(new Error("Update failed"));

    const {useProviders} = await import("./useProviders");
    const store = useProviders();

    const result = await store.update("conn-key", {} as never);

    expect(result).toBeNull();
    expect(store.error.value).toBe("Update failed");
  });

  it("runs connection test and credentials test", async () => {
    const testResult = {valid: true, status: "ok", latencyMs: 12};
    mockTestProvider.mockResolvedValueOnce(testResult);
    mockTestCredentials.mockResolvedValueOnce(testResult);

    const {useProviders} = await import("./useProviders");
    const store = useProviders();

    expect(await store.test("conn-key")).toEqual(testResult);
    expect(await store.testCredentials({} as never)).toEqual(testResult);

    // Test error handling for test & testCredentials
    mockTestProvider.mockRejectedValueOnce(new Error("Test crashed"));
    mockTestCredentials.mockRejectedValueOnce(new Error("Cred test crashed"));

    expect(await store.test("conn-key")).toBeNull();
    expect(await store.testCredentials({} as never)).toBeNull();
  });

  it("computes activeProviders and providerById correctly", async () => {
    const activeConn = {...apiKeyConnection(), id: "active-1", testStatus: {status: "active"}};
    const connectedConn = {...oauthConnection(), id: "conn-oauth", testStatus: {status: "connected"}};

    const {useProviders} = await import("./useProviders");
    const store = useProviders();
    store.providers.value = [activeConn as never, connectedConn as never];

    expect(store.activeProviders.value).toEqual([activeConn]);
    expect(store.providerById.value.get("active-1")).toEqual(activeConn);
    expect(store.providerById.value.get("conn-oauth")).toEqual(connectedConn);
  });
});
