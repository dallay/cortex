<script setup lang="ts">
/**
 * ApiKeyForm — shared form for creating and editing API keys.
 *
 * Replaces the inline forms that used to live in ApiKeysView.vue. The
 * component is fully controlled: it never owns the form state, only
 * renders it and emits `update:modelValue` when the user interacts.
 * This makes pre-population (edit modal) and reset (create modal)
 * trivial at the parent level.
 *
 * Scope and tier metadata is driven by the `scopes` and `tierOptions`
 * props so this component doesn't hardcode domain knowledge — when
 * the scope registry grows, no template changes are needed.
 */
import { computed } from 'vue'
import { Key, AlertTriangle, ShieldAlert } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Checkbox } from '@/components/ui/checkbox'
import { Badge } from '@/components/ui/badge'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import type { ProviderConnectionResponse } from '@/lib/api'
import type { ModelsByProvider } from '@/composables/useAvailableModels'
import type { ScopeDef, ScopeGroup } from '@/config/scopes'

export interface ApiKeyFormState {
  label: string
  scopes: string[]
  tier: string
  expiresAt: string | null
  allowedModels: string[]
  allowedProviders: string[]
  isActive?: boolean
}

interface Props {
  modelValue: ApiKeyFormState
  scopes: readonly ScopeDef[]
  providers: ProviderConnectionResponse[]
  modelsByProvider: ModelsByProvider[]
  tierOptions: { value: string; label: string; description: string }[]
  /** Optional group render order. Defaults to chat, providers, admin. */
  groupOrder?: readonly ScopeGroup[]
  error?: string | null
  submitLabel?: string
  cancelLabel?: string
  isEdit?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  groupOrder: () => ['chat', 'providers', 'admin'] as const,
  error: null,
  submitLabel: 'Save',
  cancelLabel: 'Cancel',
  isEdit: false,
})

const emit = defineEmits<{
  'update:modelValue': [value: ApiKeyFormState]
  submit: []
  cancel: []
}>()

function update<K extends keyof ApiKeyFormState>(key: K, value: ApiKeyFormState[K]) {
  emit('update:modelValue', { ...props.modelValue, [key]: value })
}

function toggleScope(scopeValue: string, checked: boolean) {
  const next = checked
    ? Array.from(new Set([...props.modelValue.scopes, scopeValue]))
    : props.modelValue.scopes.filter((s) => s !== scopeValue)
  update('scopes', next)
}

function toggleProvider(providerId: string, checked: boolean) {
  const next = checked
    ? Array.from(new Set([...props.modelValue.allowedProviders, providerId]))
    : props.modelValue.allowedProviders.filter((p) => p !== providerId)
  update('allowedProviders', next)
}

function toggleModel(modelId: string, checked: boolean) {
  const next = checked
    ? Array.from(new Set([...props.modelValue.allowedModels, modelId]))
    : props.modelValue.allowedModels.filter((m) => m !== modelId)
  update('allowedModels', next)
}

const groupedScopes = computed(() => {
  return props.groupOrder
    .map((group) => ({
      group,
      scopes: props.scopes.filter((s) => s.group === group),
    }))
    .filter((entry) => entry.scopes.length > 0)
})

const groupLabel: Record<ScopeGroup, string> = {
  chat: 'Chat',
  providers: 'Providers',
  admin: 'Administrative',
}

function isScopeChecked(scope: ScopeDef): boolean {
  return props.modelValue.scopes.includes(scope.value)
}

function isProviderChecked(providerId: string): boolean {
  return props.modelValue.allowedProviders.includes(providerId)
}

function isModelChecked(modelId: string): boolean {
  return props.modelValue.allowedModels.includes(modelId)
}

/** Slugify a scope value so it is safe for use in a data-testid attribute.
 *  e.g. 'chat:read' -> 'chat-read', 'admin' -> 'admin'. */
function scopeSlug(value: string): string {
  return value.replace(/[^a-zA-Z0-9_-]/g, '-')
}
</script>

