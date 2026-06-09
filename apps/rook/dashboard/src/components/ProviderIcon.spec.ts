import {mount} from "@vue/test-utils";
import {describe, expect, it} from "vitest";
import ProviderIcon from "./ProviderIcon.vue";

describe("ProviderIcon", () => {
  // -------------------------------------------------------------------------
  // Strategy 1: Iconify bundle (openai, anthropic, ollama, gemini)
  // These render as inline <svg> via @iconify/vue — no <img>, no HTTP request.
  // -------------------------------------------------------------------------

  it("renders an inline <svg> for openai (Iconify bundle path)", () => {
    const wrapper = mount(ProviderIcon, {props: {kind: "openai"}});
    expect(wrapper.find("svg").exists()).toBe(true);
    expect(wrapper.find("img").exists()).toBe(false);
  });

  it("renders an inline <svg> for anthropic (Iconify bundle path)", () => {
    const wrapper = mount(ProviderIcon, {props: {kind: "anthropic"}});
    expect(wrapper.find("svg").exists()).toBe(true);
  });

  it("renders an inline <svg> for ollama (Iconify bundle path)", () => {
    const wrapper = mount(ProviderIcon, {props: {kind: "ollama"}});
    expect(wrapper.find("svg").exists()).toBe(true);
  });

  it("renders an inline <svg> for gemini (Iconify bundle path)", () => {
    const wrapper = mount(ProviderIcon, {props: {kind: "gemini"}});
    expect(wrapper.find("svg").exists()).toBe(true);
  });

  // -------------------------------------------------------------------------
  // Strategy 2: local <img> (groq, ollama-cloud — not in simple-icons yet)
  // -------------------------------------------------------------------------

  it("renders a local <img> for groq (local asset path)", () => {
    const wrapper = mount(ProviderIcon, {props: {kind: "groq"}});
    const img = wrapper.find("img");
    expect(img.exists()).toBe(true);
    expect(img.attributes("src")).toBe("/providers/groq.svg");
  });

  it("renders an inline <svg> for ollama-cloud (same Iconify icon as ollama)", () => {
    const wrapper = mount(ProviderIcon, {props: {kind: "ollama-cloud"}});
    expect(wrapper.find("svg").exists()).toBe(true);
    expect(wrapper.find("img").exists()).toBe(false);
  });

  // -------------------------------------------------------------------------
  // Accessibility — aria semantics apply to both paths
  // -------------------------------------------------------------------------

  it("sets aria-hidden on the Iconify svg when decorative (default)", () => {
    const wrapper = mount(ProviderIcon, {props: {kind: "openai"}});
    const svg = wrapper.find("svg");
    expect(svg.attributes("aria-hidden")).toBe("true");
  });

  it("sets aria-hidden on the local img when decorative (default)", () => {
    const wrapper = mount(ProviderIcon, {props: {kind: "groq"}});
    const img = wrapper.find("img");
    expect(img.attributes("aria-hidden")).toBe("true");
    expect(img.attributes("alt")).toBe("");
  });

  it("sets role=img and aria-label on local img when not decorative", () => {
    const wrapper = mount(ProviderIcon, {
      props: {kind: "groq", decorative: false},
    });
    const img = wrapper.find("img");
    expect(img.attributes("role")).toBe("img");
    expect(img.attributes("aria-label")).toBeDefined();
    expect(img.attributes("aria-label")).not.toBe("");
  });

  it("sets role=img and aria-label on Iconify svg when not decorative", () => {
    const wrapper = mount(ProviderIcon, {
      props: {kind: "openai", decorative: false},
    });
    const svg = wrapper.find("svg");
    expect(svg.attributes("role")).toBe("img");
    expect(svg.attributes("aria-label")).toBeDefined();
    expect(svg.attributes("aria-label")).not.toBe("");
  });

  // -------------------------------------------------------------------------
  // Performance — CLS prevention and loading strategy
  // -------------------------------------------------------------------------

  it("passes explicit width and height to the Iconify svg", () => {
    const wrapper = mount(ProviderIcon, {
      props: {kind: "anthropic", width: 40, height: 40},
    });
    const svg = wrapper.find("svg");
    expect(svg.attributes("width")).toBe("40");
    expect(svg.attributes("height")).toBe("40");
  });

  it("passes explicit width and height to the local img", () => {
    const wrapper = mount(ProviderIcon, {
      props: {kind: "groq", width: 40, height: 40},
    });
    const img = wrapper.find("img");
    expect(img.attributes("width")).toBe("40");
    expect(img.attributes("height")).toBe("40");
  });

  it('sets loading="lazy" by default on local img', () => {
    const wrapper = mount(ProviderIcon, {props: {kind: "groq"}});
    expect(wrapper.find("img").attributes("loading")).toBe("lazy");
  });

  it('sets loading="eager" on local img when specified', () => {
    const wrapper = mount(ProviderIcon, {
      props: {kind: "groq", loading: "eager"},
    });
    expect(wrapper.find("img").attributes("loading")).toBe("eager");
  });

  // -------------------------------------------------------------------------
  // Strategy 3: fallback — Lucide Server icon when local asset fails
  // -------------------------------------------------------------------------

  it("renders the Lucide Server fallback when a local image emits @error", async () => {
    const wrapper = mount(ProviderIcon, {props: {kind: "groq"}});
    // Trigger image load error
    await wrapper.find("img").trigger("error");
    // img gone, svg fallback visible
    expect(wrapper.find("img").exists()).toBe(false);
    expect(wrapper.find("svg").exists()).toBe(true);
  });
});
