<script setup lang="ts">
import {Key, Pencil, Plus, RefreshCw, Trash2} from "@lucide/vue";
import {computed, onMounted, ref, watch} from "vue";
import {useI18n} from "vue-i18n";
import ApiKeyForm, {type ApiKeyFormState} from "@/components/ApiKeyForm.vue";
import CopyButton from "@/components/CopyButton.vue";
import ErrorBanner from "@/components/ErrorBanner.vue";
import KeyDisplayCard from "@/components/KeyDisplayCard.vue";
import LoadingState from "@/components/LoadingState.vue";
import PageHeader from "@/components/PageHeader.vue";
import Pagination from "@/components/Pagination.vue";
import StatusBadge from "@/components/StatusBadge.vue";
import {Button} from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {useApiKeys} from "@/composables/useApiKeys";
import {useAvailableModels} from "@/composables/useAvailableModels";
import {useProviders} from "@/composables/useProviders";
import {DEFAULT_SCOPES, SCOPES} from "@/config/scopes";
import type {CreateApiKeyRequest, UpdateApiKeyRequest} from "@/lib/api";

const {t} = useI18n();

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
  rotate,
  nextPage,
  prevPage,
} = useApiKeys();

// Tier options are static (driven by the rate limiter in the backend).
const TIER_OPTIONS = [
  {value: "free", label: "Free", description: "100 req burst / ~10 req/min"},
  {value: "pro", label: "Pro", description: "1,000 req burst / ~100 req/min"},
  {
    value: "enterprise",
    label: "Enterprise",
    description: "10,000 req burst / ~1,000 req/min",
  },
];

function buildDefaultFormState(): ApiKeyFormState {
  return {
    label: "",
    scopes: [...DEFAULT_SCOPES],
    tier: "enterprise",
    expiresAt: null,
    allowedModels: [],
    allowedProviders: [],
  };
}

// Create modal state
const showCreateModal = ref(false);
const createForm = ref<ApiKeyFormState>(buildDefaultFormState());
const createError = ref<string | null>(null);
const newlyCreatedKey = ref<string | null>(null);

// Edit modal state
const showEditModal = ref(false);
const editingKey = ref<string | null>(null);
const editForm = ref<ApiKeyFormState>(buildDefaultFormState());
const editError = ref<string | null>(null);

// Revoke confirmation
const showRevokeConfirm = ref(false);
const revokingKeyId = ref<string | null>(null);
const revokeError = ref<string | null>(null);

// Rotate confirmation
const showRotateConfirm = ref(false);
const rotatingKeyId = ref<string | null>(null);
const rotateError = ref<string | null>(null);
const newlyRotatedKey = ref<string | null>(null);
const showRotatedKey = ref(false);

// Providers and available models for the form
const {providers, fetch: fetchProviders} = useProviders();
const {modelsByProvider, fetch: fetchAvailableModels} = useAvailableModels();

// Visibility toggle for newly created key
const showNewKey = ref(false);

onMounted(async () => {
  await Promise.all([fetch(), fetchProviders(), fetchAvailableModels()]);
});

// Clear ephemeral key state whenever the create modal closes via any path.
watch(showCreateModal, (isOpen) => {
  if (!isOpen) {
    newlyCreatedKey.value = null;
    showNewKey.value = false;
  }
});

// Clear ephemeral key state whenever the rotate modal closes via any path.
watch(showRotateConfirm, (isOpen) => {
  if (!isOpen) {
    newlyRotatedKey.value = null;
    showRotatedKey.value = false;
  }
});

