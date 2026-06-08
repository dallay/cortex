import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {nextTick, ref} from "vue";

// We mock the dependencies the composable relies on so the test can
// drive the inputs deterministically.
const mockProviders = ref<any[]>([]);
const mockGetAvailableModels = vi.fn();

vi.mock("@/composables/useProviders", () => ({
  useProviders: () => ({
    providers: mockProviders,
    fetch: vi.fn(),
  }),
}));

vi.mock("@/lib/api", () => ({
  useApi: () => ({
    getAvailableModels: mockGetAvailableModels,
  }),
}));

describe("useAvailableModels", () => {
  beforeEach(() => {
    mockProviders.value = [];
    mockGetAvailableModels.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("starts with empty state and no fetch", () => {
    mockGetAvailableModels.mockResolvedValue({models: []});
    return import("./useAvailableModels").then(
      async ({useAvailableModels}) => {
        const c = useAvailableModels();
        expect(c.modelsByProvider.value).toEqual([]);
        expect(c.loading.value).toBe(false);
        expect(c.error.value).toBeNull();
        expect(c.fetched.value).toBe(false);
        expect(mockGetAvailableModels).not.toHaveBeenCalled();
      },
    );
  });

  it("fetch() populates groups and sets fetched=true", async () => {
    mockGetAvailableModels.mockResolvedValue({
      models: [
        {
          providerId: "p1",
          providerName: "OpenAI Primary",
          providerKind: "openai",
          models: ["gpt-4o"],
        },
        {
          providerId: "p2",
          providerName: "Anthropic",
          providerKind: "anthropic",
          models: ["claude-3-5-sonnet-latest"],
        },
      ],
    });
    mockProviders.value = [
      {
        id: "p1",
        name: "OpenAI Primary",
        providerKind: "openai",
        isActive: true,
      },
      {
        id: "p2",
        name: "Anthropic",
        providerKind: "anthropic",
        isActive: true,
      },
    ];

    const {useAvailableModels} = await import("./useAvailableModels");
    const c = useAvailableModels();
    await c.fetch();
    await nextTick();

    expect(c.fetched.value).toBe(true);
    expect(c.loading.value).toBe(false);
    expect(c.error.value).toBeNull();
    expect(c.modelsByProvider.value).toHaveLength(2);
    expect(c.modelsByProvider.value[0]!.provider.id).toBe("p1");
    expect(c.modelsByProvider.value[0]!.models).toEqual(["gpt-4o"]);
    expect(c.modelsByProvider.value[1]!.provider.id).toBe("p2");
  });

  it("filters out inactive providers from the cross", async () => {
    mockGetAvailableModels.mockResolvedValue({
      models: [
        {
          providerId: "p1",
          providerName: "OpenAI Primary",
          providerKind: "openai",
          models: ["gpt-4o"],
        },
        {
          providerId: "p2",
          providerName: "OpenAI Inactive",
          providerKind: "openai",
          models: ["gpt-4o"],
        },
      ],
    });
    mockProviders.value = [
      {
        id: "p1",
        name: "OpenAI Primary",
        providerKind: "openai",
        isActive: true,
      },
      {
        id: "p2",
        name: "OpenAI Inactive",
        providerKind: "openai",
        isActive: false,
      },
    ];

    const {useAvailableModels} = await import("./useAvailableModels");
    const c = useAvailableModels();
    await c.fetch();
    await nextTick();

    // Only the active provider is in the cross.
    expect(c.modelsByProvider.value).toHaveLength(1);
    expect(c.modelsByProvider.value[0]!.provider.id).toBe("p1");
  });

  it("filters out groups with no models", async () => {
    mockGetAvailableModels.mockResolvedValue({
      models: [
        {
          providerId: "p1",
          providerName: "OpenAI",
          providerKind: "openai",
          models: ["gpt-4o"],
        },
        {
          providerId: "p2",
          providerName: "Empty",
          providerKind: "openai",
          models: [],
        },
      ],
    });
    mockProviders.value = [
      {id: "p1", name: "OpenAI", providerKind: "openai", isActive: true},
      {id: "p2", name: "Empty", providerKind: "openai", isActive: true},
    ];

    const {useAvailableModels} = await import("./useAvailableModels");
    const c = useAvailableModels();
    await c.fetch();
    await nextTick();

    expect(c.modelsByProvider.value).toHaveLength(1);
    expect(c.modelsByProvider.value[0]!.provider.id).toBe("p1");
  });

  it("surfaces errors and keeps groups empty", async () => {
    mockGetAvailableModels.mockRejectedValue(new Error("network down"));
    mockProviders.value = [
      {id: "p1", name: "OpenAI", providerKind: "openai", isActive: true},
    ];

    const {useAvailableModels} = await import("./useAvailableModels");
    const c = useAvailableModels();
    await c.fetch();
    await nextTick();

    expect(c.error.value).toBe("network down");
    expect(c.fetched.value).toBe(false);
    expect(c.modelsByProvider.value).toEqual([]);
  });

  it("cross handles missing group entry for active provider", async () => {
    // Server returned no group for p1, even though p1 is active.
    // The composable should not crash and should drop the provider.
    mockGetAvailableModels.mockResolvedValue({models: []});
    mockProviders.value = [
      {id: "p1", name: "OpenAI", providerKind: "openai", isActive: true},
    ];

    const {useAvailableModels} = await import("./useAvailableModels");
    const c = useAvailableModels();
    await c.fetch();
    await nextTick();

    expect(c.modelsByProvider.value).toEqual([]);
  });
});
