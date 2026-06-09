import {createPinia, setActivePinia} from "pinia";
import {beforeEach, describe, expect, it, vi} from "vitest";
import type {RouteLocationNormalizedLoaded} from "vue-router";

type GuardResult = boolean | string | { name: string };

describe("auth router guard", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    vi.resetModules();
  });

  it("redirects protected routes to login when the session is missing", async () => {
    const {getAuthRedirect} = await import("./index");

    const result = getAuthRedirect(
      {
        name: "API Keys",
        meta: {},
        matched: [{meta: {requiresAuth: true}}, {meta: {}}],
      } as unknown as RouteLocationNormalizedLoaded,
      false,
      false,
    );

    expect(result).toEqual({name: "Login"});
  });

  it("redirects initialized login visits back home when already authenticated", async () => {
    const {getAuthRedirect} = await import("./index");

    const result: GuardResult = getAuthRedirect(
      {
        name: "Login",
        meta: {guestOnly: true},
        matched: [{meta: {guestOnly: true}}],
      } as unknown as RouteLocationNormalizedLoaded,
      true,
      false,
    );

    expect(result).toEqual({name: "Home"});
  });

  it("keeps unauthenticated users on login while bootstrap setup is required", async () => {
    const {getAuthRedirect} = await import("./index");

    const result = getAuthRedirect(
      {
        name: "Login",
        meta: {guestOnly: true},
        matched: [{meta: {guestOnly: true}}],
      } as unknown as RouteLocationNormalizedLoaded,
      false,
      true,
    );

    expect(result).toBe(true);
  });
});
