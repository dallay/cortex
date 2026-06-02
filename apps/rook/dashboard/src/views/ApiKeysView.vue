<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { Plus, Copy, Key, AlertTriangle, RefreshCw, Pencil, Trash2 } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { useApiKeys } from '@/composables/useApiKeys'
import type { CreateApiKeyRequest, UpdateApiKeyRequest } from '@/lib/api'

const { t } = useI18n()

const {
  apiKeys,
  loading,
  error,
  total,
  limit,
  offset,
  fetch,
  create,
  update,
  revoke,
  nextPage,
  prevPage,
} = useApiKeys()

// Create modal state
const showCreateModal = ref(false)
const createForm = ref<CreateApiKeyRequest>({
  label: '',
  scopes: [],
  tier: 'free',
  expiresAt: null,
})
const createError = ref<string | null>(null)
const newlyCreatedKey = ref<string | null>(null)

// Edit modal state
const showEditModal = ref(false)
const editingKey = ref<string | null>(null)
const editForm = ref<UpdateApiKeyRequest>({})
const editError = ref<string | null>(null)

// Revoke confirmation
const showRevokeConfirm = ref(false)
const revokingKeyId = ref<string | null>(null)
const revokeError = ref<string | null>(null)

// Visibility toggle for newly created key
const showNewKey = ref(false)

onMounted(() => {
  fetch()
})

function formatDate(dateStr: string | null): string {
  if (!dateStr) return '—'
  return new Date(dateStr).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  })
}

function maskKey(keyPrefix: string): string {
  return keyPrefix + '...'
}

async function handleCreate() {
  createError.value = null
  const errors: string[] = []
  if (!createForm.value.label.trim()) {
    errors.push('Label is required')
  }
  if (createForm.value.scopes.length === 0) {
    errors.push('At least one scope is required')
  }
  if (errors.length > 0) {
    createError.value = errors.join('. ')
    return
  }

  const result = await create(createForm.value)
  if (result) {
    newlyCreatedKey.value = result.plaintextKey
    showNewKey.value = true
    // Reset form
    createForm.value = { label: '', scopes: [], tier: 'free', expiresAt: null }
  } else {
    createError.value = error.value || 'Failed to create API key'
  }
}

function closeCreateWithKey() {
  showCreateModal.value = false
  newlyCreatedKey.value = null
  showNewKey.value = false
}

function openEditModal(key: typeof apiKeys.value[0]) {
  editingKey.value = key.id
  editForm.value = {
    label: key.label,
    scopes: [...key.scopes],
    tier: key.tier,
    isActive: key.isActive,
    expiresAt: key.expiresAt,
  }
  showEditModal.value = true
}

async function handleUpdate() {
  if (!editingKey.value) return
  editError.value = null

  const result = await update(editingKey.value, editForm.value)
  if (result) {
    showEditModal.value = false
    editingKey.value = null
    editForm.value = {}
  } else {
    editError.value = error.value || 'Failed to update API key'
  }
}

function confirmRevoke(keyId: string) {
  revokingKeyId.value = keyId
  showRevokeConfirm.value = true
  revokeError.value = null
}

async function handleRevoke() {
  if (!revokingKeyId.value) return

  const success = await revoke(revokingKeyId.value)
  if (success) {
    showRevokeConfirm.value = false
    revokingKeyId.value = null
  } else {
    revokeError.value = error.value || 'Failed to revoke API key'
  }
}

async function copyToClipboard(text: string) {
  try {
    await navigator.clipboard.writeText(text)
  } catch (err) {
    console.error('Failed to copy:', err)
  }
}

const hasNextPage = computed(() => offset.value + limit.value < total.value)
const hasPrevPage = computed(() => offset.value > 0)

const scopesOptions = [
  { value: 'read', label: 'Read' },
  { value: 'write', label: 'Write' },
]

const tierOptions = [
  { value: 'free', label: 'Free' },
  { value: 'pro', label: 'Pro' },
  { value: 'enterprise', label: 'Enterprise' },
]
</script>

