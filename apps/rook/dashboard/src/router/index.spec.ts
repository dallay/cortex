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

  it("allows access to routes that do not require auth or guestOnly", async () => {
    const {getAuthRedirect} = await import("./index");

    const result = getAuthRedirect(
      {
        name: "Public",
        meta: {},
        matched: [{meta: {}}],
      } as unknown as RouteLocationNormalizedLoaded,
      false,
      false,
    );

    expect(result).toBe(true);
  });

  it("validates providerKind param in route beforeEnter", async () => {
    const routerModule = await import("./index");
    const router = routerModule.default;

    const providerDetailsRoute = router
      .getRoutes()
      .find((r) => r.name === "Provider Details");

    expect(providerDetailsRoute).toBeDefined();
    expect(providerDetailsRoute?.beforeEnter).toBeDefined();

    const beforeEnter = providerDetailsRoute?.beforeEnter as (
      to: Partial<RouteLocationNormalizedLoaded>,
    ) => unknown;

    // Valid provider kind
    const validResult = beforeEnter({
      params: {providerKind: "openai"},
    });
    expect(validResult).toBeUndefined();

    // Invalid provider kind
    const invalidResult = beforeEnter({
      params: {providerKind: "invalid-provider-kind"},
    });
    expect(invalidResult).toEqual({name: "Providers"});

    // Non-string or missing provider kind
    const missingResult = beforeEnter({
      params: {},
    });
    expect(missingResult).toEqual({name: "Providers"});
  });
});
