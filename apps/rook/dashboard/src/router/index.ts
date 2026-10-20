import type {RouteLocationNormalized, RouteRecordRaw} from "vue-router";
import {createRouter, createWebHistory} from "vue-router";
import {PROVIDER_KINDS} from "@/config/providerCatalog";
import {useAuthStore} from "@/stores/auth";

/**
 * Valid provider kinds — derived from the catalog so this list and
 * `providerCatalog.ts` can never drift apart. Adding a new kind to
 * the catalog automatically makes its route valid here.
 */
const VALID_PROVIDER_KINDS: readonly string[] = PROVIDER_KINDS.map(
  (p) => p.kind,
);

const routes: RouteRecordRaw[] = [
  {
    path: "/login",
    name: "Login",
    component: () => import("../views/LoginView.vue"),
    meta: {guestOnly: true},
  },
  {
    path: "/",
    component: () => import("../views/sidebar/index.vue"),
    meta: {requiresAuth: true},
    children: [
      {
        path: "",
        name: "Home",
        component: () => import("../views/HomeView.vue"),
      },
      {
        path: "endpoints",
        name: "Endpoints",
        component: () => import("../views/EndpointsView.vue"),
      },
      {
        path: "api-keys",
        name: "API Keys",
        component: () => import("../views/ApiKeysView.vue"),
      },
      {
        path: "api-keys/new",
        name: "API Keys Create",
        component: () => import("../views/ApiKeysView.vue"),
      },
      {
        // Provider section — nested so `route.matched.length === 3` on
        // detail/quota pages, which the breadcrumb uses to opt into a
        // 3-level rendering (Home → Providers → <sub-page>).
        path: "providers",
        meta: {title: "providers.catalog.title"},
        children: [
          {
            path: "",
            name: "Providers",
            component: () => import("../views/ProvidersView.vue"),
            meta: {title: "providers.catalog.title"},
          },
          {
            path: ":providerKind",
            name: "Provider Details",
            component: () => import("../views/ProviderDetailsView.vue"),
            meta: {
              title: "providers.details.title",
              // 3-level breadcrumb opt-in. The sidebar reads this and
              // resolves the last crumb from the `providerKind` param.
              breadcrumb: true,
            },
            beforeEnter: (to) => {
              const kind = to.params.providerKind;
              if (
                typeof kind !== "string" ||
                !VALID_PROVIDER_KINDS.includes(kind)
              ) {
                // Invalid kind — bounce back to the catalog so the user
                // sees a real page rather than a flash of an empty view.
                return {name: "Providers"};
              }
            },
          },
          {
            path: "quota",
            name: "Providers Quota",
            component: () => import("../views/ProvidersQuotaView.vue"),
            meta: {title: "providers.quota.title"},
          },
        ],
      },
      {
        path: "combos",
        name: "Combos",
        component: () => import("../views/CombosView.vue"),
      },
      {
        path: "settings",
        name: "Settings",
        component: () => import("../views/SettingsView.vue"),
      },
    ],
  },
];

export function getAuthRedirect(
  to: Pick<RouteLocationNormalized, "name" | "meta" | "matched">,
  isAuthenticated: boolean,
  bootstrapRequired: boolean,
): true | {name: string} {
  const requiresAuth = to.matched.some((r) => r.meta.requiresAuth);
  const guestOnly = to.matched.some((r) => r.meta.guestOnly);

  if (requiresAuth && !isAuthenticated) {
    return {name: "Login"};
  }

  if (guestOnly && isAuthenticated && !bootstrapRequired) {
    return {name: "Home"};
  }

  return true;
}

/**
 * Auth navigation handler — extracted so it can be unit-tested without a
 * live router or a real Pinia store. Accepts the auth store as a plain
 * object so callers can inject a mock in tests.
 *
 * Responsibilities:
 * 1. Lazy-load bootstrap status on the first navigation guard invocation.
 * 2. Surface bootstrap failures in the console but never block navigation.
 * 3. Delegate the actual redirect decision to {@link getAuthRedirect}.
 */
export async function handleAuthNavigation(
  to: Pick<RouteLocationNormalized, "name" | "meta" | "matched">,
  authStore: {
    initialized: boolean;
    isAuthenticated: boolean;
    bootstrapRequired: boolean;
    loadBootstrapStatus: () => Promise<void>;
  },
): Promise<true | {name: string}> {
  if (!authStore.initialized) {
    await authStore.loadBootstrapStatus().catch((err) => {
      console.error(
        "[router] bootstrap failed, allowing navigation to proceed",
        err,
      );
      authStore.initialized = true;
    });
  }

  return getAuthRedirect(
    to,
    authStore.isAuthenticated,
    authStore.bootstrapRequired,
  );
}

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes,
});

router.beforeEach(async (to) => handleAuthNavigation(to, useAuthStore()));

export default router;
