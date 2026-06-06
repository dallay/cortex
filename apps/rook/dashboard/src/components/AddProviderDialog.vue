<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { AlertCircle, CheckCircle2, Loader2, Trash2 } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import PasswordInput from '@/components/PasswordInput.vue'
import { useProviders } from '@/composables/useProviders'
import {
  PROVIDER_KINDS,
  findCatalogEntry,
  type AuthType,
  type ProviderKind,
} from '@/config/providerCatalog'
import type {
  CreateProviderRequest,
  ProviderConnectionResponse,
  UpdateProviderRequest,
  WireAuthType,
} from '@/lib/api'

type DialogMode = 'create' | 'edit'

const props = withDefaults(
  defineProps<{
    /** v-model:open — controlled by parent */
    open?: boolean
    /** When provided, dialog is scoped to this provider (no kind selector) */
    providerKind?: ProviderKind
    /** 'create' = new connection, 'edit' = load existing connection */
    mode?: DialogMode
    /** Required when mode='edit' */
    connectionId?: string
  }>(),
  { open: false, mode: 'create' },
)

const emit = defineEmits<{
  'update:open': [value: boolean]
  saved: [connection: ProviderConnectionResponse]
  deleted: [id: string]
}>()

const { t } = useI18n()
const {
  create,
  update,
  remove,
  testCredentials,
  fetch,
  providerById,
} = useProviders()

// ---------------------------------------------------------------------------
// Form state
// ---------------------------------------------------------------------------

/** Currently selected kind. Defaults to prop or first catalog entry. */
const kind = ref<ProviderKind>(props.providerKind ?? PROVIDER_KINDS[0].kind)
/** Currently selected auth type. OAuth is gated — see supportsOAuth below. */
const authType = ref<AuthType>('apikey')

interface FormState {
  displayName: string
  apiKey: string
  baseUrl: string
  priority: number
  models: string
  enabled: boolean
}

function blankForm(): FormState {
  return {
    displayName: '',
    apiKey: '',
    baseUrl: findCatalogEntry(kind.value).defaultBaseUrl,
    priority: 50,
    models: '',
    enabled: true,
  }
}

const form = ref<FormState>(blankForm())
const testing = ref(false)
const saving = ref(false)
const deleting = ref(false)
const testResult = ref<{ ok: boolean; message: string } | null>(null)
const showDeleteConfirm = ref(false)

// ---------------------------------------------------------------------------
// Derived state
// ---------------------------------------------------------------------------

const isEditMode = computed(() => props.mode === 'edit')
const showKindSelector = computed(() => props.providerKind === undefined)
const catalogEntry = computed(() => findCatalogEntry(kind.value))
const isApikey = computed(() => authType.value === 'apikey')
const supportsOAuth = computed(() => catalogEntry.value.authTypes.includes('oauth'))

const canTest = computed(
  () =>
    !testing.value &&
    !saving.value &&
    form.value.displayName.trim() !== '' &&
    (isApikey.value ? form.value.apiKey.trim() !== '' : false),
)

const canSave = computed(
  () => canTest.value && testResult.value?.ok === true && !saving.value,
)

// ---------------------------------------------------------------------------
// Wire format helpers
// ---------------------------------------------------------------------------

/** Internal `apikey` → wire `apiKey` (backend enum uses camelCase). */
function wireAuthType(a: AuthType): WireAuthType {
  return a === 'apikey' ? 'apiKey' : 'oauth'
}

function buildTestPayload() {
  const runtimeId = `${kind.value}-test-${Date.now()}-${crypto.randomUUID().slice(0, 8)}`
  return {
    providerKind: kind.value,
    providerRuntimeId: runtimeId,
    authType: wireAuthType(authType.value),
    credentials: isApikey.value ? { apiKey: form.value.apiKey } : {},
    config: {
      maxConcurrent: 10,
      quotaWindowThresholds: { warning: 0.8, error: 0.95 },
      defaultModel: firstModel() ?? undefined,
      baseUrl: form.value.baseUrl.trim() || undefined,
    },
  }
}

function firstModel(): string | null {
  const first = form.value.models.split(',')[0]?.trim() ?? ''
  return first === '' ? null : first
}

