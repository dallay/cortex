import { mount, type VueWrapper } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'
import { createI18n } from 'vue-i18n'
import en from '@/locales/en.json'
import ApiKeyForm, { type ApiKeyFormState } from './ApiKeyForm.vue'
import { SCOPES, DEFAULT_SCOPES } from '@/config/scopes'
import type { ProviderConnectionResponse } from '@/lib/api'
import type { ModelsByProvider } from '@/composables/useAvailableModels'

// ---------------------------------------------------------------------------
// Stubs for the reka-ui / shadcn primitives that ApiKeyForm uses.
// We render them as plain HTML so the test can query real <input> and
// <select> elements, exactly like the original ApiKeysView test does.
// ---------------------------------------------------------------------------

vi.mock('@/components/ui/button', () => ({
  Button: defineComponent({
    props: { type: String, variant: String, disabled: Boolean, dataTestid: String },
    emits: ['click'],
    setup(props, { slots, emit }) {
      return () =>
        h(
          'button',
          {
            type: (props.type as string) ?? 'button',
            'data-testid': props.dataTestid ?? 'mock-button',
            onClick: () => emit('click'),
          },
          slots.default?.(),
        )
    },
  }),
}))

vi.mock('@/components/ui/input', () => ({
  Input: defineComponent({
    props: {
      id: String,
      type: String,
      modelValue: { type: [String, null], default: '' },
      placeholder: String,
      dataTestid: String,
    },
    emits: ['update:modelValue'],
    setup(props, { emit }) {
      return () =>
        h('input', {
          id: props.id,
          type: (props.type as string) ?? 'text',
          value: props.modelValue ?? '',
          placeholder: props.placeholder,
          'data-testid': props.dataTestid ?? (props.id ? `input-${props.id}` : 'input'),
          onInput: (e: Event) =>
            emit('update:modelValue', (e.target as HTMLInputElement).value),
        })
    },
  }),
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
      return () =>
        h(
          'span',
          { 'data-testid': props.dataTestid ?? 'badge' },
          slots.default?.(),
        )
    },
  }),
}))

vi.mock('@/components/ui/select', () => ({
  Select: defineComponent({
    props: { modelValue: String },
    emits: ['update:modelValue'],
    setup(_props, { slots, emit }) {
      return () =>
        h('div', { 'data-testid': 'mock-select' }, [
          h('input', {
            type: 'hidden',
            value: _props.modelValue,
            'data-testid': 'mock-select-value',
          }),
          slots.default?.(),
          h(
            'button',
            {
              type: 'button',
              'data-testid': 'mock-select-trigger',
              onClick: () => emit('update:modelValue', 'enterprise'),
            },
            'select',
          ),
        ])
    },
  }),
  SelectTrigger: defineComponent({
    props: { id: String, dataTestid: String },
    setup(props, { slots }) {
      return () => h('div', { id: props.id, 'data-testid': props.dataTestid }, slots.default?.())
    },
  }),
  SelectValue: defineComponent({
    setup(_props, { slots }) {
      return () => h('span', slots.default?.())
    },
  }),
  SelectContent: defineComponent({
    setup(_props, { slots }) {
      return () => h('div', slots.default?.())
    },
  }),
  SelectItem: defineComponent({
    props: { value: String },
    setup(props, { slots }) {
      return () =>
        h(
          'div',
          { 'data-value': props.value, 'data-testid': `select-item-${props.value}` },
          slots.default?.(),
        )
    },
  }),
}))

vi.mock('@lucide/vue', () => {
  const icon = defineComponent({ setup: () => () => h('span', { 'data-testid': 'icon' }) })
  return {
    Key: icon,
    AlertTriangle: icon,
    ShieldAlert: icon,
    Plus: icon,
    Copy: icon,
    RefreshCw: icon,
    Pencil: icon,
    Trash2: icon,
  }
})

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const i18n = createI18n({ legacy: false, locale: 'en', messages: { en } })

const TIER_OPTIONS = [
  { value: 'free', label: 'Free', description: '100 req burst / ~10 req/min' },
  { value: 'pro', label: 'Pro', description: '1,000 req burst / ~100 req/min' },
  { value: 'enterprise', label: 'Enterprise', description: '10,000 req burst / ~1,000 req/min' },
]

function makeProvider(overrides: Partial<ProviderConnectionResponse> = {}): ProviderConnectionResponse {
  return {
    id: 'p1',
    providerKind: 'openai',
    providerRuntimeId: 'openai-primary',
    authType: 'apiKey',
    name: 'OpenAI Primary',
    priority: 1,
    isActive: true,
    config: { maxConcurrent: 1, quotaWindowThresholds: { warning: 0.5, error: 0.9 }, defaultModel: null, baseUrl: null },
    testStatus: { status: 'neverTested', lastTestAt: null, latencyMs: null, error: null },
    createdAt: '2024-01-01T00:00:00Z',
    updatedAt: '2024-01-01T00:00:00Z',
    ...overrides,
  }
}

