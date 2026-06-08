import {createPinia, setActivePinia} from "pinia";
import {beforeEach, describe, expect, it, vi} from "vitest";

const apiMock = vi.hoisted(() => ({
  getMe: vi.fn(),
  getBootstrapStatus: vi.fn(),
  setupBootstrap: vi.fn(),
  login: vi.fn(),
  logout: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  useApi: () => apiMock,
}));

describe("auth store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.clearAllMocks();
  });

  it("marks bootstrap as required when the backend is not initialized", async () => {
    apiMock.getBootstrapStatus.mockResolvedValueOnce({
      isInitialized: false,
      adminUserExists: true,
      // setup_token is no longer returned by the status endpoint — it is
      // printed to server logs at startup as an out-of-band secret.
    });

    const {useAuthStore} = await import("./auth");
    const store = useAuthStore();

    await store.loadBootstrapStatus();

    expect(store.bootstrapRequired).toBe(true);
    expect(store.initialized).toBe(true);
    // setupToken is NOT populated from status — user must paste it from server logs
    expect(store.isAuthenticated).toBe(false);
  });

  it("marks the admin session authenticated after successful login", async () => {
    apiMock.login.mockResolvedValueOnce({
      sessionId: "session-1",
      expiresAt: "2026-01-01T00:00:00Z",
    });

    const {useAuthStore} = await import("./auth");
    const store = useAuthStore();

    await store.login("test-fixture-password");

    expect(apiMock.login).toHaveBeenCalledWith({
      username: "admin",
      password: "test-fixture-password",
    });
    expect(store.isAuthenticated).toBe(true);
    expect(store.currentUser?.username).toBe("admin");
    expect(store.error).toBeNull();
  });

  it("sets up the first admin password using the provided setup token", async () => {
    apiMock.setupBootstrap.mockResolvedValueOnce({
      apiKey: "rk_admin_initial",
    });
    apiMock.login.mockResolvedValueOnce({
      sessionId: "session-1",
      expiresAt: "2026-01-01T00:00:00Z",
    });

    const {useAuthStore} = await import("./auth");
    const store = useAuthStore();

    // User pastes the token from server logs into the setup form
    await store.setupAdminPassword(
      "rk-setup-token-from-logs",
      "test-fixture-password",
    );

    expect(apiMock.setupBootstrap).toHaveBeenCalledWith({
      setupToken: "rk-setup-token-from-logs",
      password: "test-fixture-password",
    });
    expect(apiMock.login).toHaveBeenCalledWith({
      username: "admin",
      password: "test-fixture-password",
    });
    expect(store.bootstrapRequired).toBe(false);
    expect(store.initialApiKey).toBe("rk_admin_initial");
    expect(store.isAuthenticated).toBe(true);
  });

  it("rejects setup when no setup token is provided", async () => {
    const {useAuthStore} = await import("./auth");
    const store = useAuthStore();

    await expect(
      store.setupAdminPassword("", "test-fixture-password"),
    ).rejects.toThrow("Setup token is missing");

    expect(apiMock.setupBootstrap).not.toHaveBeenCalled();
    expect(store.error).toBe("Setup token is missing");
  });
});