function buildCreateRequest(): CreateProviderRequest {
  const runtimeId = `${kind.value}-${Date.now()}-${crypto.randomUUID().slice(0, 8)}`
  return {
    providerKind: kind.value,
    providerRuntimeId: runtimeId,
    authType: wireAuthType(authType.value),
    name: form.value.displayName.trim(),
    priority: form.value.priority,
    isActive: form.value.enabled,
    credentials: isApikey.value
      ? { apiKey: form.value.apiKey }
      : { apiKey: '' },
    config: {
      maxConcurrent: 10,
      quotaWindowThresholds: { warning: 0.8, error: 0.95 },
      defaultModel: firstModel() ?? undefined,
      baseUrl: form.value.baseUrl.trim() || undefined,
    },
  }
}

function buildUpdateRequest(
  existing: ProviderConnectionResponse,
): UpdateProviderRequest {
  // API key is only sent when the user typed a new one (stored secrets
  // are never re-exposed on edit, see spec REQ-3 Scenario "Edit existing").
  const credentials =
    isApikey.value && form.value.apiKey.trim() !== ''
      ? { apiKey: form.value.apiKey }
      : undefined
  return {
    expectedUpdatedAt: existing.updatedAt,
    providerKind: kind.value,
    authType: wireAuthType(authType.value),
    name: form.value.displayName.trim(),
    priority: form.value.priority,
    isActive: form.value.enabled,
    credentials,
    config: {
      maxConcurrent: 10,
      quotaWindowThresholds: { warning: 0.8, error: 0.95 },
      defaultModel: firstModel() ?? undefined,
      baseUrl: form.value.baseUrl.trim() || undefined,
    },
  }
}

// ---------------------------------------------------------------------------
// Lifecycle: load on open, reset on close
// ---------------------------------------------------------------------------

function loadFromConnection(conn: ProviderConnectionResponse) {
  kind.value = conn.providerKind
  authType.value = conn.authType === 'oauth' ? 'oauth' : 'apikey'
  form.value.displayName = conn.name
  form.value.baseUrl = conn.config.baseUrl ?? findCatalogEntry(kind.value).defaultBaseUrl
  form.value.priority = conn.priority
  form.value.models = conn.config.defaultModel ?? ''
  form.value.enabled = conn.isActive
  form.value.apiKey = '' // never re-expose stored secret
  testResult.value = null
}

function resetForm() {
  kind.value = props.providerKind ?? PROVIDER_KINDS[0].kind
  authType.value = 'apikey'
  form.value = blankForm()
  testResult.value = null
  showDeleteConfirm.value = false
}

watch(
  () => props.open,
  async isOpen => {
    if (!isOpen) {
      resetForm()
      return
    }
    if (isEditMode.value && props.connectionId) {
      // Use cache if we already have the connection, otherwise fetch the list.
      let conn = providerById.value.get(props.connectionId)
      if (!conn) {
        await fetch()
        conn = providerById.value.get(props.connectionId)
      }
      if (conn) loadFromConnection(conn)
    } else {
      // Create mode — seed from current kind.
      if (props.providerKind) kind.value = props.providerKind
      form.value = blankForm()
      testResult.value = null
    }
  },
  { immediate: true },
)

// When the user changes the kind manually (create mode), keep baseUrl in sync
// with the catalog default so the user always starts from a known-good URL.
watch(kind, newKind => {
  if (!isEditMode.value) {
    form.value.baseUrl = findCatalogEntry(newKind).defaultBaseUrl
  }
})

// ---------------------------------------------------------------------------
// Event handlers
// ---------------------------------------------------------------------------

function handleOpenChange(value: boolean) {
  emit('update:open', value)
}

async function handleTest() {
  if (!canTest.value) return
  testing.value = true
  testResult.value = null
  try {
    const result = await testCredentials(buildTestPayload())
    if (result?.ok) {
      testResult.value = {
        ok: true,
        message: `Connected successfully (${result.latencyMs ?? 0}ms)`,
      }
    } else {
      testResult.value = {
        ok: false,
        message: result?.error ?? 'Connection failed',
      }
    }
  } catch (e) {
    testResult.value = {
      ok: false,
      message: e instanceof Error ? e.message : 'Test failed',
    }
  } finally {
    testing.value = false
  }
}