<template>
  <div class="space-y-6">
    <!-- Page Header -->
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-2xl font-semibold tracking-tight">{{ t('nav.apiKeys') }}</h1>
        <p class="text-muted-foreground">{{ t('apiKeys.description') || 'Manage API keys for external agents' }}</p>
      </div>
      <Button @click="showCreateModal = true">
        <Plus class="h-4 w-4 mr-2" />
        {{ t('apiKeys.create') || 'Create API Key' }}
      </Button>
    </div>

    <!-- Error State -->
    <div v-if="error" class="rounded-lg border border-destructive/50 bg-destructive/10 p-4">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2 text-destructive">
          <AlertTriangle class="h-4 w-4" />
          <span class="text-sm font-medium">{{ error }}</span>
        </div>
        <Button variant="ghost" size="sm" @click="fetch()">
          <RefreshCw class="h-4 w-4 mr-1" />
          Retry
        </Button>
      </div>
    </div>

    <!-- Loading State -->
    <div v-if="loading && apiKeys.length === 0" class="flex items-center justify-center py-12">
      <RefreshCw class="h-6 w-6 animate-spin text-muted-foreground" />
    </div>

    <!-- Keys List -->
    <div v-else class="rounded-lg border overflow-hidden">
      <table class="w-full">
        <thead>
          <tr class="border-b bg-muted/50">
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.name') || 'Name' }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.keyPrefix') || 'Key' }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">Scopes</th>
            <th class="px-4 py-3 text-left text-sm font-medium">Tier</th>
            <th class="px-4 py-3 text-left text-sm font-medium">Status</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.created') || 'Created' }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.lastUsed') || 'Last Used' }}</th>
            <th class="px-4 py-3 text-right text-sm font-medium">{{ t('common.actions') || 'Actions' }}</th>
          </tr>
        </thead>
        <tbody class="divide-y">
          <tr v-for="item in apiKeys" :key="item.id" class="hover:bg-muted/30">
            <td class="px-4 py-3">
              <div class="flex items-center gap-2">
                <Key class="h-4 w-4 text-muted-foreground" />
                <span class="font-medium">{{ item.label }}</span>
              </div>
            </td>
            <td class="px-4 py-3">
              <code class="text-sm font-mono text-muted-foreground">
                {{ maskKey(item.keyPrefix) }}
              </code>
            </td>
            <td class="px-4 py-3">
              <div class="flex gap-1">
                <span
                  v-for="scope in item.scopes"
                  :key="scope"
                  class="inline-flex items-center rounded-full bg-muted px-2 py-0.5 text-xs font-medium"
                >
                  {{ scope }}
                </span>
              </div>
            </td>
            <td class="px-4 py-3">
              <span class="text-sm capitalize text-muted-foreground">{{ item.tier }}</span>
            </td>
            <td class="px-4 py-3">
              <span
                v-if="item.isActive"
                class="inline-flex items-center rounded-full bg-green-500/10 px-2 py-0.5 text-xs font-medium text-green-600"
              >
                Active
              </span>
              <span
                v-else
                class="inline-flex items-center rounded-full bg-destructive/10 px-2 py-0.5 text-xs font-medium text-destructive"
              >
                Revoked
              </span>
            </td>
            <td class="px-4 py-3 text-sm text-muted-foreground">{{ formatDate(item.createdAt) }}</td>
            <td class="px-4 py-3 text-sm text-muted-foreground">{{ formatDate(item.lastUsedAt) }}</td>
            <td class="px-4 py-3 text-right">
              <div class="flex items-center justify-end gap-2">
                <Button
                  v-if="item.isActive"
                  variant="ghost"
                  size="sm"
                  @click="openEditModal(item)"
                >
                  <Pencil class="h-4 w-4" />
                </Button>
                <Button
                  v-if="item.isActive"
                  variant="ghost"
                  size="sm"
                  class="text-destructive hover:text-destructive"
                  @click="confirmRevoke(item.id)"
                >
                  <Trash2 class="h-4 w-4" />
                </Button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>

      <!-- Empty State -->
      <div v-if="apiKeys.length === 0 && !loading" class="p-8 text-center text-muted-foreground">
        <Key class="h-12 w-12 mx-auto text-muted-foreground mb-4 opacity-50" />
        <p>{{ t('apiKeys.empty') || 'No API keys yet' }}</p>
        <p class="text-sm mt-1">Create your first API key to enable external agent access</p>
      </div>
    </div>

    <!-- Pagination -->
    <div v-if="apiKeys.length > 0" class="flex items-center justify-between">
      <p class="text-sm text-muted-foreground">
        Showing {{ offset + 1 }} - {{ Math.min(offset + limit, total) }} of {{ total }}
      </p>
      <div class="flex gap-2">
        <Button variant="outline" size="sm" :disabled="!hasPrevPage" @click="prevPage">
          Previous
        </Button>
        <Button variant="outline" size="sm" :disabled="!hasNextPage" @click="nextPage">
          Next
        </Button>
      </div>
    </div>

    <!-- Create Modal -->
    <Dialog v-model:open="showCreateModal">
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Create API Key</DialogTitle>
          <DialogDescription>
            Create a new API key for external agent authentication.
            The key will only be shown once — save it securely.
          </DialogDescription>
        </DialogHeader>

        <!-- Newly created key display -->
        <div v-if="newlyCreatedKey" class="space-y-4">
          <div class="rounded-lg bg-amber-500/10 border border-amber-500/20 p-4">
            <div class="flex items-center gap-2 text-amber-600 mb-2">
              <AlertTriangle class="h-4 w-4" />
              <span class="text-sm font-medium">Save this key now — it will not be shown again</span>
            </div>
            <div class="flex items-center gap-2">
              <code class="flex-1 text-sm font-mono bg-muted rounded px-3 py-2 break-all">
                {{ newlyCreatedKey }}
              </code>
              <Button size="sm" variant="outline" aria-label="Copy" @click="copyToClipboard(newlyCreatedKey)">
                <Copy class="h-4 w-4" />
              </Button>
            </div>
          </div>
          <Button class="w-full" @click="closeCreateWithKey">Done</Button>
        </div>

        <!-- Create form -->
        <form v-else @submit.prevent="handleCreate" class="space-y-4">
          <div class="space-y-2">
            <label for="create-key-label" class="text-sm font-medium">Label</label>
            <Input
              id="create-key-label"
              v-model="createForm.label"
              placeholder="e.g., opencode-agent, hermes-agent"
            />
          </div>

          <div class="space-y-2">
            <label class="text-sm font-medium">Scopes</label>
            <div class="flex gap-4">
              <label
                v-for="scope in scopesOptions"
                :key="scope.value"
                class="flex items-center gap-2"
              >
                <input
                  type="checkbox"
                  :value="scope.value"
                  v-model="createForm.scopes"
                  class="rounded border-input"
                />
                <span class="text-sm">{{ scope.label }}</span>
              </label>
            </div>
          </div>

          <div class="space-y-2">
            <label class="text-sm font-medium">Tier</label>
            <Select v-model="createForm.tier">
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectLabel>Tier</SelectLabel>
                  <SelectItem v-for="tier in tierOptions" :key="tier.value" :value="tier.value">
                    {{ tier.label }}
                  </SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>

          <div v-if="createError" class="text-sm text-destructive">{{ createError }}</div>

          <DialogFooter>
            <Button type="button" variant="outline" @click="showCreateModal = false">Cancel</Button>
            <Button type="submit">Create Key</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>

    <!-- Edit Modal -->
    <Dialog v-model:open="showEditModal">
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Edit API Key</DialogTitle>
          <DialogDescription>
            Update the API key metadata.
          </DialogDescription>
        </DialogHeader>

        <form @submit.prevent="handleUpdate" class="space-y-4">
          <div class="space-y-2">
            <label for="edit-key-label" class="text-sm font-medium">Label</label>
            <Input id="edit-key-label" v-model="editForm.label!" />
          </div>

          <div class="space-y-2">
            <label class="text-sm font-medium">Scopes</label>
            <div class="flex gap-4">
              <label
                v-for="scope in scopesOptions"
                :key="scope.value"
                class="flex items-center gap-2"
              >
                <input
                  type="checkbox"
                  :value="scope.value"
                  v-model="editForm.scopes!"
                  class="rounded border-input"
                />
                <span class="text-sm">{{ scope.label }}</span>
              </label>
            </div>
          </div>

          <div class="space-y-2">
            <label class="text-sm font-medium">Tier</label>
            <Select v-model="editForm.tier!">
              <SelectTrigger>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectLabel>Tier</SelectLabel>
                  <SelectItem v-for="tier in tierOptions" :key="tier.value" :value="tier.value">
                    {{ tier.label }}
                  </SelectItem>
                </SelectGroup>
              </SelectContent>
            </Select>
          </div>

          <div v-if="editError" class="text-sm text-destructive">{{ editError }}</div>

          <DialogFooter>
            <Button type="button" variant="outline" @click="showEditModal = false">Cancel</Button>
            <Button type="submit">Save Changes</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>

    <!-- Revoke Confirmation -->
    <Dialog v-model:open="showRevokeConfirm">
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Revoke API Key</DialogTitle>
          <DialogDescription>
            This will immediately invalidate the API key. External agents using this key will no
            longer be able to authenticate. This action cannot be undone.
          </DialogDescription>
        </DialogHeader>

        <div v-if="revokeError" class="text-sm text-destructive">{{ revokeError }}</div>

        <DialogFooter>
          <Button variant="outline" @click="showRevokeConfirm = false">Cancel</Button>
          <Button variant="destructive" @click="handleRevoke">Revoke Key</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  </div>
</template>
