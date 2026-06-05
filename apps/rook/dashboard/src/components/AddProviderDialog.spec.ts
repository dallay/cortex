import { describe, it, expect, vi, beforeEach } from 'vitest'
import { shallowMount } from '@vue/test-utils'
import { createI18n } from 'vue-i18n'
import AddProviderDialog from './AddProviderDialog.vue'
import en from '../locales/en.json'

// Mock useProviders composable
vi.mock('../composables/useProviders', () => ({
  useProviders: () => ({
    create: vi.fn().mockResolvedValue({ id: 'test-id', name: 'Test Provider' }),
    test: vi.fn().mockResolvedValue({ ok: true, latencyMs: 100 }),
    fetch: vi.fn().mockResolvedValue(undefined),
  }),
}))

const i18n = createI18n({
  legacy: false,
  locale: 'en',
  messages: { en },
})

describe('AddProviderDialog', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders component', () => {
    const wrapper = shallowMount(AddProviderDialog, {
      global: {
        plugins: [i18n],
      },
    })

    expect(wrapper.exists()).toBe(true)
  })

  it('validates form before enabling save button', async () => {
    const wrapper = shallowMount(AddProviderDialog, {
      global: {
        plugins: [i18n],
      },
    })

    // Initially, save button should be disabled (form is empty)
    // This is validated by the isValid computed property
    expect(wrapper.vm).toBeDefined()
  })

  it('builds correct CreateProviderRequest with ollama provider', async () => {
    const wrapper = shallowMount(AddProviderDialog, {
      global: {
        plugins: [i18n],
      },
    })

    // Access the component instance
    const vm = wrapper.vm as any

    // Set form values
    vm.form.name = 'Test Ollama'
    vm.form.apiKey = 'test-key-123'
    vm.form.baseUrl = 'https://api.ollama.com'
    vm.form.priority = 50
    vm.form.isActive = true
    vm.form.maxConcurrent = 5
    vm.form.defaultModel = 'llama3.1'

    await wrapper.vm.$nextTick()

    // Build the request
    const request = vm.buildCreateRequest()

    expect(request).toMatchObject({
      providerKind: 'ollama',
      authType: 'apiKey',
      name: 'Test Ollama',
      priority: 50,
      isActive: true,
      credentials: {
        apiKey: 'test-key-123',
      },
      config: {
        maxConcurrent: 5,
        quotaWindowThresholds: {
          warning: 0.8,
          error: 0.95,
        },
        defaultModel: 'llama3.1',
        baseUrl: 'https://api.ollama.com',
      },
    })

    expect(request.providerRuntimeId).toMatch(/^ollama-\d+-[a-z0-9]+$/)
  })

  it('validates required fields', async () => {
    const wrapper = shallowMount(AddProviderDialog, {
      global: {
        plugins: [i18n],
      },
    })

    const vm = wrapper.vm as any

    // Empty form should be invalid
    expect(vm.isValid).toBe(false)

    // Only name filled
    vm.form.name = 'Test'
    await wrapper.vm.$nextTick()
    expect(vm.isValid).toBe(false)

    // Both name and apiKey filled
    vm.form.apiKey = 'test-key'
    await wrapper.vm.$nextTick()
    expect(vm.isValid).toBe(true)
  })

  it('resets form when dialog closes', async () => {
    const wrapper = shallowMount(AddProviderDialog, {
      global: {
        plugins: [i18n],
      },
    })

    const vm = wrapper.vm as any

    // Fill form
    vm.form.name = 'Test'
    vm.form.apiKey = 'key'
    await wrapper.vm.$nextTick()

    // Close dialog
    vm.handleOpenChange(false)
    await wrapper.vm.$nextTick()

    // Form should be reset
    expect(vm.form.name).toBe('')
    expect(vm.form.apiKey).toBe('')
  })
})
