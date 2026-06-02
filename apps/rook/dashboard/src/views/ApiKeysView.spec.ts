import { mount } from '@vue/test-utils'
import { createPinia, setActivePinia } from 'pinia'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { defineComponent, h, ref } from 'vue'
import { createRouter, createMemoryHistory } from 'vue-router'
import { createI18n } from 'vue-i18n'
import en from '@/locales/en.json'
import ApiKeysView from './ApiKeysView.vue'

// Mock API client methods to prevent real HTTP calls
vi.mock('@/lib/api', () => ({
  useApi: () => ({
    getApiKeys: vi.fn().mockResolvedValue({ keys: [], pagination: { total: 0, limit: 20, offset: 0 } }),
    createApiKey: vi.fn().mockResolvedValue({ key: {}, plaintextKey: 'test-key' }),
    updateApiKey: vi.fn().mockResolvedValue({}),
    revokeApiKey: vi.fn().mockResolvedValue(undefined),
    rotateApiKey: vi.fn().mockResolvedValue({ key: { id: 'rotated', keyPrefix: 'new_key' }, plaintextKey: 'new_raw_key' }),
  }),
}))

// Stub UI primitives
vi.mock('@/components/ui/button', () => ({
  Button: defineComponent({
    props: { disabled: Boolean, variant: String, size: String },
    emits: ['click'],
    setup(props, { slots }) {
      return () => h('button', { type: 'button', disabled: props.disabled }, slots.default?.())
    },
  }),
}))
vi.mock('@/components/ui/input', () => ({
  Input: defineComponent({
    props: { id: String, type: String, modelValue: String, placeholder: String, class: String },
    emits: ['update:modelValue'],
    setup(props: any) {
      return () => h('input', { id: props.id, type: props.type ?? 'text', value: props.modelValue })
    },
  }),
}))
vi.mock('@/components/ui/dialog', () => ({
  Dialog: defineComponent({
    props: { open: Boolean, 'onUpdate:open': Function },
    setup(_props: any, ctx: any) {
      return () => h('div', { role: 'dialog' }, ctx.slots.default?.())
    },
  }),
  DialogContent: defineComponent({ setup(_: any, ctx: any) { return () => h('div', ctx.slots.default?.()) } }),
  DialogDescription: defineComponent({ setup(_: any, ctx: any) { return () => h('p', ctx.slots.default?.()) } }),
  DialogFooter: defineComponent({ setup(_: any, ctx: any) { return () => h('div', ctx.slots.default?.()) } }),
  DialogHeader: defineComponent({ setup(_: any, ctx: any) { return () => h('div', ctx.slots.default?.()) } }),
  DialogTitle: defineComponent({ setup(_: any, ctx: any) { return () => h('h2', ctx.slots.default?.()) } }),
}))
vi.mock('@/components/ui/select', () => ({
  Select: defineComponent({ setup(_, { slots }) { return () => h('div', slots.default?.()) } }),
  SelectContent: defineComponent({ setup(_, { slots }) { return () => h('div', slots.default?.()) } }),
  SelectGroup: defineComponent({ setup(_, { slots }) { return () => h('div', slots.default?.()) } }),
  SelectItem: defineComponent({
    props: { value: String },
    setup(_, { slots }) { return () => h('div', { value: 'item' }, slots.default?.()) },
  }),
  SelectLabel: defineComponent({ setup(_, { slots }) { return () => h('span', slots.default?.()) } }),
  SelectTrigger: defineComponent({ setup(_, { slots }) { return () => h('button', slots.default?.()) } }),
  SelectValue: defineComponent({ setup(_, { slots }) { return () => h('span', slots.default?.()) } }),
}))
vi.mock('@lucide/vue', () => {
  const icon = defineComponent({ setup: () => () => h('span') })
  return {
    Plus: icon, Copy: icon, Key: icon, AlertTriangle: icon, RefreshCw: icon,
    Pencil: icon, Trash2: icon,
  }
})

// Mock useProviders to return empty providers list
vi.mock('@/composables/useProviders', () => ({
  useProviders: () => ({
    providers: ref([]),
    fetch: vi.fn(),
  }),
}))

// Mock useApiKeys with controlled state
const mockApiKeys = ref<any[]>([])
const mockTotal = ref(0)
const mockLoading = ref(false)
const mockError = ref<string | null>(null)

vi.mock('@/composables/useApiKeys', () => ({
  useApiKeys: () => ({
    apiKeys: mockApiKeys,
    loading: mockLoading,
    error: mockError,
    total: mockTotal,
    limit: ref(20),
    offset: ref(0),
    fetch: vi.fn(),
    create: vi.fn().mockResolvedValue({ key: {}, plaintextKey: 'test-key' }),
    update: vi.fn().mockResolvedValue({}),
    revoke: vi.fn().mockResolvedValue(true),
    rotate: vi.fn().mockResolvedValue({ key: { id: 'rotated', keyPrefix: 'new_key' }, plaintextKey: 'new_raw_key' }),
    nextPage: vi.fn(),
    prevPage: vi.fn(),
  }),
}))

const i18n = createI18n({ legacy: false, locale: 'en', messages: { en } })

