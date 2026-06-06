import { createRouter, createWebHistory } from 'vue-router'
import type { RouteLocationNormalized, RouteRecordRaw } from 'vue-router'
import { useAuthStore } from '@/stores/auth'

/**
 * Provider kinds — hardcoded mirror of `apps/rook/dashboard/src/config/providerCatalog.ts`.
 *
 * We intentionally do NOT import `PROVIDER_KINDS` here: the router is
 * bootstrap code and must remain self-contained. Adding a new kind
 * requires updating BOTH the catalog AND this list. Drift would
 * surface as a 404 instead of a developer-facing error, which is the
 * expected (visible) failure mode.
 */
const VALID_PROVIDER_KINDS: readonly string[] = [
  'openai',
  'anthropic',
  'ollama',
  'gemini',
  'groq',
]

const routes: RouteRecordRaw[] = [
  {
    path: '/login',
    name: 'Login',
    component: () => import('../views/LoginView.vue'),
    meta: { guestOnly: true },
  },
  {
    path: '/',
    component: () => import('../views/sidebar/index.vue'),
    meta: { requiresAuth: true },
    children: [
      {
        path: '',
        name: 'Home',
        component: () => import('../views/HomeView.vue'),
      },
      {
        path: 'endpoints',
        name: 'Endpoints',
        component: () => import('../views/EndpointsView.vue'),
      },
      {
        path: 'api-keys',
        name: 'API Keys',
        component: () => import('../views/ApiKeysView.vue'),
      },
      {
        path: 'api-keys/new',
        name: 'API Keys Create',
        component: () => import('../views/ApiKeysView.vue'),
      },
      {
        // Provider section — nested so `route.matched.length === 3` on
        // detail/quota pages, which the breadcrumb uses to opt into a
        // 3-level rendering (Home → Providers → <sub-page>).
        path: 'providers',
        meta: { title: 'providers.catalog.title' },
        children: [
          {
            path: '',
            name: 'Providers',
            component: () => import('../views/ProvidersView.vue'),
            meta: { title: 'providers.catalog.title' },
          },
          {
            path: ':providerKind',
            name: 'Provider Details',
            component: () => import('../views/ProviderDetailsView.vue'),
            meta: {
              title: 'providers.details.title',
              // 3-level breadcrumb opt-in. The sidebar reads this and
              // resolves the last crumb from the `providerKind` param.
              breadcrumb: true,
            },
            beforeEnter: (to) => {
              const kind = to.params.providerKind
              if (typeof kind !== 'string' || !VALID_PROVIDER_KINDS.includes(kind)) {
                // Invalid kind — bounce back to the catalog so the user
                // sees a real page rather than a flash of an empty view.
                return { name: 'Providers' }
              }
            },
          },
          {
            path: 'quota',
            name: 'Providers Quota',
            component: () => import('../views/ProvidersQuotaView.vue'),
            meta: { title: 'providers.quota.title' },
          },
        ],
      },
      {
        path: 'combos',
        name: 'Combos',
        component: () => import('../views/CombosView.vue'),
      },
      {
        path: 'settings',
        name: 'Settings',
        component: () => import('../views/SettingsView.vue'),
      },
    ],
  },
]

export function getAuthRedirect(
  to: Pick<RouteLocationNormalized, 'name' | 'meta' | 'matched'>,
  isAuthenticated: boolean,
  bootstrapRequired: boolean,
): true | { name: string } {
  const requiresAuth = to.matched.some(r => r.meta.requiresAuth)
  const guestOnly = to.matched.some(r => r.meta.guestOnly)

  if (requiresAuth && !isAuthenticated) {
    return { name: 'Login' }
  }

  if (guestOnly && isAuthenticated && !bootstrapRequired) {
    return { name: 'Home' }
  }

  return true
}

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes,
})

router.beforeEach(async (to) => {
  const auth = useAuthStore()

  if (!auth.initialized) {
    await auth.loadBootstrapStatus().catch((err) => {
      console.error('[router] bootstrap failed, allowing navigation to proceed', err)
      auth.initialized = true
    })
  }

  return getAuthRedirect(to, auth.isAuthenticated, auth.bootstrapRequired)
})

export default router
