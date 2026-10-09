import {mount} from "@vue/test-utils";
import {beforeEach, describe, expect, it} from "vitest";
import {createI18n} from "vue-i18n";
import en from "@/locales/en.json";
import SettingsView from "./SettingsView.vue";

describe("SettingsView", () => {
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
  });

  const i18n = createI18n({
    legacy: false,
    locale: "en",
    messages: {en},
  });

  it("renders with associated label and inputs", async () => {
    const wrapper = mount(SettingsView, {
      global: {
        plugins: [i18n],
      },
    });

    const input = wrapper.find("input#settings-base-url");
    expect(input.exists()).toBe(true);

    await input.setValue("https://new-api.local");
    const saveButton = wrapper
      .findAll("button")
      .find((b) => b.text().includes("Save"));
    expect(saveButton).toBeDefined();
    await saveButton?.trigger("click");

    expect(storageMock["rook-api-base-url"]).toBe("https://new-api.local");
  });
});