function formatDate(dateStr: string | null): string {
  if (!dateStr) return "—";
  return new Date(dateStr).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function maskKey(keyPrefix: string): string {
  return keyPrefix + "...";
}

async function handleCreate() {
  createError.value = null;
  if (!createForm.value.label.trim()) {
    createError.value = "Label is required";
    return;
  }
  if (createForm.value.scopes.length === 0) {
    createError.value = "At least one scope is required";
    return;
  }

  const submissionData: CreateApiKeyRequest = {
    label: createForm.value.label,
    scopes: createForm.value.scopes,
    tier: createForm.value.tier,
    expiresAt: createForm.value.expiresAt,
    allowedModels: createForm.value.allowedModels,
    allowedProviders: createForm.value.allowedProviders,
  };

  const result = await create(submissionData);
  if (result) {
    newlyCreatedKey.value = result.plaintextKey;
    showNewKey.value = true;
    createForm.value = buildDefaultFormState();
  } else {
    createError.value = error.value || "Failed to create API key";
  }
}

function closeCreateWithKey() {
  showCreateModal.value = false;
  newlyCreatedKey.value = null;
  showNewKey.value = false;
}

function openEditModal(key: (typeof apiKeys.value)[0]) {
  editingKey.value = key.id;
  editForm.value = {
    label: key.label,
    scopes: [...key.scopes],
    tier: key.tier,
    isActive: key.isActive,
    expiresAt: key.expiresAt,
    allowedModels: [...(key.allowedModels || [])],
    allowedProviders: [...(key.allowedProviders || [])],
  };
  showEditModal.value = true;
}

async function handleUpdate() {
  if (!editingKey.value) return;
  editError.value = null;

  const submissionData: UpdateApiKeyRequest = {
    label: editForm.value.label,
    scopes: editForm.value.scopes,
    tier: editForm.value.tier,
    isActive: editForm.value.isActive,
    expiresAt: editForm.value.expiresAt,
    allowedModels: editForm.value.allowedModels,
    allowedProviders: editForm.value.allowedProviders,
  };

  const result = await update(editingKey.value, submissionData);
  if (result) {
    showEditModal.value = false;
    editingKey.value = null;
    editForm.value = buildDefaultFormState();
  } else {
    editError.value = error.value || "Failed to update API key";
  }
}

function confirmRevoke(keyId: string) {
  revokingKeyId.value = keyId;
  showRevokeConfirm.value = true;
  revokeError.value = null;
}

async function handleRevoke() {
  if (!revokingKeyId.value) return;

  const success = await revoke(revokingKeyId.value);
  if (success) {
    showRevokeConfirm.value = false;
    revokingKeyId.value = null;
  } else {
    revokeError.value = error.value || "Failed to revoke API key";
  }
}

function confirmRotate(keyId: string) {
  rotatingKeyId.value = keyId;
  showRotateConfirm.value = true;
  rotateError.value = null;
}

async function handleRotate() {
  if (!rotatingKeyId.value) return;

  const result = await rotate(rotatingKeyId.value);
  if (result) {
    newlyRotatedKey.value = result.plaintextKey;
    showRotatedKey.value = true;
    showRotateConfirm.value = false;
    rotatingKeyId.value = null;
  } else {
    rotateError.value = error.value || "Failed to rotate API key";
  }
}

function closeRotateWithKey() {
  showRotateConfirm.value = false;
  newlyRotatedKey.value = null;
  showRotatedKey.value = false;
}

const hasNextPage = computed(() => offset.value + limit.value < total.value);
const hasPrevPage = computed(() => offset.value > 0);

function getKeyStatus(
  status: "active" | "revoked",
  allowedModels?: string[],
  allowedProviders?: string[],
) {
  if (status === "revoked") return "revoked";
  if (!allowedModels?.length && !allowedProviders?.length)
    return "unrestricted";
  return "restricted";
}
</script>

<template>
  <div class="space-y-6">
    <PageHeader
      :title="t('nav.apiKeys')"
      :description="t('apiKeys.description') || 'Manage API keys for external agents'"
    >
      <template #default>
        <Button @click="showCreateModal = true">
          <Plus class="h-4 w-4 mr-2" />
          {{ t('apiKeys.create') || 'Create API Key' }}
        </Button>
      </template>
    </PageHeader>

    <!-- Error State -->
    <ErrorBanner v-if="error" :message="error" @retry="fetch">
      <template #default>
        <Button variant="ghost" size="sm" @click="fetch">
          <RefreshCw class="h-4 w-4 mr-1" />
          Retry
        </Button>
      </template>
    </ErrorBanner>

    <!-- Loading State -->
    <LoadingState v-if="loading && apiKeys.length === 0" />

    <!-- Keys List -->
    <div v-else class="rounded-md border">
      <Table>
        <TableHeader>
          <TableRow class="bg-muted/50">
            <TableHead class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.name') || 'Name' }}</TableHead>
            <TableHead class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.keyPrefix') || 'Key' }}</TableHead>
            <TableHead class="px-4 py-3 text-left text-sm font-medium">Scopes</TableHead>
            <TableHead class="px-4 py-3 text-left text-sm font-medium">Tier</TableHead>
            <TableHead class="px-4 py-3 text-left text-sm font-medium">Status</TableHead>
            <TableHead class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.created') || 'Created' }}</TableHead>
            <TableHead class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.lastUsed') || 'Last Used' }}</TableHead>
            <TableHead class="px-4 py-3 text-left text-sm font-medium">Restrictions</TableHead>
            <TableHead class="px-4 py-3 text-right text-sm font-medium">{{ t('common.actions') || 'Actions' }}</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <template v-if="apiKeys.length > 0">
            <TableRow v-for="item in apiKeys" :key="item.id" class="hover:bg-muted/30">
              <TableCell class="px-4 py-3">
                <div class="flex items-center gap-2">
                  <Key class="h-4 w-4 text-muted-foreground" />
                  <span class="font-medium">{{ item.label }}</span>
                </div>
              </TableCell>
              <TableCell class="px-4 py-3">
                <code class="text-sm font-mono text-muted-foreground">
                  {{ maskKey(item.keyPrefix) }}
                </code>
              </TableCell>
              <TableCell class="px-4 py-3">
                <div class="flex gap-1">
                  <span
                    v-for="scope in item.scopes"
                    :key="scope"
                    class="inline-flex items-center rounded-full bg-muted px-2 py-0.5 text-xs font-medium"
                  >
                    {{ scope }}
                  </span>
                </div>
              </TableCell>
              <TableCell class="px-4 py-3">
                <span class="text-sm capitalize text-muted-foreground">{{ item.tier }}</span>
              </TableCell>
              <TableCell class="px-4 py-3">
                <StatusBadge :status="item.isActive ? 'active' : 'revoked'" />
              </TableCell>
              <TableCell class="px-4 py-3 text-sm text-muted-foreground">{{ formatDate(item.createdAt) }}</TableCell>
              <TableCell class="px-4 py-3 text-sm text-muted-foreground">{{ formatDate(item.lastUsedAt) }}</TableCell>
              <TableCell class="px-4 py-3">
                <StatusBadge :status="getKeyStatus(item.isActive ? 'active' : 'revoked', item.allowedModels, item.allowedProviders)" />
              </TableCell>
              <TableCell class="px-4 py-3 text-right">
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
                    @click="confirmRotate(item.id)"
                  >
                    <RefreshCw class="h-4 w-4" />
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
              </TableCell>
            </TableRow>
          </template>
          <TableRow v-else>
            <TableCell :colspan="9" class="p-8 text-center text-muted-foreground">
              <div class="flex flex-col items-center">
                <Key class="h-12 w-12 mb-4 opacity-50" />
                <p>{{ t('apiKeys.empty') || 'No API keys yet' }}</p>
                <p class="text-sm mt-1">Create your first API key to enable external agent access</p>
              </div>
            </TableCell>
          </TableRow>
        </TableBody>
      </Table>
    </div>

    <!-- Pagination -->
    <Pagination
      v-if="apiKeys.length > 0"
      :offset="offset"
      :limit="limit"
      :total="total"
      :has-prev="hasPrevPage"
      :has-next="hasNextPage"
      :on-prev="prevPage"
      :on-next="nextPage"
    />

    <!-- Create Modal -->
    <Dialog v-model:open="showCreateModal">
      <DialogContent class="max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Create API Key</DialogTitle>
          <DialogDescription>
            Create a new API key for external agent authentication. The key will only be
            shown once — save it securely.
          </DialogDescription>
        </DialogHeader>

        <!-- Newly created key display -->
        <KeyDisplayCard
          v-if="newlyCreatedKey"
          :api-key="newlyCreatedKey"
          :warning-text="t('apiKeys.warning.saveNow') || 'Save this key now — it will not be shown again'"
          :on-done="closeCreateWithKey"
        >
          <template #copy>
            <CopyButton :text="newlyCreatedKey" />
          </template>
        </KeyDisplayCard>

        <!-- Create form -->
        <ApiKeyForm
          v-else
          v-model="createForm"
          :scopes="SCOPES"
          :providers="providers"
          :models-by-provider="modelsByProvider"
          :tier-options="TIER_OPTIONS"
          :error="createError"
          submit-label="Create Key"
          cancel-label="Cancel"
          @submit="handleCreate"
          @cancel="showCreateModal = false"
        />
      </DialogContent>
    </Dialog>

    <!-- Edit Modal -->
    <Dialog v-model:open="showEditModal">
      <DialogContent class="max-h-[90vh] overflow-y-auto">
        <DialogHeader>
          <DialogTitle>Edit API Key</DialogTitle>
          <DialogDescription>Update the API key metadata.</DialogDescription>
        </DialogHeader>

        <ApiKeyForm
          v-model="editForm"
          :scopes="SCOPES"
          :providers="providers"
          :models-by-provider="modelsByProvider"
          :tier-options="TIER_OPTIONS"
          :error="editError"
          submit-label="Save Changes"
          cancel-label="Cancel"
          :is-edit="true"
          @submit="handleUpdate"
          @cancel="showEditModal = false"
        />
      </DialogContent>
    </Dialog>

    <!-- Revoke Confirmation -->
    <Dialog v-model:open="showRevokeConfirm">
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Revoke API Key</DialogTitle>
          <DialogDescription>
            This will immediately invalidate the API key. External agents using this key
            will no longer be able to authenticate. This action cannot be undone.
          </DialogDescription>
        </DialogHeader>

        <div v-if="revokeError" class="text-sm text-destructive">{{ revokeError }}</div>

        <DialogFooter>
          <Button variant="outline" @click="showRevokeConfirm = false">Cancel</Button>
          <Button variant="destructive" @click="handleRevoke">Revoke Key</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>

    <!-- Rotate Confirmation -->
    <Dialog v-model:open="showRotateConfirm">
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Rotate API Key</DialogTitle>
          <DialogDescription>
            Rotate this key? The old key will stop working immediately and a new key will
            be generated. Make sure to copy the new key — it will only be shown once.
          </DialogDescription>
        </DialogHeader>

        <!-- Newly rotated key display -->
        <KeyDisplayCard
          v-if="newlyRotatedKey"
          :api-key="newlyRotatedKey"
          :warning-text="t('apiKeys.warning.saveNow') || 'Save this key now — it will not be shown again'"
          :on-done="closeRotateWithKey"
        >
          <template #copy>
            <CopyButton :text="newlyRotatedKey" />
          </template>
        </KeyDisplayCard>

        <div v-else>
          <div v-if="rotateError" class="text-sm text-destructive mb-4">{{ rotateError }}</div>
          <DialogFooter>
            <Button variant="outline" @click="showRotateConfirm = false">Cancel</Button>
            <Button @click="handleRotate">Rotate Key</Button>
          </DialogFooter>
        </div>
      </DialogContent>
    </Dialog>
  </div>
</template>
