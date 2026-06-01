import { createRouter, createWebHistory } from 'vue-router'
import type { RouteLocationNormalized, RouteRecordRaw } from 'vue-router'
import { useAuthStore } from '@/stores/auth'

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
        path: 'providers',
        name: 'Providers',
        component: () => import('../views/ProvidersView.vue'),
      },
      {
        path: 'providers/quotes',
        name: 'Providers Quotes',
        component: () => import('../views/ProvidersView.vue'),
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
