import { createRouter, createWebHistory } from 'vue-router'
import type { RouteRecordRaw } from 'vue-router'

const routes: RouteRecordRaw[] = [
  {
    path: '/',
    component: () => import('../views/sidebar/index.vue'),
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

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes,
})

export default router
