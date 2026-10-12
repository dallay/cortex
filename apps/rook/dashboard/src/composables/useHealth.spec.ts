import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextTick } from "vue";

const mockGetHealth = vi.fn();

vi.mock("@/lib/api", () => ({
  useApi: () => ({
    getHealth: mockGetHealth,
  }),
}));

describe("useHealth", () => {
  beforeEach(() => {
    mockGetHealth.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("defaults appName to Rook and version to empty when health data is null", async () => {
    mockGetHealth.mockReturnValue(new Promise(() => {})); // Never resolves
    const { useHealth } = await import("./useHealth");
    const health = useHealth();

    expect(health.appName.value).toBe("Rook");
    expect(health.version.value).toBe("");
  });

  it("fetches health exactly once when the component mounts", async () => {
    mockGetHealth.mockResolvedValue({
      status: "healthy",
      app_name: "Rook",
      version: "0.2.0",
      providers: [],
    });
    const { useHealth } = await import("./useHealth");
    const wrapper = mount({
      setup() {
        useHealth();
        expect(mockGetHealth).not.toHaveBeenCalled();
        return () => null;
      },
    });

    try {
      await flushPromises();
      expect(mockGetHealth).toHaveBeenCalledTimes(1);
    } finally {
      wrapper.unmount();
    }
  });

  it("populates appName and version from health endpoint response", async () => {
    mockGetHealth.mockResolvedValue({
      status: "healthy",
      app_name: "Rook Custom",
      version: "0.2.0",
      providers: [],
    });

    const { useHealth } = await import("./useHealth");
    const health = useHealth();
    await health.fetch();
    await nextTick();

    expect(health.appName.value).toBe("Rook Custom");
    expect(health.version.value).toBe("v0.2.0");
  });

  it("preserves v prefix if version already starts with v", async () => {
    mockGetHealth.mockResolvedValue({
      status: "healthy",
      app_name: "Rook",
      version: "v1.0.0",
      providers: [],
    });

    const { useHealth } = await import("./useHealth");
    const health = useHealth();
    await health.fetch();
    await nextTick();

    expect(health.version.value).toBe("v1.0.0");
  });
});
