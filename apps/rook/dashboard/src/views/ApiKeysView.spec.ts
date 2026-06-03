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

vi.mock('@/components/ui/checkbox', () => ({
  Checkbox: defineComponent({
    props: { modelValue: Boolean, dataTestid: String },
    emits: ['update:modelValue'],
    setup(props, { emit }) {
      return () =>
        h('input', {
          type: 'checkbox',
          checked: props.modelValue === true,
          'data-testid': props.dataTestid ?? 'checkbox',
          onChange: (e: Event) =>
            emit('update:modelValue', (e.target as HTMLInputElement).checked),
        })
    },
  }),
}))

vi.mock('@/components/ui/badge', () => ({
  Badge: defineComponent({
    props: { variant: String, dataTestid: String },
    setup(props, { slots }) {
      return () => h('span', { 'data-testid': props.dataTestid ?? 'badge' }, slots.default?.())
    },
  }),
}))
vi.mock('@lucide/vue', () => {
  const icon = defineComponent({ setup: () => () => h('span') })
  return {
    Plus: icon, Copy: icon, Key: icon, AlertTriangle: icon, RefreshCw: icon,
    Pencil: icon, Trash2: icon, ShieldAlert: icon,
  }
})

// Mock useProviders to return empty providers list
vi.mock('@/composables/useProviders', () => ({
  useProviders: () => ({
    providers: ref([]),
    fetch: vi.fn(),
  }),
}))

vi.mock('@/composables/useAvailableModels', () => ({
  useAvailableModels: () => ({
    modelsByProvider: ref([]),
    groups: ref([]),
    loading: ref(false),
    error: ref(null),
    fetched: ref(false),
    fetch: vi.fn(),
    fetchProviders: vi.fn(),
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
  describe('Create modal — default state', () => {
    it('opens the modal with scopes pre-checked (all except admin)', async () => {
      const wrapper = makeWrapper()
      // Click the "Create API Key" button (first button in the page)
      const createButton = wrapper.findAll('button').find((b) => b.text().includes('Create API Key'))
      expect(createButton, 'create button should exist').toBeDefined()
      await createButton!.trigger('click')
      await wrapper.vm.$nextTick()

      // chat:read, chat:write, providers:read, providers:write → 4 checkboxes checked
      const chatRead = wrapper.find<HTMLInputElement>('[data-testid="scope-checkbox-chat-read"]')
      const chatWrite = wrapper.find<HTMLInputElement>('[data-testid="scope-checkbox-chat-write"]')
      const providersRead = wrapper.find<HTMLInputElement>('[data-testid="scope-checkbox-providers-read"]')
      const providersWrite = wrapper.find<HTMLInputElement>('[data-testid="scope-checkbox-providers-write"]')
      const admin = wrapper.find<HTMLInputElement>('[data-testid="scope-checkbox-admin"]')

      expect(chatRead.exists()).toBe(true)
      expect(chatRead.element.checked).toBe(true)
      expect(chatWrite.element.checked).toBe(true)
      expect(providersRead.element.checked).toBe(true)
      expect(providersWrite.element.checked).toBe(true)
      expect(admin.exists()).toBe(true)
      expect(admin.element.checked).toBe(false)
    })

    it('does not render the legacy free-text input for allowed models', async () => {
      const wrapper = makeWrapper()
      const createButton = wrapper.findAll('button').find((b) => b.text().includes('Create API Key'))
      await createButton!.trigger('click')
      await wrapper.vm.$nextTick()

      // The legacy form had inputs with id="create-allowed-models" or "edit-allowed-models".
      // The new form has no such input.
      expect(wrapper.find('input#create-allowed-models').exists()).toBe(false)
      expect(wrapper.find('input#edit-allowed-models').exists()).toBe(false)
    })

    it('renders helper text "Leave empty to allow all models" in the create form', async () => {
      const wrapper = makeWrapper()
      const createButton = wrapper.findAll('button').find((b) => b.text().includes('Create API Key'))
      await createButton!.trigger('click')
      await wrapper.vm.$nextTick()

      expect(wrapper.text()).toContain('Leave empty to allow all models')
      expect(wrapper.text()).toContain('Leave empty to allow all providers')
    })
  })

  describe('Edit modal', () => {
    it('pre-populates the form state with the existing key scopes and tier', async () => {
      mockApiKeys.value = [
        createApiKey({
          scopes: ['chat:read', 'providers:read'],
          tier: 'pro',
        }),
      ]
      mockTotal.value = 1

      const wrapper = makeWrapper()
      await wrapper.vm.$nextTick()

      // Open the edit modal by calling openEditModal directly.
      const vm = wrapper.vm as unknown as {
        openEditModal: (key: unknown) => void
        editForm: { label: string; scopes: string[]; tier: string }
      }
      vm.openEditModal(mockApiKeys.value[0])
      await wrapper.vm.$nextTick()

      // Verify the editForm state directly — this is what the shared
      // form receives via v-model. (We don't query the rendered DOM
      // because both the create and edit modals render simultaneously,
      // and the form-component tests already cover the rendering.)
      expect(vm.editForm.scopes).toEqual(['chat:read', 'providers:read'])
      expect(vm.editForm.tier).toBe('pro')
      expect(vm.editForm.label).toBe('test-key')
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
    it('requires at least one scope when all are unchecked', async () => {
      const wrapper = makeWrapper()
      const createButton = wrapper.findAll('button').find((b) => b.text().includes('Create API Key'))
      await createButton!.trigger('click')
      await wrapper.vm.$nextTick()

      // Set a valid label so we get past the first validation gate.
      const vm = wrapper.vm as unknown as { createForm: { label: string } }
      vm.createForm.label = 'valid-label'
      await wrapper.vm.$nextTick()

      // Uncheck the default-checked scopes so we end up with no scopes selected
      const chatRead = wrapper.find<HTMLInputElement>('[data-testid="scope-checkbox-chat-read"]')
      const chatWrite = wrapper.find<HTMLInputElement>('[data-testid="scope-checkbox-chat-write"]')
      const providersRead = wrapper.find<HTMLInputElement>('[data-testid="scope-checkbox-providers-read"]')
      const providersWrite = wrapper.find<HTMLInputElement>('[data-testid="scope-checkbox-providers-write"]')

      await chatRead.setValue(false)
      await wrapper.vm.$nextTick()
      await chatWrite.setValue(false)
      await wrapper.vm.$nextTick()
      await providersRead.setValue(false)
      await wrapper.vm.$nextTick()
      await providersWrite.setValue(false)
      await wrapper.vm.$nextTick()

      // Submit the form (form submit, not button click — the stub button
      // doesn't fire native form submit on click).
      const form = wrapper.find('form')
      await form.trigger('submit')
      await wrapper.vm.$nextTick()

      // The error block should appear with the validation message
      const errorBlock = wrapper.find('[data-testid="api-key-form-error"]')
      expect(errorBlock.exists()).toBe(true)
      expect(wrapper.text()).toContain('At least one scope is required')
    })
  })
})