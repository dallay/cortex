import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath, URL } from 'node:url'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
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
    proxy: {
      '/api/': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
      '/health': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
      '/login': {
        target: 'http://localhost:8080',
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
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
})