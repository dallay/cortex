import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath, URL } from 'node:url'
import { codecovVitePlugin } from '@codecov/vite-plugin'

// Backend target for the Vite dev server proxy. Override via the API_TARGET
// env var (e.g. set it in dev/e2e/run-api-keys-e2e.sh when the backend is on
// a non-default port like 8081).
const API_TARGET = process.env.API_TARGET ?? 'http://127.0.0.1:3773'

// Base path: serve dashboard from /dashboard/ prefix
// This ensures assets are generated with correct /dashboard/assets/ paths
const BASE_PATH = process.env.BASE_PATH ?? '/dashboard/'

export default defineConfig({
  base: BASE_PATH,
  plugins: [
    vue(),
    tailwindcss(),
    // Put the Codecov vite plugin after all other plugins
    codecovVitePlugin({
      enableBundleAnalysis: process.env.CODECOV_TOKEN !== undefined,
      bundleName: 'rook-dashboard',
      uploadToken: process.env.CODECOV_TOKEN,
    }),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url)),
    },
  },
  test: {
    environment: 'jsdom',
    // transform these packages so vitest can handle their ESM exports
    server: {
      deps: {
        inline: ['class-variance-authority', 'clsx', 'tailwind-merge', 'reka-ui'],
      },
    },
  },
  server: {
    port: 4747,
    proxy: {
      '/api/': {
        target: API_TARGET,
        changeOrigin: true,
      },
      '/health': {
        target: API_TARGET,
        changeOrigin: true,
      },
      '/login': {
        target: API_TARGET,
        changeOrigin: true,
        bypass(req) {
          // Browser navigation sends Accept: text/html — serve the SPA so
          // Vue Router handles the /login route. XHR/fetch calls (CSRF token
          // retrieval) send Accept: */* and must reach the backend.
          const accept = req.headers['accept'] ?? ''
          if (req.method === 'GET' && accept.includes('text/html')) {
            return '/index.html'
          }
          return null
        },
      },
      '/logout': {
        target: API_TARGET,
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
})
