import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { createI18n } from "vue-i18n";
import en from "@/locales/en.json";
import ProviderCatalogFilter from "./ProviderCatalogFilter.vue";

const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });

function mountFilter() {
  return mount(ProviderCatalogFilter, {
    props: { searchQuery: "", activeCategory: "all" },
    global: { plugins: [i18n] },
  });
}

describe("ProviderCatalogFilter accessibility", () => {
  it("associates the search input with a label via id/for", () => {
    const wrapper = mountFilter();
    const input = wrapper.find("#catalog-search");
    expect(input.exists()).toBe(true);
    const label = wrapper.find('label[for="catalog-search"]');
    expect(label.exists()).toBe(true);
  });
});
