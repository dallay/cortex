import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {mount} from "@vue/test-utils";
import {defineComponent, nextTick} from "vue";

const getProvidersQuota = vi.fn();
vi.mock("@/lib/api", () => ({useApi: () => ({getProvidersQuota})}));
const response = {generatedAt: "2026-01-01T00:00:00Z", items: []};

 describe("useProvidersQuota", () => {
  beforeEach(() => { vi.useFakeTimers(); getProvidersQuota.mockReset(); });
  afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

  it("keeps the last good result and exposes stale state after refresh failure", async () => {
    getProvidersQuota.mockResolvedValueOnce(response).mockRejectedValueOnce(new Error("offline"));
    const {useProvidersQuota} = await import("./useProvidersQuota");
    const wrapper = mount(defineComponent({setup() { return useProvidersQuota(); }, template: "<div />"}));
    await nextTick(); await Promise.resolve(); await Promise.resolve();
    expect(wrapper.vm.quota).toEqual(response);
    await wrapper.vm.fetch();
    expect(wrapper.vm.quota).toEqual(response);
    expect(wrapper.vm.stale).toBe(true);
    expect(wrapper.vm.error).toBe("offline");
    wrapper.unmount();
  });

  it("coalesces overlapping fetches and refreshes every 60 seconds", async () => {
    let resolve!: (value: typeof response) => void;
    getProvidersQuota.mockReturnValue(new Promise((done) => { resolve = done; }));
    const {useProvidersQuota} = await import("./useProvidersQuota");
    const wrapper = mount(defineComponent({setup() { return useProvidersQuota(); }, template: "<div />"}));
    await nextTick();
    expect(getProvidersQuota).toHaveBeenCalledTimes(1);
    const pending = wrapper.vm.fetch();
    expect(getProvidersQuota).toHaveBeenCalledTimes(1);
    resolve(response);
    await pending;
    await vi.advanceTimersByTimeAsync(60_000);
    expect(getProvidersQuota).toHaveBeenCalledTimes(2);
    wrapper.unmount();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(getProvidersQuota).toHaveBeenCalledTimes(2);
  });

  it("clears stale state after a successful recovery", async () => {
    getProvidersQuota.mockResolvedValueOnce(response).mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce({...response, generatedAt: "recovered"});
    const {useProvidersQuota} = await import("./useProvidersQuota");
    const wrapper = mount(defineComponent({setup() { return useProvidersQuota(); }, template: "<div />"}));
    await nextTick(); await Promise.resolve(); await Promise.resolve();
    await wrapper.vm.fetch();
    expect(wrapper.vm.stale).toBe(true);
    await wrapper.vm.fetch();
    expect(wrapper.vm.stale).toBe(false);
    expect(wrapper.vm.error).toBeNull();
    expect(wrapper.vm.generatedAt).toBe("recovered");
    wrapper.unmount();
  });

  it("reports an initial failure without creating stale data", async () => {
    getProvidersQuota.mockRejectedValue(new Error("offline"));
    const {useProvidersQuota} = await import("./useProvidersQuota");
    const wrapper = mount(defineComponent({setup() { return useProvidersQuota(); }, template: "<div />"}));
    await nextTick(); await Promise.resolve(); await Promise.resolve();
    expect(wrapper.vm.quota).toBeNull();
    expect(wrapper.vm.stale).toBe(false);
    expect(wrapper.vm.error).toBe("offline");
    wrapper.unmount();
  });
});
