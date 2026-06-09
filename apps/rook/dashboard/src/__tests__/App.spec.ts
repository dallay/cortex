import {mount} from "@vue/test-utils";
import {describe, expect, it} from "vitest";
import {createMemoryHistory, createRouter} from "vue-router";
import App from "../App.vue";

const router = createRouter({
  history: createMemoryHistory(),
  routes: [{path: "/", component: {template: "<div>home</div>"}}],
});

describe("App", () => {
  it("mounts and renders RouterView", async () => {
    const wrapper = mount(App, {global: {plugins: [router]}});
    await router.isReady();
    expect(wrapper.exists()).toBe(true);
  });
});
