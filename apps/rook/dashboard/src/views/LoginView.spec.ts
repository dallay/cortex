import {mount} from "@vue/test-utils";
import {createPinia, setActivePinia} from "pinia";
import {describe, expect, it, vi} from "vitest";
import {defineComponent, h} from "vue";
import {createI18n} from "vue-i18n";
import {createMemoryHistory, createRouter} from "vue-router";
import en from "@/locales/en.json";
import {useAuthStore} from "@/stores/auth";
import LoginView from "./LoginView.vue";

// Stub heavy UI primitives that pull in class-variance-authority
vi.mock("@/components/ui/button", () => ({
  Button: defineComponent({
    props: {disabled: Boolean},
    setup(props, {slots}) {
      return () =>
        h(
          "button",
          {type: "submit", disabled: props.disabled},
          slots.default?.(),
        );
    },
  }),
}));
vi.mock("@/components/ui/input", () => ({
  Input: defineComponent({
    props: {
      id: String,
      type: String,
      modelValue: String,
      class: String,
      autocomplete: String,
      required: Boolean,
    },
    emits: ["update:modelValue"],
    setup(props) {
      return () => h("input", {id: props.id, type: props.type ?? "text"});
    },
  }),
}));
vi.mock("@/components/ui/label", () => ({
  Label: defineComponent({
    props: {for: String},
    setup(_, {slots}) {
      return () => h("label", slots.default?.());
    },
  }),
}));
vi.mock("@lucide/vue", () => {
  const icon = defineComponent({setup: () => () => h("span")});
  return {Lock: icon, Eye: icon, EyeOff: icon, ShieldCheck: icon};
});

const i18n = createI18n({legacy: false, locale: "en", messages: {en}});

const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    {path: "/", name: "Home", component: {template: "<div/>"}},
    {path: "/login", name: "Login", component: LoginView},
  ],
});

describe("LoginView", () => {
  it("renders first-time admin setup when bootstrap is required", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const auth = useAuthStore();
    auth.bootstrapRequired = true;

    const wrapper = mount(LoginView, {
      global: {plugins: [pinia, router, i18n]},
    });

    expect(wrapper.get("h1").text()).toContain("Set admin password");
    expect(wrapper.text()).toContain(
      "You must set the admin password before using Rook",
    );
    expect(wrapper.get("input#setup-password").attributes("type")).toBe(
      "password",
    );
    expect(wrapper.get('button[type="submit"]').text()).toContain(
      "Set password",
    );
  });

  it("renders a token input field in bootstrap mode so user can paste the setup token from server logs", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const auth = useAuthStore();
    auth.bootstrapRequired = true;

    const wrapper = mount(LoginView, {
      global: {plugins: [pinia, router, i18n]},
    });

    // The setup token is out-of-band — printed to server logs at startup.
    // User must paste it into the form manually.
    expect(wrapper.get("input#setup-token")).toBeTruthy();
  });

  it("renders normal admin login when bootstrap is complete", async () => {
    const pinia = createPinia();
    setActivePinia(pinia);
    const auth = useAuthStore();
    auth.bootstrapRequired = false;

    const wrapper = mount(LoginView, {
      global: {plugins: [pinia, router, i18n]},
    });

    expect(wrapper.get("h1").text()).toContain("Admin login");
    expect(wrapper.text()).toContain("Sign in with the admin password");
    expect(wrapper.get("input#login-password").attributes("type")).toBe(
      "password",
    );
  });
});
