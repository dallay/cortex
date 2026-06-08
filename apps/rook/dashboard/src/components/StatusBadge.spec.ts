import {mount} from "@vue/test-utils";
import {describe, expect, it, vi} from "vitest";
import {defineComponent, h} from "vue";
import StatusBadge from "./StatusBadge.vue";

// Badge mock that forwards variant and class as data attributes so tests can
// assert on the styling decisions without needing the real shadcn component.
vi.mock("@/components/ui/badge", () => ({
  Badge: defineComponent({
    props: {variant: String, class: String},
    setup(props, {slots}) {
      return () =>
        h(
          "span",
          {
            "data-variant": props.variant,
            "data-testid": "badge",
            class: props.class ?? [],
          },
          slots.default?.(),
        );
    },
  }),
}));

describe("StatusBadge", () => {
  // -------------------------------------------------------------------------
  // variant — maps status strings to shadcn badge variants
  // -------------------------------------------------------------------------

  const activeVariants = ["active", "enabled", "connected"] as const;
  const inactiveVariants = ["inactive", "disabled", "disconnected"] as const;
  const errorVariants = ["error", "failed"] as const;

  for (const status of activeVariants) {
    it(`maps "${status}" → variant "default"`, () => {
      const wrapper = mount(StatusBadge, {props: {status}});
      expect(wrapper.find('[data-testid="badge"]').attributes("data-variant")).toBe(
        "default",
      );
    });
  }

  for (const status of inactiveVariants) {
    it(`maps "${status}" → variant "secondary"`, () => {
      const wrapper = mount(StatusBadge, {props: {status}});
      expect(wrapper.find('[data-testid="badge"]').attributes("data-variant")).toBe(
        "secondary",
      );
    });
  }

  for (const status of errorVariants) {
    it(`maps "${status}" → variant "destructive"`, () => {
      const wrapper = mount(StatusBadge, {props: {status}});
      expect(wrapper.find('[data-testid="badge"]').attributes("data-variant")).toBe(
        "destructive",
      );
    });
  }

  it('maps unknown status → variant "outline"', () => {
    const wrapper = mount(StatusBadge, {props: {status: "unknown-status"}});
    expect(wrapper.find('[data-testid="badge"]').attributes("data-variant")).toBe(
      "outline",
    );
  });

  // -------------------------------------------------------------------------
  // colorClass — maps status strings to Tailwind color classes
  // -------------------------------------------------------------------------

  for (const status of activeVariants) {
    it(`maps "${status}" → green color class`, () => {
      const wrapper = mount(StatusBadge, {props: {status}});
      const badge = wrapper.find('[data-testid="badge"]');
      expect(badge.classes()).toContain("bg-green-100");
      expect(badge.classes()).toContain("text-green-800");
    });
  }

  for (const status of inactiveVariants) {
    it(`maps "${status}" → gray color class`, () => {
      const wrapper = mount(StatusBadge, {props: {status}});
      const badge = wrapper.find('[data-testid="badge"]');
      expect(badge.classes()).toContain("bg-gray-100");
      expect(badge.classes()).toContain("text-gray-800");
    });
  }

  for (const status of errorVariants) {
    it(`maps "${status}" → no green/gray class (uses destructive variant)`, () => {
      const wrapper = mount(StatusBadge, {props: {status}});
      const badge = wrapper.find('[data-testid="badge"]');
      // destructive variant already provides red styling — no extra color class needed
      expect(badge.classes()).not.toContain("bg-green-100");
      expect(badge.classes()).not.toContain("bg-gray-100");
    });
  }

  it('maps unknown status → no extra color class', () => {
    const wrapper = mount(StatusBadge, {props: {status: "foobar"}});
    const badge = wrapper.find('[data-testid="badge"]');
    expect(badge.classes()).not.toContain("bg-green-100");
    expect(badge.classes()).not.toContain("bg-gray-100");
  });

  // -------------------------------------------------------------------------
  // Case-insensitive matching
  // -------------------------------------------------------------------------

  it("is case-insensitive — maps 'ACTIVE' to green", () => {
    const wrapper = mount(StatusBadge, {props: {status: "ACTIVE"}});
    const badge = wrapper.find('[data-testid="badge"]');
    expect(badge.classes()).toContain("bg-green-100");
  });

  // -------------------------------------------------------------------------
  // Text content
  // -------------------------------------------------------------------------

  it("renders the status text inside the badge", () => {
    const wrapper = mount(StatusBadge, {props: {status: "connected"}});
    expect(wrapper.text()).toBe("connected");
  });
});
