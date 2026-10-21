<div align="center">

# 📊 Rook Dashboard

Vue 3 Management Interface for Rook AI Proxy

[![Vue](https://img.shields.io/github/package-json/dependency-version/dallay/cortex/vue/main?filename=apps%2Frook%2Fdashboard%2Fpackage.json&logo=vuedotjs&color=4FC08D)](https://vuejs.org)
[![TypeScript](https://img.shields.io/github/package-json/dependency-version/dallay/cortex/dev/typescript/main?filename=apps%2Frook%2Fdashboard%2Fpackage.json&logo=typescript&color=3178C6)](https://www.typescriptlang.org)
[![Vite](https://img.shields.io/github/package-json/dependency-version/dallay/cortex/dev/vite/main?filename=apps%2Frook%2Fdashboard%2Fpackage.json&logo=vite&color=646CFF)](https://vitejs.dev)
[![Vitest](https://img.shields.io/github/package-json/dependency-version/dallay/cortex/dev/vitest/main?filename=apps%2Frook%2Fdashboard%2Fpackage.json&logo=vitest&color=6E9F18)](https://vitest.dev)
[![Playwright](https://img.shields.io/github/package-json/dependency-version/dallay/cortex/dev/@playwright%2Ftest/main?filename=apps%2Frook%2Fdashboard%2Fpackage.json&logo=playwright&color=2EAD33)](https://playwright.dev)
[![pnpm](https://img.shields.io/static/v1?label=pnpm&message=12.14.0&color=C867F2&logo=pnpm)](https://pnpm.io)
<!-- pnpm version from root package.json -->
[![License](https://img.shields.io/github/license/dallay/cortex)](https://github.com/dallay/cortex/blob/main/LICENSE)

</div>

---

## 🎯 What is the Dashboard?

The Rook Dashboard is a **Vue 3 SPA** that provides a visual interface for managing your Rook AI Proxy instance.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                        Rook Dashboard Architecture                        │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                         │
│   Browser (localhost:4747)                                             │
│        │                                                                │
│        │  Vue 3 SPA                                                     │
│        ▼                                                                │
│   ┌─────────────────────────────────────────────────────────────┐      │
│   │  Views                    │  Components                     │      │
│   │  ─────────────────────    │  ─────────────────────         │      │
│   │  • Login                  │  • ProviderCard                 │      │
│   │  • Dashboard              │  • ApiKeyTable                  │      │
│   │  • Providers              │  • ComboEditor                  │      │
│   │  • API Keys               │  • AuditLog                     │      │
│   │  • Settings               │  • ChatInterface                │      │
│   └─────────────────────────────────────────────────────────────┘      │
│        │                                                                │
│        │  Vite Proxy (/api/* → port 3773)                             │
│        ▼                                                                │
│   ┌─────────────────────────────────────────────────────────────┐      │
│   │                     Rook Backend (port 3773)                │      │
│   └─────────────────────────────────────────────────────────────┘      │
│                                                                         │
└─────────────────────────────────────────────────────────────────────────┘
```

### Features

| Feature | Description |
|---------|-------------|
| 🔐 **Authentication** | Secure login with Rook's admin credentials |
| 🌍 **Provider Management** | Configure OpenAI, Anthropic, Gemini, Ollama, Groq |
| 🔑 **API Key CRUD** | Create, view, and revoke API keys |
| 🔄 **Combo Editor** | Visual builder for fallback chains |
| 📊 **Audit Logs** | Searchable request/response history |
| 💬 **Chat Interface** | Test requests directly in the browser |
| 🌐 **i18n** | English and Spanish translations |

---

## 🚀 Quick Start

### Prerequisites

| Requirement | Version |
|-------------|---------|
| Node.js | 24+ |
| pnpm | latest |
| Running Rook backend | port 3773 |

### Setup

```bash
# Install dependencies (from repo root)
pnpm install

# Or from this directory
cd apps/rook/dashboard
pnpm install
```

### Development

```bash
# Option 1: Using just (recommended)
just run-dashboard

# Option 2: Manual
cd apps/rook/dashboard
pnpm dev
```

Then open [http://localhost:4747/dashboard/](http://localhost:4747/dashboard/)

The dev server automatically proxies:
- `/api/*` → `http://127.0.0.1:3773`
- `/health` → `http://127.0.0.1:3773`

### Custom Configuration

```bash
# Different backend URL
API_TARGET=http://127.0.0.1:8081 pnpm dev

# Custom base path
BASE_PATH=/custom-path pnpm dev
```

---

## 📁 Project Structure

```
apps/rook/dashboard/
├── src/
│   ├── views/           # 📄 Page components
│   │   ├── LoginView.vue
│   │   ├── HomeView.vue
│   │   ├── ProvidersView.vue
│   │   ├── ApiKeysView.vue
│   │   └── SettingsView.vue
│   │
│   ├── components/     # 🧩 Reusable UI components
│   │   ├── common/     # Buttons, inputs, cards
│   │   ├── providers/  # Provider-specific components
│   │   ├── api-keys/   # API key management
│   │   └── audit/      # Audit log components
│   │
│   ├── stores/         # 📦 Pinia state management
│   │   ├── auth.ts
│   │   ├── providers.ts
│   │   └── audit.ts
│   │
│   ├── router/        # 🛤️ Vue Router configuration
│   │   └── index.ts
│   │
│   ├── locales/        # 🌐 i18n translations
│   │   ├── en.json
│   │   └── es.json
│   │
│   ├── lib/           # 🔌 Backend API client
│   │   └── api.ts
│   │
│   └── App.vue        # Root component
│
├── e2e/               # 🧪 Playwright E2E tests
│   ├── login.spec.ts
│   ├── providers.spec.ts
│   ├── api-keys.spec.ts
│   └── vue.spec.ts
│
├── vite.config.ts     # ⚡ Vite configuration
├── playwright.config.ts # 🎭 Playwright configuration
├── tsconfig.json      # 📘 TypeScript config
└── package.json       # 📦 Dependencies
```

---

## 🧪 Testing

### Unit Tests

```bash
cd apps/rook/dashboard

# Run all tests
pnpm exec vitest run

# Watch mode
pnpm exec vitest

# With coverage
pnpm exec vitest run --coverage
```

### Type Checking

```bash
pnpm typecheck
```

### Build

```bash
pnpm build
```

### End-to-End Tests

E2E tests require Docker and a running Rook backend:

```bash
# Full E2E suite
just test-e2e

# Manual run
just test-e2e-build  # Build Docker image
just test-e2e        # Run tests
```

📖 [E2E Testing Guide](../../../dev/README.md#path-2--playwright-e2e-against-docker)

### Quality Gate

```bash
# Run all checks (CI simulation)
just ci-local
```

---

## 🎨 Development

### Type Check

```bash
pnpm typecheck
```

### Adding a New View

```typescript
// src/views/NewFeatureView.vue
<script setup lang="ts">
// Your component logic
</script>

<template>
  <div class="new-feature">
    <!-- Template -->
  </div>
</template>
```

### Adding a Component

```typescript
// src/components/common/MyButton.vue
<script setup lang="ts">
interface Props {
  variant?: 'primary' | 'secondary'
}
defineProps<Props>()
</script>

<template>
  <button :class="['my-button', variant]">
    <slot />
  </button>
</template>
```

---

## 🔧 Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `API_TARGET` | `http://127.0.0.1:3773` | Rook backend URL |
| `BASE_PATH` | `/dashboard` | Application base path |
| `VITE_APP_TITLE` | `Rook Dashboard` | Browser tab title |

---

## 📚 Documentation

| Resource | Description |
|----------|-------------|
| [Rook Docs](docs/rook/README.md) | Rook proxy documentation |
| [API Reference](docs/rook/api.md) | Backend API endpoints |
| [Configuration](docs/rook/configuration.md) | Rook config options |
| [Dev Guide](dev/README.md) | Local testing with Docker |

---

## 🤝 Contributing

See [Contributing Guide](CONTRIBUTING.md) in the repository root.

---

## 📄 License

MIT License - same as [Cortex repository](https://github.com/dallay/cortex).

---

<div align="center">

**Vue 3** · **TypeScript** · **Pinia** · **Vue Router** · **i18n**

</div>
