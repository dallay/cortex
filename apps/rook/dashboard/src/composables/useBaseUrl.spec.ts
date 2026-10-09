import {afterEach, beforeEach, describe, expect, it} from "vitest";
import {useBaseUrl} from "./useBaseUrl";

describe("useBaseUrl", () => {
  const originalLocalStorage = globalThis.localStorage;
  const originalLocation = globalThis.location;

  let storageMock: Record<string, string>;

  beforeEach(() => {
    storageMock = {};
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: {
        getItem: (key: string) => storageMock[key] ?? null,
        setItem: (key: string, value: string) => {
          storageMock[key] = value;
        },
        removeItem: (key: string) => {
          delete storageMock[key];
        },
        clear: () => {
          storageMock = {};
        },
      },
    });
    Object.defineProperty(globalThis, "location", {
      configurable: true,
      value: {
        origin: "http://example.com:4747",
      },
    });
  });

  afterEach(() => {
    Object.defineProperty(globalThis, "localStorage", {
      configurable: true,
      value: originalLocalStorage,
    });
    Object.defineProperty(globalThis, "location", {
      configurable: true,
      value: originalLocation,
    });
  });

  it("defaults to detected location origin without override", () => {
    const {baseUrl, fullBaseUrl, isOverridden} = useBaseUrl();

    expect(isOverridden.value).toBe(false);
    expect(baseUrl.value).toBe("http://example.com:4747");
    expect(fullBaseUrl.value).toBe("http://example.com:4747/v1");
  });

  it("sets, stores, and clears override", () => {
    const {baseUrl, isOverridden, setOverride, clearOverride} = useBaseUrl();

    setOverride("https://custom-rook.api");
    expect(isOverridden.value).toBe(true);
    expect(baseUrl.value).toBe("https://custom-rook.api");
    expect(globalThis.localStorage.getItem("rook-api-base-url")).toBe(
      "https://custom-rook.api"
    );

    clearOverride();
    expect(isOverridden.value).toBe(false);
    expect(baseUrl.value).toBe("http://example.com:4747");
    expect(globalThis.localStorage.getItem("rook-api-base-url")).toBeNull();
  });
});
