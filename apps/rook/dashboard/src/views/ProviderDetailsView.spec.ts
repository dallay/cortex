import {flushPromises, mount} from '@vue/test-utils'
import {createPinia, setActivePinia} from 'pinia'
import {describe, expect, it, vi, beforeEach} from 'vitest'
import {defineComponent} from 'vue'
import {createRouter, createMemoryHistory} from 'vue-router'
import {createI18n} from 'vue-i18n'
import en from '@/locales/en.json'
import ProviderDetailsView from './ProviderDetailsView.vue'

// ---------------------------------------------------------------------------
// Composable mocks — prevent real HTTP calls
// ---------------------------------------------------------------------------

vi.mock('@/composables/useProviders', () => ({
  useProviders: () => ({
    providers: {value: []},
    loading: {value: false},
    error: {value: null},
    fetch: vi.fn().mockResolvedValue(undefined),
    test: vi.fn().mockResolvedValue({ok: true}),
    update: vi.fn().mockResolvedValue(undefined),
    remove: vi.fn().mockResolvedValue(undefined),
  }),
}))

vi.mock('@/composables/useAvailableModels', () => ({
  useAvailableModels: () => ({
    modelsByProvider: {value: []},
    fetch: vi.fn().mockResolvedValue(undefined),
  }),
}))

vi.mock('@/composables/useProviderCatalog', () => ({
  useProviderCatalog: () => ({
    byKind: {value: new Map()},
  }),
}))

// ---------------------------------------------------------------------------
// Stub heavy child components
// ---------------------------------------------------------------------------

vi.mock('@/components/AddProviderDialog.vue', () => ({
  default: defineComponent({template: '<div data-testid="add-dialog" />'}),
}))
vi.mock('@/components/ConnectionListItem.vue', () => ({
  default: defineComponent({
    props: ['connection', 'models', 'testing', 'busy'],
    template: '<div data-testid="connection-item" />',
  }),
}))
vi.mock('@/components/LoadingState.vue', () => ({
  default: defineComponent({template: '<div data-testid="loading" />'}),
}))
vi.mock('@/components/ErrorBanner.vue', () => ({
  default: defineComponent({
    props: ['message'],
    emits: ['retry'],
    template: '<div data-testid="error-banner" />',
  }),
}))
vi.mock('@/components/EmptyState.vue', () => ({
  default: defineComponent({
    props: ['title'],
    template: '<div data-testid="empty-state"><slot name="actions" /></div>',
  }),
}))

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

function buildRouter(initialPath: string) {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      {
        path: '/providers',
        name: 'Providers',
        component: defineComponent({template: '<div />'}),
      },
      {
        path: '/providers/:providerKind',
        name: 'Provider Details',
        component: ProviderDetailsView,
        props: true,
      },
    ],
  })
  router.push(initialPath)
  return router
}

const i18n = createI18n({
  legacy: false,
  locale: 'en',
  messages: {en},
})

async function mountView(path: string) {
  setActivePinia(createPinia())
  const router = buildRouter(path)
  await router.isReady()
  const wrapper = mount(ProviderDetailsView, {
    global: {
      plugins: [router, i18n],
    },
  })
  await flushPromises()
  return {wrapper, router}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('ProviderDetailsView', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
  })

  it('renders the provider display name in the header', async () => {
    const {wrapper} = await mountView('/providers/openai')
    expect(wrapper.text()).toContain('OpenAI')
  })

  it('renders the title as an external link with correct href for openai', async () => {
    const {wrapper} = await mountView('/providers/openai')
    const link = wrapper.find('a[href^="https://"]')
    expect(link.exists()).toBe(true)
    expect(link.attributes('href')).toBe('https://platform.openai.com/api-keys')
    expect(link.attributes('target')).toBe('_blank')
    expect(link.attributes('rel')).toContain('noopener')
    expect(link.attributes('rel')).toContain('noreferrer')
  })

  it('announces the link opens in a new tab via aria-label', async () => {
    const {wrapper} = await mountView('/providers/ollama-cloud')
    const link = wrapper.find('a[href^="https://"]')
    expect(link.attributes('aria-label')).toBe('Ollama Cloud — opens in new tab')
  })

  it('renders a ProviderIcon in the detail header (Iconify svg for openai)', async () => {
    const {wrapper} = await mountView('/providers/openai')
    // openai uses the Iconify bundle path → renders an inline <svg>.
    // The component passes width=28 height=28 — find the svg with those attrs.
    const svg = wrapper.find('svg[width="28"]')
    expect(svg.exists()).toBe(true)
    expect(svg.attributes('height')).toBe('28')
  })

  it('redirects to /providers when the kind param is not in the catalog', async () => {
    const {router} = await mountView('/providers/not-a-real-kind')
    await flushPromises()
    expect(router.currentRoute.value.name).toBe('Providers')
  })
})
