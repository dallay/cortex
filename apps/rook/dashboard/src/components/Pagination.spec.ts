import {mount} from "@vue/test-utils";
import {describe, expect, it, vi} from "vitest";
import Pagination from "./Pagination.vue";

describe("Pagination", () => {
  // -------------------------------------------------------------------------
  // Page calculation
  // -------------------------------------------------------------------------

  it("shows Page 1 of 1 when offset=0, limit=20, total=5", () => {
    const wrapper = mount(Pagination, {
      props: {offset: 0, limit: 20, total: 5},
    });
    expect(wrapper.text()).toContain("Page 1 of 1");
  });

  it("shows Page 2 of 3 when offset=20, limit=20, total=50", () => {
    const wrapper = mount(Pagination, {
      props: {offset: 20, limit: 20, total: 50},
    });
    expect(wrapper.text()).toContain("Page 2 of 3");
  });

  it("shows Page 3 of 3 when offset=40, limit=20, total=50", () => {
    const wrapper = mount(Pagination, {
      props: {offset: 40, limit: 20, total: 50},
    });
    expect(wrapper.text()).toContain("Page 3 of 3");
  });

  // -------------------------------------------------------------------------
  // Prev button disabled/enabled state
  // -------------------------------------------------------------------------

  it("disables Previous button when hasPrev=false", () => {
    const wrapper = mount(Pagination, {
      props: {hasPrev: false},
    });
    const prevBtn = wrapper.findAll("button")[0];
    expect(prevBtn.attributes("disabled")).toBeDefined();
  });

  it("enables Previous button when hasPrev=true", () => {
    const wrapper = mount(Pagination, {
      props: {hasPrev: true},
    });
    const prevBtn = wrapper.findAll("button")[0];
    expect(prevBtn.attributes("disabled")).toBeUndefined();
  });

  it("calls onPrev when Previous is clicked", async () => {
    const onPrev = vi.fn();
    const wrapper = mount(Pagination, {
      props: {hasPrev: true, onPrev},
    });

    await wrapper.findAll("button")[0].trigger("click");
    expect(onPrev).toHaveBeenCalledTimes(1);
  });

  // -------------------------------------------------------------------------
  // Next button disabled/enabled state
  // -------------------------------------------------------------------------

  it("disables Next button when hasNext=false", () => {
    const wrapper = mount(Pagination, {
      props: {hasNext: false},
    });
    const nextBtn = wrapper.findAll("button")[1];
    expect(nextBtn.attributes("disabled")).toBeDefined();
  });

  it("enables Next button when hasNext=true", () => {
    const wrapper = mount(Pagination, {
      props: {hasNext: true},
    });
    const nextBtn = wrapper.findAll("button")[1];
    expect(nextBtn.attributes("disabled")).toBeUndefined();
  });

  it("calls onNext when Next is clicked", async () => {
    const onNext = vi.fn();
    const wrapper = mount(Pagination, {
      props: {hasNext: true, onNext},
    });

    await wrapper.findAll("button")[1].trigger("click");
    expect(onNext).toHaveBeenCalledTimes(1);
  });

  // -------------------------------------------------------------------------
  // Default props
  // -------------------------------------------------------------------------

  it("uses sensible defaults (Page 1 of 1, both buttons disabled)", () => {
    const wrapper = mount(Pagination, {});
    expect(wrapper.text()).toContain("Page 1 of 1");
    const buttons = wrapper.findAll("button");
    expect(buttons[0].attributes("disabled")).toBeDefined();
    expect(buttons[1].attributes("disabled")).toBeDefined();
  });
});