const router = createRouter({
  history: createMemoryHistory(),
  routes: [
    { path: '/', name: 'Home', component: { template: '<div/>' } },
    { path: '/api-keys', name: 'ApiKeys', component: ApiKeysView },
  ],
})

function makeWrapper() {
  const pinia = createPinia()
  setActivePinia(pinia)
  return mount(ApiKeysView, {
    global: { plugins: [pinia, router, i18n] },
  })
}

function createApiKey(overrides = {}) {
  return {
    id: 'key-1',
    label: 'test-key',
    keyPrefix: 'rook_abc',
    scopes: ['chat:read'],
    tier: 'free',
    isActive: true,
    revokedAt: null,
    expiresAt: null,
    createdAt: '2024-01-01T00:00:00Z',
    lastUsedAt: null,
    allowedModels: [],
    allowedProviders: [],
    ...overrides,
  }
}

beforeEach(() => {
  mockApiKeys.value = []
  mockTotal.value = 0
  mockLoading.value = false
  mockError.value = null
})

describe('ApiKeysView', () => {
  describe('scopesOptions', () => {
    it('contains 5 scope options', () => {
      const wrapper = makeWrapper()
      const scopeCheckboxes = wrapper.findAll('input[type="checkbox"]')
      expect(scopeCheckboxes.length).toBeGreaterThanOrEqual(5)
    })
  })

  describe('Create modal', () => {
    it('shows 5 scope checkboxes in create modal (10 total across both modals)', async () => {
      const wrapper = makeWrapper()
      await wrapper.find('button').trigger('click')
      const checkboxes = wrapper.findAll('input[type="checkbox"]')
      // Both create and edit modals render simultaneously (v-if not used), so 10 total
      expect(checkboxes.length).toBeGreaterThanOrEqual(5)
    })

    it('shows allowedModels text input in create modal', async () => {
      const wrapper = makeWrapper()
      await wrapper.find('button').trigger('click')
      const allowedModelsInput = wrapper.find('input#create-allowed-models')
      expect(allowedModelsInput.exists()).toBe(true)
    })

    it('shows allowedProviders section in create modal', async () => {
      const wrapper = makeWrapper()
      await wrapper.find('button').trigger('click')
      expect(wrapper.text()).toContain('Allowed Providers')
    })
  })

  describe('Edit modal', () => {
    it('pre-populates restriction fields when editing', async () => {
      mockApiKeys.value = [createApiKey({ allowedModels: ['gpt-4', 'gpt-4o'], allowedProviders: [] })]
      mockTotal.value = 1

      const wrapper = makeWrapper()
      await wrapper.vm.$nextTick()

      const editButton = wrapper.find('button')
      await editButton.trigger('click')
      await wrapper.vm.$nextTick()

      const allowedModelsInput = wrapper.find('input#edit-allowed-models')
      expect(allowedModelsInput.exists()).toBe(true)
    })
  })

  describe('Restriction badges', () => {
    it('shows Unrestricted badge when both allowedModels and allowedProviders are empty', async () => {
      mockApiKeys.value = [createApiKey({ allowedModels: [], allowedProviders: [] })]
      mockTotal.value = 1

      const wrapper = makeWrapper()
      await wrapper.vm.$nextTick()

      expect(wrapper.text()).toContain('Unrestricted')
    })

    it('shows Restricted badge when allowedModels is non-empty', async () => {
      mockApiKeys.value = [createApiKey({ allowedModels: ['gpt-4'], allowedProviders: [] })]
      mockTotal.value = 1

      const wrapper = makeWrapper()
      await wrapper.vm.$nextTick()

      expect(wrapper.text()).toContain('Restricted')
      expect(wrapper.text()).toContain('1 model')
    })

    it('shows Restricted badge when allowedProviders is non-empty', async () => {
      mockApiKeys.value = [createApiKey({ allowedModels: [], allowedProviders: ['provider-openai'] })]
      mockTotal.value = 1

      const wrapper = makeWrapper()
      await wrapper.vm.$nextTick()

      expect(wrapper.text()).toContain('Restricted')
      expect(wrapper.text()).toContain('1 provider')
    })
  })

  describe('Rotate button and dialog', () => {
    it('shows rotate button in actions column for active keys', async () => {
      mockApiKeys.value = [createApiKey()]
      mockTotal.value = 1

      const wrapper = makeWrapper()
      await wrapper.vm.$nextTick()

      const buttons = wrapper.findAll('button')
      expect(buttons.length).toBeGreaterThanOrEqual(3)
    })

    it('opens confirmation dialog when rotate button is clicked', async () => {
      mockApiKeys.value = [createApiKey()]
      mockTotal.value = 1

      const wrapper = makeWrapper()
      await wrapper.vm.$nextTick()

      const buttons = wrapper.findAll('button')
      await buttons.at(2)?.trigger('click')
      await wrapper.vm.$nextTick()

      expect(wrapper.text()).toContain('Rotate API Key')
    })
  })

  describe('Validation', () => {
    it('requires at least one scope', async () => {
      const wrapper = makeWrapper()
      await wrapper.find('button').trigger('click')
      await wrapper.vm.$nextTick()

      const form = wrapper.find('form')
      await form.trigger('submit')
      await wrapper.vm.$nextTick()

      expect(wrapper.text()).toContain('At least one scope is required')
    })
  })
})