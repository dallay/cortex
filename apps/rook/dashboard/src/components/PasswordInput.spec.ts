import {mount} from "@vue/test-utils";
import {describe, expect, it} from "vitest";
import PasswordInput from "./PasswordInput.vue";

describe("PasswordInput", () => {
  // -------------------------------------------------------------------------
  // Password visibility toggle
  // -------------------------------------------------------------------------

  it("shows password field by default (hidden)", () => {
    const wrapper = mount(PasswordInput, {
      props: {modelValue: "secret"},
    });
    const input = wrapper.find('input[type="password"]');
    expect(input.exists()).toBe(true);
  });

  it("reveals password when toggle button is clicked", async () => {
    const wrapper = mount(PasswordInput, {
      props: {modelValue: "secret"},
    });

    const toggleButton = wrapper.find("button");
    await toggleButton.trigger("click");

    const input = wrapper.find('input[type="text"]');
    expect(input.exists()).toBe(true);
  });

  it("hides password again when toggle is clicked twice", async () => {
    const wrapper = mount(PasswordInput, {
      props: {modelValue: "secret"},
    });

    await wrapper.find("button").trigger("click");
    await wrapper.find("button").trigger("click");

    const input = wrapper.find('input[type="password"]');
    expect(input.exists()).toBe(true);
  });

  // -------------------------------------------------------------------------
  // aria-label reflects current visibility state
  // -------------------------------------------------------------------------

  it('has aria-label "Show password" when hidden', () => {
    const wrapper = mount(PasswordInput, {
      props: {modelValue: "secret"},
    });
    expect(wrapper.find("button").attributes("aria-label")).toBe(
      "Show password",
    );
  });

  it('has aria-label "Hide password" when visible', async () => {
    const wrapper = mount(PasswordInput, {
      props: {modelValue: "secret"},
    });

    await wrapper.find("button").trigger("click");

    expect(wrapper.find("button").attributes("aria-label")).toBe(
      "Hide password",
    );
  });

  // -------------------------------------------------------------------------
  // Emits update:modelValue
  // -------------------------------------------------------------------------

  it("emits update:modelValue on input", async () => {
    const wrapper = mount(PasswordInput, {
      props: {modelValue: ""},
    });

    const input = wrapper.find("input");
    await input.setValue("new value");

    const emitted = wrapper.emitted("update:modelValue");
    expect(emitted).toBeTruthy();
    expect(emitted![emitted!.length - 1]).toEqual(["new value"]);
  });
});