<template>
  <form
    data-testid="api-key-form"
    class="space-y-6"
    @submit.prevent="emit('submit')"
  >
    <!-- Label -->
    <div class="space-y-2">
      <label for="api-key-label" class="text-sm font-medium">Label</label>
      <Input
        id="api-key-label"
        data-testid="input-api-key-label"
        :model-value="modelValue.label"
        placeholder="e.g., opencode-agent, hermes-agent"
        @update:model-value="(v) => update('label', String(v ?? ''))"
      />
    </div>

    <!-- Scopes — grouped, with descriptions and danger indicators -->
    <div class="space-y-2" data-testid="api-key-scopes">
      <p class="text-sm font-medium">Scopes</p>
      <p class="text-xs text-muted-foreground">
        Select what this key is allowed to do. Defaults to least-privilege — enable Admin explicitly if needed.
      </p>
      <div class="space-y-4 pt-2">
        <div
          v-for="entry in groupedScopes"
          :key="entry.group"
          class="space-y-2"
          :data-testid="`scope-group-${entry.group}`"
        >
          <p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {{ groupLabel[entry.group] }}
          </p>
          <div class="space-y-2">
            <label
              v-for="scope in entry.scopes"
              :key="scope.value"
              :data-testid="`scope-row-${scopeSlug(scope.value)}`"
              :data-scope-value="scope.value"
              class="flex items-start gap-3 rounded-md border p-3 hover:bg-muted/30 cursor-pointer"
              :class="scope.danger ? 'border-destructive/30 bg-destructive/5' : ''"
            >
              <Checkbox
                :model-value="isScopeChecked(scope)"
                :data-testid="`scope-checkbox-${scopeSlug(scope.value)}`"
                @update:model-value="(v) => toggleScope(scope.value, v === true)"
              />
              <div class="flex-1 space-y-0.5">
                <div class="flex items-center gap-2">
                  <code class="text-sm font-mono font-semibold">{{ scope.label }}</code>
                  <Badge
                    v-if="scope.danger"
                    variant="destructive"
                    :data-testid="`scope-danger-${scopeSlug(scope.value)}`"
                    class="gap-1"
                  >
                    <ShieldAlert class="h-3 w-3" />
                    Danger
                  </Badge>
                </div>
                <p class="text-xs text-muted-foreground">{{ scope.description }}</p>
              </div>
            </label>
          </div>
        </div>
      </div>
    </div>

    <!-- Tier -->
    <div class="space-y-2">
      <label for="api-key-tier" class="text-sm font-medium">Rate limit tier</label>
      <Select
        :model-value="modelValue.tier"
        @update:model-value="(v) => update('tier', String(v ?? modelValue.tier))"
      >
        <SelectTrigger id="api-key-tier" data-testid="api-key-tier">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem
            v-for="opt in tierOptions"
            :key="opt.value"
            :value="opt.value"
          >
            {{ opt.label }} — {{ opt.description }}
          </SelectItem>
        </SelectContent>
      </Select>
      <p class="text-xs text-muted-foreground">
        Controls the request-rate bucket for this key. Default: highest tier.
      </p>
    </div>

    <!-- Allowed Providers -->
    <div class="space-y-2" data-testid="api-key-providers">
      <p class="text-sm font-medium">Allowed providers</p>
      <p class="text-xs text-muted-foreground">
        Leave empty to allow all providers.
      </p>
      <div v-if="providers.length === 0" class="text-xs text-muted-foreground italic">
        No providers configured.
      </div>
      <div v-else class="space-y-2 pt-1">
        <label
          v-for="provider in providers"
          :key="provider.id"
          class="flex items-center gap-2"
          :data-testid="`provider-row-${provider.id}`"
        >
          <Checkbox
            :model-value="isProviderChecked(provider.id)"
            :data-testid="`provider-checkbox-${provider.id}`"
            @update:model-value="(v) => toggleProvider(provider.id, v === true)"
          />
          <span class="text-sm">{{ provider.name }}</span>
          <code class="text-xs text-muted-foreground">({{ provider.providerKind }})</code>
        </label>
      </div>
    </div>

    <!-- Allowed Models -->
    <div class="space-y-2" data-testid="api-key-models">
      <p class="text-sm font-medium">Allowed models</p>
      <p class="text-xs text-muted-foreground">
        Leave empty to allow all models.
      </p>
      <div v-if="modelsByProvider.length === 0" class="text-xs text-muted-foreground italic">
        No models available yet — fetch the model catalog from the API key form to populate.
      </div>
      <div v-else class="space-y-4 pt-1">
        <div
          v-for="entry in modelsByProvider"
          :key="entry.provider.id"
          class="space-y-1"
          :data-testid="`model-group-${entry.provider.id}`"
        >
          <p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {{ entry.provider.name }}
            <code class="ml-1 normal-case">({{ entry.provider.providerKind }})</code>
          </p>
          <div class="grid grid-cols-2 gap-1 pl-2">
            <label
              v-for="model in entry.models"
              :key="model"
              class="flex items-center gap-2"
              :data-testid="`model-row-${entry.provider.id}-${model}`"
            >
              <Checkbox
                :model-value="isModelChecked(model)"
                :data-testid="`model-checkbox-${entry.provider.id}-${model}`"
                @update:model-value="(v) => toggleModel(model, v === true)"
              />
              <code class="text-xs font-mono">{{ model }}</code>
            </label>
          </div>
        </div>
      </div>
    </div>

    <!-- Expires at -->
    <div class="space-y-2">
      <label for="api-key-expires" class="text-sm font-medium">Expires at (optional)</label>
      <Input
        id="api-key-expires"
        data-testid="input-api-key-expires"
        type="datetime-local"
        :model-value="modelValue.expiresAt ?? ''"
        @update:model-value="(v) => update('expiresAt', v ? String(v) : null)"
      />
    </div>

    <!-- Error display -->
    <div
      v-if="error"
      data-testid="api-key-form-error"
      class="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
    >
      <div class="flex items-start gap-2">
        <AlertTriangle class="h-4 w-4 mt-0.5" />
        <span>{{ error }}</span>
      </div>
    </div>

    <!-- Footer -->
    <div class="flex items-center justify-end gap-2 pt-2">
      <Button type="button" variant="outline" :data-testid="'api-key-cancel'" @click="emit('cancel')">
        {{ cancelLabel }}
      </Button>
      <Button type="submit" :data-testid="'api-key-submit'">
        <Key class="h-4 w-4 mr-1" />
        {{ submitLabel }}
      </Button>
    </div>
  </form>
</template>