async function handleSave() {
  if (!canSave.value) return
  saving.value = true
  try {
    if (isEditMode.value && props.connectionId) {
      const existing = providerById.value.get(props.connectionId)
      if (!existing) {
        testResult.value = { ok: false, message: 'Connection not found' }
        return
      }
      const updated = await update(
        props.connectionId,
        buildUpdateRequest(existing),
      )
      if (updated) {
        emit('saved', updated)
        emit('update:open', false)
      }
    } else {
      const created = await create(buildCreateRequest())
      if (created) {
        await fetch()
        emit('saved', created)
        emit('update:open', false)
      }
    }
  } finally {
    saving.value = false
  }
}

async function handleDelete() {
  if (!isEditMode.value || !props.connectionId) return
  deleting.value = true
  try {
    const ok = await remove(props.connectionId)
    if (ok) {
      emit('deleted', props.connectionId)
      showDeleteConfirm.value = false
      emit('update:open', false)
    }
  } finally {
    deleting.value = false
  }
}
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <DialogContent class="sm:max-w-[560px]">
      <DialogHeader>
        <DialogTitle>
          {{
            isEditMode
              ? t('providers.form.editTitle', {
                  providerName: t(catalogEntry.displayNameKey),
                })
              : t('providers.form.createTitle', {
                  providerName: t(catalogEntry.displayNameKey),
                })
          }}
        </DialogTitle>
        <DialogDescription>
          {{ t('providers.form.authType') }}
        </DialogDescription>
      </DialogHeader>

      <div class="space-y-4 py-2">
        <!-- Kind selector (hidden when providerKind is pre-scoped) -->
        <div v-if="showKindSelector" class="space-y-2">
          <Label for="kind">{{ t('providers.form.selectProvider') }}</Label>
          <Select
            :model-value="kind"
            @update:model-value="(v) => v && (kind = v as ProviderKind)"
          >
            <SelectTrigger id="kind" data-testid="kind-select-trigger">
              <SelectValue :placeholder="t('providers.form.selectProvider')" />
            </SelectTrigger>
            <SelectContent>
              <SelectItem
                v-for="entry in PROVIDER_KINDS"
                :key="entry.kind"
                :value="entry.kind"
              >
                {{ t(entry.displayNameKey) }}
              </SelectItem>
            </SelectContent>
          </Select>
        </div>

        <!-- Auth type toggle (OAuth disabled when kind doesn't support it) -->
        <div class="space-y-2">
          <Label>{{ t('providers.form.authType') }}</Label>
          <ToggleGroup
            v-model="authType"
            type="single"
            class="justify-start"
            data-testid="auth-type-toggle"
          >
            <ToggleGroupItem
              value="apikey"
              :aria-label="t('providers.form.authTypeApikey')"
              data-testid="auth-type-apikey"
            >
              {{ t('providers.form.authTypeApikey') }}
            </ToggleGroupItem>
            <ToggleGroupItem
              value="oauth"
              :aria-label="t('providers.form.authTypeOauth')"
              :disabled="!supportsOAuth"
              data-testid="auth-type-oauth"
            >
              {{ t('providers.form.authTypeOauth') }}
            </ToggleGroupItem>
          </ToggleGroup>
          <p
            v-if="!supportsOAuth"
            class="text-xs text-muted-foreground"
            data-testid="oauth-coming-soon"
          >
            {{ t('providers.form.oauthComingSoon') }}
          </p>
        </div>

        <!-- Display name -->
        <div class="space-y-2">
          <Label for="displayName">{{ t('providers.form.displayName') }}</Label>
          <Input
            id="displayName"
            v-model="form.displayName"
            :placeholder="
              t('providers.form.displayNamePlaceholder', {
                providerName: t(catalogEntry.displayNameKey),
              })
            "
            :disabled="saving"
            data-testid="input-displayName"
          />
        </div>

        <!-- API key (apikey auth only) -->
        <div v-if="isApikey" class="space-y-2">
          <Label for="apiKey">{{ t('providers.form.apiKey') }}</Label>
          <PasswordInput
            id="apiKey"
            v-model="form.apiKey"
            :placeholder="t('providers.form.apiKeyPlaceholder')"
            :disabled="saving"
          />
        </div>

        <!-- Base URL -->
        <div class="space-y-2">
          <Label for="baseUrl">{{ t('providers.form.baseUrl') }}</Label>
          <Input
            id="baseUrl"
            v-model="form.baseUrl"
            :disabled="saving"
            data-testid="input-baseUrl"
          />
          <p class="text-xs text-muted-foreground">
            {{ t('providers.form.baseUrlHelp') }}
          </p>
        </div>

        <!-- Priority -->
        <div class="space-y-2">
          <Label for="priority">{{ t('providers.form.priority') }}</Label>
          <Input
            id="priority"
            v-model.number="form.priority"
            type="number"
            min="0"
            max="100"
            :disabled="saving"
            data-testid="input-priority"
          />
          <p class="text-xs text-muted-foreground">
            {{ t('providers.form.priorityHelp') }}
          </p>
        </div>

        <!-- Models (comma-separated) -->
        <div class="space-y-2">
          <Label for="models">{{ t('providers.form.models') }}</Label>
          <Input
            id="models"
            v-model="form.models"
            :placeholder="t('providers.form.modelsPlaceholder')"
            :disabled="saving"
            data-testid="input-models"
          />
          <p class="text-xs text-muted-foreground">
            {{ t('providers.form.modelsHelp') }}
          </p>
        </div>

        <!-- Enabled -->
        <div class="flex items-center justify-between rounded-md border px-3 py-2">
          <Label for="enabled">{{ t('providers.form.enabled') }}</Label>
          <Switch
            id="enabled"
            :checked="form.enabled"
            @update:checked="(v: boolean) => (form.enabled = v)"
            :disabled="saving"
            data-testid="switch-enabled"
          />
        </div>

        <!-- Test result -->
        <div
          v-if="testResult"
          class="flex items-start gap-2 rounded-md border p-3"
          data-testid="test-result"
        >
          <CheckCircle2
            v-if="testResult.ok"
            class="h-5 w-5 text-green-500 mt-0.5"
          />
          <AlertCircle v-else class="h-5 w-5 text-destructive mt-0.5" />
          <div class="flex-1">
            <p
              class="text-sm font-medium"
              :class="testResult.ok ? 'text-green-600' : 'text-destructive'"
            >
              {{
                testResult.ok
                  ? t('providers.form.testSuccess')
                  : t('providers.form.testFailed')
              }}
            </p>
            <p class="text-xs text-muted-foreground mt-0.5">
              {{ testResult.message }}
            </p>
          </div>
        </div>
      </div>

      <DialogFooter class="flex-col sm:flex-row gap-2">
        <div class="flex gap-2">
          <Button
            variant="outline"
            :disabled="!canTest"
            data-testid="test-button"
            @click="handleTest"
          >
            <Loader2 v-if="testing" class="h-4 w-4 mr-2 animate-spin" />
            {{
              testing
                ? t('providers.form.testing')
                : t('providers.form.testCredentials')
            }}
          </Button>
          <Button
            :disabled="!canSave"
            data-testid="save-button"
            @click="handleSave"
          >
            <Loader2 v-if="saving" class="h-4 w-4 mr-2 animate-spin" />
            {{ t('providers.form.save') }}
          </Button>
        </div>
        <Button
          v-if="isEditMode"
          variant="destructive"
          class="sm:ml-auto"
          data-testid="delete-button"
          @click="showDeleteConfirm = true"
        >
          <Trash2 class="h-4 w-4 mr-2" />
          {{ t('providers.form.delete') }}
        </Button>
      </DialogFooter>
    </DialogContent>

    <!-- Delete confirmation -->
    <AlertDialog :open="showDeleteConfirm" @update:open="(v: boolean) => (showDeleteConfirm = v)">
      <AlertDialogContent data-testid="delete-confirm">
        <AlertDialogHeader>
          <AlertDialogTitle>
            {{ t('providers.details.deleteConfirm') }}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {{ form.displayName || t(catalogEntry.displayNameKey) }}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel data-testid="delete-cancel">
            {{ t('providers.form.cancel') }}
          </AlertDialogCancel>
          <AlertDialogAction
            data-testid="delete-confirm-button"
            @click="handleDelete"
          >
            <Loader2 v-if="deleting" class="h-4 w-4 mr-2 animate-spin" />
            {{ t('providers.form.delete') }}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </Dialog>
</template>