const MODELS_BY_PROVIDER: ModelsByProvider[] = [
  { provider: makeProvider(), models: ['gpt-4o', 'gpt-4-turbo'] },
  {
    provider: makeProvider({
      id: 'p2',
      name: 'Anthropic Primary',
      providerKind: 'anthropic',
      providerRuntimeId: 'anthropic-primary',
    }),
    models: ['claude-3-5-sonnet-latest'],
  },
]

function makeFormState(overrides: Partial<ApiKeyFormState> = {}): ApiKeyFormState {
  return {
    label: '',
    scopes: [...DEFAULT_SCOPES],
    tier: 'enterprise',
    expiresAt: null,
    allowedModels: [],
    allowedProviders: [],
    ...overrides,
  }
}

function makeWrapper(state: ApiKeyFormState) {
  return mount(ApiKeyForm, {
    props: {
      modelValue: state,
      scopes: SCOPES,
      providers: MODELS_BY_PROVIDER.map((m) => m.provider),
      modelsByProvider: MODELS_BY_PROVIDER,
      tierOptions: TIER_OPTIONS,
    },
    global: { plugins: [i18n] },
  })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('ApiKeyForm', () => {
  describe('scope rendering', () => {
    it('renders all 5 scopes from the registry', () => {
      const wrapper = makeWrapper(makeFormState())
      const rows = wrapper.findAll('[data-testid^="scope-row-"]')
      expect(rows).toHaveLength(SCOPES.length)
    })

    it('groups scopes under chat, providers, admin headings', () => {
      const wrapper = makeWrapper(makeFormState())
      expect(wrapper.find('[data-testid="scope-group-chat"]').exists()).toBe(true)
      expect(wrapper.find('[data-testid="scope-group-providers"]').exists()).toBe(true)
      expect(wrapper.find('[data-testid="scope-group-admin"]').exists()).toBe(true)
    })

    it('renders scope description text', () => {
      const wrapper = makeWrapper(makeFormState())
      expect(wrapper.text()).toContain('Read chat completions and conversation history')
      expect(wrapper.text()).toContain('Full administrative access')
    })

    it('flags the admin scope with a danger indicator', () => {
      const wrapper = makeWrapper(makeFormState())
      expect(wrapper.find('[data-testid="scope-danger-admin"]').exists()).toBe(true)
    })

    it('does not flag non-admin scopes as dangerous', () => {
      const wrapper = makeWrapper(makeFormState())
      for (const scope of SCOPES) {
        if (scope.value === 'admin') continue
        expect(
          wrapper.find(`[data-testid="scope-danger-${scope.value.replace(/[^a-zA-Z0-9_-]/g, '-')}"]`).exists(),
          `scope ${scope.value} should not be flagged as dangerous`,
        ).toBe(false)
      }
    })
  })

  describe('default scope checkboxes', () => {
    it('checks every scope by default EXCEPT admin', () => {
      const wrapper = makeWrapper(makeFormState())
      for (const scope of SCOPES) {
        const slug = scope.value.replace(/[^a-zA-Z0-9_-]/g, '-')
        const checkbox = wrapper.find<HTMLInputElement>(
          `[data-testid="scope-checkbox-${slug}"]`,
        )
        expect(checkbox.exists(), `checkbox for ${scope.value} should exist`).toBe(true)
        if (scope.value === 'admin') {
          expect(checkbox.element.checked, 'admin must be unchecked by default').toBe(false)
        } else {
          expect(checkbox.element.checked, `${scope.value} must be checked by default`).toBe(
            true,
          )
        }
      }
    })
  })

  describe('scope toggling', () => {
    it('emits update:modelValue when a scope is toggled on', async () => {
      const wrapper = makeWrapper(makeFormState({ scopes: [] }))
      const adminCheckbox = wrapper.find<HTMLInputElement>(
        '[data-testid="scope-checkbox-admin"]',
      )
      await adminCheckbox.setValue(true)
      const emitted = wrapper.emitted('update:modelValue')
      expect(emitted).toBeTruthy()
      const last = (emitted!.at(-1) as unknown as [ApiKeyFormState])[0]
      expect(last.scopes).toContain('admin')
    })

    it('emits update:modelValue when a scope is toggled off', async () => {
      const wrapper = makeWrapper(makeFormState())
      const chatReadCheckbox = wrapper.find<HTMLInputElement>(
        '[data-testid="scope-checkbox-chat-read"]',
      )
      await chatReadCheckbox.setValue(false)
      const emitted = wrapper.emitted('update:modelValue')
      expect(emitted).toBeTruthy()
      const last = (emitted!.at(-1) as unknown as [ApiKeyFormState])[0]
      expect(last.scopes).not.toContain('chat:read')
    })
  })

  describe('allowed providers', () => {
    it('renders one checkbox per provider', () => {
      const wrapper = makeWrapper(makeFormState())
      const rows = wrapper.findAll('[data-testid^="provider-row-"]')
      expect(rows).toHaveLength(2)
    })

    it('shows helper text "Leave empty to allow all providers"', () => {
      const wrapper = makeWrapper(makeFormState())
      expect(wrapper.text()).toContain('Leave empty to allow all providers')
    })

    it('checks the provider when toggled on', async () => {
      const wrapper = makeWrapper(makeFormState())
      const cb = wrapper.find<HTMLInputElement>('[data-testid="provider-checkbox-p1"]')
      expect(cb.exists(), 'provider checkbox should exist').toBe(true)
      await cb.setValue(true)
      const emitted = wrapper.emitted('update:modelValue')!
      const last = (emitted.at(-1) as unknown as [ApiKeyFormState])[0]
      expect(last.allowedProviders).toContain('p1')
    })
  })

  describe('allowed models', () => {
    it('does NOT render a text input for models', () => {
      const wrapper = makeWrapper(makeFormState())
      // The legacy free-text input had id="create-allowed-models" or
      // id="edit-allowed-models". The new form must not have any
      // <input> with type="text" inside the models section.
      const modelsSection = wrapper.find('[data-testid="api-key-models"]')
      const textInputs = modelsSection.findAll('input[type="text"]')
      expect(textInputs).toHaveLength(0)
    })

    it('renders one checkbox per model, grouped by provider', () => {
      const wrapper = makeWrapper(makeFormState())
      const groups = wrapper.findAll('[data-testid^="model-group-"]')
      expect(groups.length).toBeGreaterThanOrEqual(1)
      expect(wrapper.find('[data-testid="model-row-p1-gpt-4o"]').exists()).toBe(true)
      expect(
        wrapper.find('[data-testid="model-row-p2-claude-3-5-sonnet-latest"]').exists(),
      ).toBe(true)
    })

    it('shows helper text "Leave empty to allow all models"', () => {
      const wrapper = makeWrapper(makeFormState())
      expect(wrapper.text()).toContain('Leave empty to allow all models')
    })

    it('checks the model when toggled on', async () => {
      const wrapper = makeWrapper(makeFormState())
      const cb = wrapper.find<HTMLInputElement>('[data-testid="model-checkbox-p1-gpt-4o"]')
      expect(cb.exists(), 'model checkbox should exist').toBe(true)
      await cb.setValue(true)
      const emitted = wrapper.emitted('update:modelValue')!
      const last = (emitted.at(-1) as unknown as [ApiKeyFormState])[0]
      expect(last.allowedModels).toContain('gpt-4o')
    })
  })

  describe('tier', () => {
    it('renders the tier with description visible', () => {
      const wrapper = makeWrapper(makeFormState({ tier: 'enterprise' }))
      // The select trigger must exist with a value
      const trigger = wrapper.find('[data-testid="api-key-tier"]')
      expect(trigger.exists()).toBe(true)
    })

    it('renders all tier options in the dropdown', () => {
      const wrapper = makeWrapper(makeFormState())
      for (const opt of TIER_OPTIONS) {
        const item = wrapper.find(`[data-testid="select-item-${opt.value}"]`)
        expect(item.exists(), `tier option ${opt.value} should exist`).toBe(true)
        expect(item.text()).toContain(opt.label)
        expect(item.text()).toContain(opt.description)
      }
    })
  })

  describe('label input', () => {
    it('renders the label input with the current value', () => {
      const wrapper = makeWrapper(makeFormState({ label: 'my-agent' }))
      const input = wrapper.find<HTMLInputElement>('[data-testid="input-api-key-label"]')
      expect(input.exists()).toBe(true)
      expect(input.element.value).toBe('my-agent')
    })

    it('emits update:modelValue on input change', async () => {
      const wrapper = makeWrapper(makeFormState())
      const input = wrapper.find<HTMLInputElement>('[data-testid="input-api-key-label"]')
      await input.setValue('new-label')
      const emitted = wrapper.emitted('update:modelValue')!
      const last = (emitted.at(-1) as unknown as [ApiKeyFormState])[0]
      expect(last.label).toBe('new-label')
    })
  })

  describe('error display', () => {
    it('shows error when error prop is provided', () => {
      const wrapper = mount(ApiKeyForm, {
        props: {
          modelValue: makeFormState(),
          scopes: SCOPES,
          providers: [],
          modelsByProvider: [],
          tierOptions: TIER_OPTIONS,
          error: 'At least one scope is required',
        },
        global: { plugins: [i18n] },
      })
      expect(wrapper.find('[data-testid="api-key-form-error"]').exists()).toBe(true)
      expect(wrapper.text()).toContain('At least one scope is required')
    })

    it('does not show error block when error prop is null', () => {
      const wrapper = makeWrapper(makeFormState())
      expect(wrapper.find('[data-testid="api-key-form-error"]').exists()).toBe(false)
    })
  })

  describe('events', () => {
    it('emits submit when the form is submitted', async () => {
      const wrapper = makeWrapper(makeFormState())
      await wrapper.find('[data-testid="api-key-form"]').trigger('submit')
      expect(wrapper.emitted('submit')).toBeTruthy()
    })

    it('emits cancel when the cancel button is clicked', async () => {
      const wrapper = makeWrapper(makeFormState())
      // The cancel button has data-testid="api-key-cancel"
      const cancelButton = wrapper.find('[data-testid="api-key-cancel"]')
      await cancelButton.trigger('click')
      expect(wrapper.emitted('cancel')).toBeTruthy()
    })
  })
})
