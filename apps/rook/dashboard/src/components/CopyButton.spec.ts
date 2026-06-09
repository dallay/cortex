import {flushPromises, mount} from "@vue/test-utils";
import {describe, expect, it, vi} from "vitest";
import {defineComponent, h} from "vue";
import CopyButton from "./CopyButton.vue";

// Mock lucide icons — each renders as a <span> with a predictable data-testid.
// Defined inline inside vi.mock because vi.mock is hoisted to the top of the file.
vi.mock("@lucide/vue", () => ({
  Check: defineComponent({
    name: "Check",
    setup: () => () => h("span", {"data-testid": "icon-Check"}),
  }),
  Copy: defineComponent({
    name: "Copy",
    setup: () => () => h("span", {"data-testid": "icon-Copy"}),
  }),
}));

describe("CopyButton", () => {
  // -------------------------------------------------------------------------
  // navigator.clipboard.writeText — successful path
  // -------------------------------------------------------------------------

  it("copies the given value to clipboard when clicked", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: {writeText},
      configurable: true,
    });

    const wrapper = mount(CopyButton, {
      props: {value: "hello world"},
    });

    await wrapper.find("button").trigger("click");
    await flushPromises();

    expect(writeText).toHaveBeenCalledWith("hello world");
  });

  it("copies the text prop when value is not provided", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: {writeText},
      configurable: true,
    });

    const wrapper = mount(CopyButton, {
      props: {text: "fallback text"},
    });

    await wrapper.find("button").trigger("click");
    await flushPromises();

    expect(writeText).toHaveBeenCalledWith("fallback text");
  });

  // -------------------------------------------------------------------------
  // Icons — Check shown after copy, Copy shown before
  // -------------------------------------------------------------------------

  it("shows Check icon after successful copy", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: {writeText},
      configurable: true,
    });

    const wrapper = mount(CopyButton, {
      props: {value: "hello"},
    });

    // Before click — Copy icon is shown
    expect(wrapper.find('[data-testid="icon-Copy"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="icon-Check"]').exists()).toBe(false);

    await wrapper.find("button").trigger("click");
    await flushPromises();

    // After click — Check icon is shown
    expect(wrapper.find('[data-testid="icon-Check"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="icon-Copy"]').exists()).toBe(false);
  });

  // -------------------------------------------------------------------------
  // Reset after 2 seconds
  // -------------------------------------------------------------------------

  it("resets to Copy icon after 2 seconds", async () => {
    vi.useFakeTimers();
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: {writeText},
      configurable: true,
    });

    const wrapper = mount(CopyButton, {
      props: {value: "test value"},
    });

    await wrapper.find("button").trigger("click");
    await flushPromises();

    // Immediately after click — Check icon shown
    expect(wrapper.find('[data-testid="icon-Check"]').exists()).toBe(true);

    // Advance time past the 2-second reset window
    vi.advanceTimersByTime(2001);
    await flushPromises();

    // Back to Copy icon
    expect(wrapper.find('[data-testid="icon-Copy"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="icon-Check"]').exists()).toBe(false);

    vi.useRealTimers();
  });

  // -------------------------------------------------------------------------
  // aria-label
  // -------------------------------------------------------------------------

  it('has aria-label "Copy to clipboard" before clicking', () => {
    const wrapper = mount(CopyButton, {
      props: {value: "hello"},
    });

    expect(wrapper.find("button").attributes("aria-label")).toBe(
      "Copy to clipboard",
    );
  });

  it('has aria-label "Copied" after clicking', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      value: {writeText},
      configurable: true,
    });

    const wrapper = mount(CopyButton, {
      props: {value: "hello"},
    });

    await wrapper.find("button").trigger("click");
    await flushPromises();

    expect(wrapper.find("button").attributes("aria-label")).toBe("Copied");
  });

  // -------------------------------------------------------------------------
  // DOM fallback — triggered when clipboard API is absent or throws
  // -------------------------------------------------------------------------

  it("uses DOM fallback when clipboard API throws", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("clipboard unavailable"));
    // jsdom doesn't implement execCommand — mock it so DOM fallback succeeds.
    const execCommandMock = vi.fn().mockReturnValue(true);
    Object.defineProperty(document, "execCommand", {
      value: execCommandMock,
      configurable: true,
    });
    Object.defineProperty(navigator, "clipboard", {
      value: {writeText},
      configurable: true,
    });

    const wrapper = mount(CopyButton, {
      props: {value: "fallback text"},
      attachTo: document.body,
    });

    await wrapper.find("button").trigger("click");
    await flushPromises();

    // writeText was called (clipboard API tried first)
    expect(writeText).toHaveBeenCalledWith("fallback text");
    // DOM fallback executed execCommand("copy")
    expect(execCommandMock).toHaveBeenCalledWith("copy");
    // copied became true via DOM fallback
    expect(wrapper.find("button").attributes("aria-label")).toBe("Copied");

    // Restore
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    delete (document as any).execCommand;
  });

  it("uses DOM fallback when clipboard is undefined (jsdom/no HTTPS)", async () => {
    // Remove clipboard entirely — simulates non-HTTPS environment.
    // jsdom doesn't implement execCommand, so we mock it at the document level.
    const desc = Object.getOwnPropertyDescriptor(navigator, "clipboard");
    const execCommandMock = vi.fn().mockReturnValue(true);
    Object.defineProperty(document, "execCommand", {
      value: execCommandMock,
      configurable: true,
    });
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    delete (navigator as any).clipboard;

    const wrapper = mount(CopyButton, {
      props: {value: "no-clipboard-env"},
      attachTo: document.body,
    });

    await wrapper.find("button").trigger("click");
    await flushPromises();

    expect(execCommandMock).toHaveBeenCalledWith("copy");
    expect(wrapper.find("button").attributes("aria-label")).toBe("Copied");

    // Restore for other tests
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    delete (document as any).execCommand;
    if (desc) {
      Object.defineProperty(navigator, "clipboard", desc);
    }
  });
});
