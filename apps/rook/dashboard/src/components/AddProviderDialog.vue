<script setup lang="ts">
import {
  AlertCircle,
  AlertTriangle,
  CheckCircle2,
  Loader2,
  Trash2,
} from "@lucide/vue";
import {computed, onUnmounted, ref, watch} from "vue";
import {useI18n} from "vue-i18n";
import PasswordInput from "@/components/PasswordInput.vue";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {Button} from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {Input} from "@/components/ui/input";
import {Label} from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {Switch} from "@/components/ui/switch";
import {ToggleGroup, ToggleGroupItem} from "@/components/ui/toggle-group";
import {useProviders} from "@/composables/useProviders";
import {
  type AuthType,
  findCatalogEntry,
  PROVIDER_KINDS,
  type ProviderKind,
} from "@/config/providerCatalog";
import {
  OAUTH_FLOW_TYPES,
  type CreateProviderRequest,
  type OAuthFlowType,
  type ProviderConnectionResponse,
  type TestConnectionResponse,
  type UpdateProviderRequest,
  type WireAuthType,
} from "@/lib/api";

type DialogMode = "create" | "edit";

const props = withDefaults(
  defineProps<{
    /** v-model:open — controlled by parent */
    open?: boolean;
    /** When provided, dialog is scoped to this provider (no kind selector) */
    providerKind?: ProviderKind;
    /** 'create' = new connection, 'edit' = load existing connection */
    mode?: DialogMode;
    /** Required when mode='edit' */
    connectionId?: string;
  }>(),
  {open: false, mode: "create"},
);

const emit = defineEmits<{
  "update:open": [value: boolean];
  saved: [connection: ProviderConnectionResponse];
  deleted: [id: string];
}>();

const {t} = useI18n();
const {create, update, remove, testCredentials, fetch, providerById, startOAuthFlow, getOAuthStatus} =
  useProviders();

// ---------------------------------------------------------------------------
// Form state
// ---------------------------------------------------------------------------

/** Currently selected kind. Defaults to prop or first catalog entry. */
const kind = ref<ProviderKind>(props.providerKind ?? PROVIDER_KINDS[0].kind);
/** Currently selected auth type. OAuth is gated — see supportsOAuth below. */
const authType = ref<AuthType>("apikey");

interface FormState {
  displayName: string;
  apiKey: string;
  baseUrl: string;
  priority: number;
  models: string;
  enabled: boolean;
  oauthClientId: string;
  oauthClientSecret: string;
  oauthTenantId: string;
  oauthProjectId: string;
  oauthFlowType: OAuthFlowType;
}

function blankForm(): FormState {
  return {
    displayName: "",
    apiKey: "",
    baseUrl: findCatalogEntry(kind.value).defaultBaseUrl,
    priority: 50,
    models: "",
    enabled: true,
    oauthClientId: "",
    oauthClientSecret: "",
    oauthTenantId: "",
    oauthProjectId: "",
    oauthFlowType: OAUTH_FLOW_TYPES.AUTHORIZATION_CODE,
  };
}

const form = ref<FormState>(blankForm());
const testing = ref(false);
const saving = ref(false);
const deleting = ref(false);
const testResult = ref<TestConnectionResponse | null>(null);
const showDeleteConfirm = ref(false);
const oauthFlowId = ref<string | null>(null);
const oauthUserCode = ref<string | null>(null);
const oauthVerificationUri = ref<string | null>(null);
const oauthAuthorizationUrl = ref<string | null>(null);
const oauthMessage = ref<string | null>(null);
const oauthBusy = ref(false);
let oauthPollTimer: number | undefined;

// ---------------------------------------------------------------------------
// Derived state
// ---------------------------------------------------------------------------

const isEditMode = computed(() => props.mode === "edit");
const showKindSelector = computed(() => props.providerKind === undefined);
const catalogEntry = computed(() => findCatalogEntry(kind.value));
const isApikey = computed(() => authType.value === "apikey");
const supportsOAuth = computed(() =>
  catalogEntry.value.authTypes.includes("oauth"),
);
const isOAuth = computed(() => authType.value === "oauth");
const canStartOAuth = computed(
  () =>
    !oauthBusy.value &&
    !saving.value &&
    isOAuth.value &&
    form.value.displayName.trim() !== "" &&
    form.value.oauthClientId.trim() !== "" &&
    (kind.value !== "azure-openai" || form.value.baseUrl.startsWith("https://")) &&
    (kind.value !== "azure-openai" || firstModel() !== null) &&
    (kind.value !== "gemini" ||
      isEditMode.value ||
      form.value.oauthProjectId.trim() !== "") &&
    (kind.value === "azure-openai" ||
      form.value.oauthFlowType !== OAUTH_FLOW_TYPES.DEVICE_CODE),
);
// Managed-cloud providers (e.g. Ollama Cloud) ship with a vendor-fixed
// base URL — we still send it to the backend, but the user has no reason
// to edit it and may shoot themselves in the foot if they do. Default
// to `true` to keep current behavior for every other provider.
const baseUrlEditable = computed(
  () => catalogEntry.value.baseUrlEditable ?? true,
);

const canTest = computed(
  () =>
    !testing.value &&
    !saving.value &&
    form.value.displayName.trim() !== "" &&
    (isApikey.value ? form.value.apiKey.trim() !== "" : false),
);

const canSave = computed(
  () => canTest.value && testResult.value?.valid === true && !saving.value,
);

/**
 * Display message for the test-result block. Derived from the wire
 * response so the user sees a single human-readable line regardless
 * of which variant the backend emitted.
 */
const testResultMessage = computed(() => {
  const r = testResult.value;
  if (!r) return "";
  if (r.warning) return r.warning;
  if (r.error) return r.error;
  if (r.status === "ok") {
    return `Connected successfully (${r.latencyMs ?? 0}ms)`;
  }
  return `Status: ${r.status}`;
});

// ---------------------------------------------------------------------------
// Wire format helpers
// ---------------------------------------------------------------------------

/** Internal `apikey` → wire `apiKey` (backend enum uses camelCase). */
function wireAuthType(a: AuthType): WireAuthType {
  return a === "apikey" ? "apiKey" : "oauth";
}

function buildTestPayload() {
  const runtimeId = `${kind.value}-test-${Date.now()}-${crypto.randomUUID().slice(0, 8)}`;
  return {
    providerKind: kind.value,
    providerRuntimeId: runtimeId,
    authType: wireAuthType(authType.value),
    credentials: isApikey.value ? {apiKey: form.value.apiKey} : {},
    config: {
      maxConcurrent: 10,
      quotaWindowThresholds: {warning: 0.8, error: 0.95},
      defaultModel: firstModel() ?? undefined,
      baseUrl: form.value.baseUrl.trim() || undefined,
    },
  };
}

function firstModel(): string | null {
  const first = form.value.models.split(",")[0]?.trim() ?? "";
  return first === "" ? null : first;
}

function buildCreateRequest(): CreateProviderRequest {
  const runtimeId = `${kind.value}-${Date.now()}-${crypto.randomUUID().slice(0, 8)}`;
  return {
    providerKind: kind.value,
    providerRuntimeId: runtimeId,
    authType: wireAuthType(authType.value),
    name: form.value.displayName.trim(),
    priority: form.value.priority,
    isActive: form.value.enabled,
    credentials: isApikey.value
      ? {apiKey: form.value.apiKey}
      : {apiKey: ""},
    config: {
      maxConcurrent: 10,
      quotaWindowThresholds: {warning: 0.8, error: 0.95},
      defaultModel: firstModel() ?? undefined,
      baseUrl: form.value.baseUrl.trim() || undefined,
    },
  };
}

function buildUpdateRequest(
  existing: ProviderConnectionResponse,
): UpdateProviderRequest {
  // API key is only sent when the user typed a new one (stored secrets
  // are never re-exposed on edit, see spec REQ-3 Scenario "Edit existing").
  const credentials =
    isApikey.value && form.value.apiKey.trim() !== ""
      ? {apiKey: form.value.apiKey}
      : undefined;
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
      quotaWindowThresholds: {warning: 0.8, error: 0.95},
      defaultModel: firstModel() ?? undefined,
      baseUrl: form.value.baseUrl.trim() || undefined,
    },
  };
}

// ---------------------------------------------------------------------------
// Lifecycle: load on open, reset on close
// ---------------------------------------------------------------------------

function loadFromConnection(conn: ProviderConnectionResponse) {
  kind.value = conn.providerKind;
  authType.value = conn.authType === "oauth" ? "oauth" : "apikey";
  form.value.displayName = conn.name;
  form.value.baseUrl =
    conn.config.baseUrl ?? findCatalogEntry(kind.value).defaultBaseUrl;
  form.value.priority = conn.priority;
  form.value.models = conn.config.defaultModel ?? "";
  form.value.enabled = conn.isActive;
  form.value.apiKey = ""; // never re-expose stored secret
  form.value.oauthClientId = conn.oauthClientId ?? "";
  form.value.oauthTenantId = conn.oauthTenantId ?? "";
  form.value.oauthProjectId = ""; // project IDs are kept out of connection DTOs
  testResult.value = null;
}

function resetForm() {
  stopOAuthPolling();
  oauthFlowId.value = null;
  oauthUserCode.value = null;
  oauthVerificationUri.value = null;
  oauthAuthorizationUrl.value = null;
  oauthMessage.value = null;
  oauthBusy.value = false;
  kind.value = props.providerKind ?? PROVIDER_KINDS[0].kind;
  authType.value = findCatalogEntry(kind.value).authTypes.includes("apikey")
    ? "apikey"
    : "oauth";
  form.value = blankForm();
  testResult.value = null;
  showDeleteConfirm.value = false;
}

watch(
  () => props.open,
  async (isOpen) => {
    if (!isOpen) {
      resetForm();
      return;
    }
    if (isEditMode.value && props.connectionId) {
      // Use cache if we already have the connection, otherwise fetch the list.
      let conn = providerById.value.get(props.connectionId);
      if (!conn) {
        await fetch();
        conn = providerById.value.get(props.connectionId);
      }
      if (conn) loadFromConnection(conn);
    } else {
      // Create mode — seed from current kind.
      if (props.providerKind) kind.value = props.providerKind;
      authType.value = findCatalogEntry(kind.value).authTypes.includes("apikey")
        ? "apikey"
        : "oauth";
      form.value = blankForm();
      testResult.value = null;
    }
  },
  {immediate: true},
);

// When the user changes the kind manually (create mode), keep baseUrl in sync
// with the catalog default so the user always starts from a known-good URL.
watch(kind, (newKind) => {
  if (!isEditMode.value) {
    form.value.baseUrl = findCatalogEntry(newKind).defaultBaseUrl;
    if (!findCatalogEntry(newKind).authTypes.includes(authType.value)) {
      authType.value = findCatalogEntry(newKind).authTypes[0] ?? "oauth";
    }
  }
});

// ---------------------------------------------------------------------------
// Event handlers
// ---------------------------------------------------------------------------

function handleOpenChange(value: boolean) {
  emit("update:open", value);
}

async function handleTest() {
  if (!canTest.value) return;
  testing.value = true;
  testResult.value = null;
  try {
    const result = await testCredentials(buildTestPayload());
    if (result) {
      testResult.value = result;
    } else {
      testResult.value = {
        valid: false,
        status: "unhealthy",
        latencyMs: null,
        error: "Connection failed",
        warning: null,
        method: null,
      };
    }
  } catch (e) {
    testResult.value = {
      valid: false,
      status: "unhealthy",
      latencyMs: null,
      error: e instanceof Error ? e.message : "Test failed",
      warning: null,
      method: null,
    };
  } finally {
    testing.value = false;
  }
}

function stopOAuthPolling() {
  if (oauthPollTimer !== undefined) {
    globalThis.clearInterval(oauthPollTimer);
    oauthPollTimer = undefined;
  }
}

async function handleOAuthConnect() {
  if (!canStartOAuth.value) return;
  oauthBusy.value = true;
  oauthMessage.value = t("providers.oauth.starting");
  const authWindow =
    form.value.oauthFlowType === OAUTH_FLOW_TYPES.AUTHORIZATION_CODE
      ? globalThis.open("about:blank", "_blank")
      : null;
  const existing =
    isEditMode.value && props.connectionId
      ? providerById.value.get(props.connectionId)
      : undefined;
  const runtimeId =
    existing?.providerRuntimeId ??
    `${kind.value}-${Date.now()}-${crypto.randomUUID().slice(0, 8)}`;
  const redirectUri = new URL(
    "/api/oauth/callback",
    globalThis.location.origin,
  ).toString();
  const started = await startOAuthFlow(kind.value, {
    flowType: form.value.oauthFlowType,
    connectionId: existing?.id,
    providerRuntimeId: runtimeId,
    name: form.value.displayName.trim(),
    priority: form.value.priority,
    isActive: form.value.enabled,
    config: {
      maxConcurrent: 10,
      quotaWindowThresholds: {warning: 0.8, error: 0.95},
      defaultModel: firstModel() ?? undefined,
      baseUrl: form.value.baseUrl.trim() || undefined,
    },
    clientId: form.value.oauthClientId.trim(),
    clientSecret: form.value.oauthClientSecret.trim() || undefined,
    tenantId: form.value.oauthTenantId.trim() || undefined,
    redirectUri,
    projectId: form.value.oauthProjectId.trim() || undefined,
  });
  if (!started) {
    authWindow?.close();
    oauthBusy.value = false;
    oauthMessage.value = t("providers.oauth.startFailed");
    return;
  }

  oauthFlowId.value = started.flowId;
  oauthUserCode.value = started.userCode;
  oauthVerificationUri.value = started.verificationUri;
  oauthAuthorizationUrl.value = started.authorizationUrl;
  oauthMessage.value = started.message ?? t("providers.oauth.waiting");
  if (started.authorizationUrl) {
    if (authWindow) {
      authWindow.opener = null;
      authWindow.location.href = started.authorizationUrl;
    } else {
      oauthMessage.value = t("providers.oauth.openAuthorizationLink");
    }
  }
  oauthPollTimer = globalThis.setInterval(
    () => void pollOAuthStatus(),
    Math.max((started.intervalSecs ?? 2) * 1000, 2000),
  );
  await pollOAuthStatus();
}

async function pollOAuthStatus() {
  if (!oauthFlowId.value) return;
  const status = await getOAuthStatus(kind.value, {flowId: oauthFlowId.value});
  if (!status) return;
  oauthMessage.value = status.message ?? t("providers.oauth.waiting");
  if (status.status === "connected" && status.connectionId) {
    stopOAuthPolling();
    await fetch();
    const connection = providerById.value.get(status.connectionId);
    if (connection) {
      emit("saved", connection);
      emit("update:open", false);
    } else {
      oauthMessage.value = t("providers.oauth.connectionNotFound");
    }
    oauthBusy.value = false;
  } else if (status.status === "failed" || status.status === "expired") {
    stopOAuthPolling();
    oauthBusy.value = false;
  }
}

async function handleSave() {
  if (!canSave.value) return;
  saving.value = true;
  try {
    if (isEditMode.value && props.connectionId) {
      const existing = providerById.value.get(props.connectionId);
      if (!existing) {
        testResult.value = {
          valid: false,
          status: "unhealthy",
          latencyMs: null,
          error: "Connection not found",
          warning: null,
          method: null,
        };
        return;
      }
      const updated = await update(
        props.connectionId,
        buildUpdateRequest(existing),
      );
      if (updated) {
        emit("saved", updated);
        emit("update:open", false);
      }
    } else {
      const created = await create(buildCreateRequest());
      if (created) {
        await fetch();
        emit("saved", created);
        emit("update:open", false);
      }
    }
  } finally {
    saving.value = false;
  }
}

async function handleDelete() {
  if (!isEditMode.value || !props.connectionId) return;
  deleting.value = true;
  try {
    const ok = await remove(props.connectionId);
    if (ok) {
      emit("deleted", props.connectionId);
      showDeleteConfirm.value = false;
      emit("update:open", false);
    }
  } finally {
    deleting.value = false;
  }
}

onUnmounted(stopOAuthPolling);
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <DialogContent class="sm:max-w-140">
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
            id="kind"
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
          <span id="provider-auth-type-label" class="text-sm font-medium">
            {{ t('providers.form.authType') }}
          </span>
          <ToggleGroup
            v-model="authType"
            type="single"
            aria-labelledby="provider-auth-type-label"
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

        <div v-if="isOAuth" class="space-y-4 rounded-md border p-3">
          <div class="space-y-2">
            <Label for="oauthClientId">{{ t('providers.oauth.clientId') }}</Label>
            <Input
              id="oauthClientId"
              v-model="form.oauthClientId"
              autocomplete="off"
              autocapitalize="off"
              spellcheck="false"
              :disabled="oauthBusy"
              data-testid="oauth-client-id"
            />
          </div>
          <div class="space-y-2">
            <Label for="oauthClientSecret">{{ t('providers.oauth.clientSecret') }}</Label>
            <PasswordInput
              id="oauthClientSecret"
              v-model="form.oauthClientSecret"
              :disabled="oauthBusy"
              data-testid="oauth-client-secret"
            />
          </div>
          <div v-if="kind === 'azure-openai'" class="space-y-2">
            <Label for="oauthTenantId">{{ t('providers.oauth.tenantId') }}</Label>
            <Input
              id="oauthTenantId"
              v-model="form.oauthTenantId"
              autocomplete="off"
              autocapitalize="off"
              spellcheck="false"
              :disabled="oauthBusy"
              data-testid="oauth-tenant-id"
            />
            <Label id="oauthFlowTypeLabel" for="oauthFlowType">
              {{ t('providers.oauth.flowType') }}
            </Label>
            <Select
              id="oauthFlowType"
              v-model="form.oauthFlowType"
              :disabled="oauthBusy"
            >
              <SelectTrigger id="oauthFlowType" aria-labelledby="oauthFlowTypeLabel">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem :value="OAUTH_FLOW_TYPES.AUTHORIZATION_CODE">
                  {{ t('providers.oauth.authorizationCode') }}
                </SelectItem>
                <SelectItem :value="OAUTH_FLOW_TYPES.DEVICE_CODE">
                  {{ t('providers.oauth.deviceCode') }}
                </SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div v-if="kind === 'gemini'" class="space-y-2">
            <Label for="oauthProjectId">{{ t('providers.oauth.googleProjectId') }}</Label>
            <Input
              id="oauthProjectId"
              v-model="form.oauthProjectId"
              autocomplete="off"
              autocapitalize="off"
              spellcheck="false"
              :disabled="oauthBusy"
              required
              data-testid="oauth-project-id"
            />
          </div>
          <output v-if="oauthMessage" class="space-y-2" aria-live="polite">
            <p class="text-sm">{{ oauthMessage }}</p>
            <p v-if="oauthUserCode" class="font-mono text-lg font-semibold">
              {{ oauthUserCode }}
            </p>
            <a
              v-if="oauthVerificationUri"
              :href="oauthVerificationUri"
              target="_blank"
              rel="noopener noreferrer"
              class="text-sm underline underline-offset-4"
            >
              {{ t('providers.oauth.openVerificationPage') }}
            </a>
            <a
              v-if="oauthAuthorizationUrl && !oauthUserCode"
              :href="oauthAuthorizationUrl"
              target="_blank"
              rel="noopener noreferrer"
              class="text-sm underline underline-offset-4"
            >
              {{ t('providers.oauth.openAuthorizationPage') }}
            </a>
          </output>
        </div>

        <!-- Base URL (hidden for managed-cloud providers where the
             endpoint is vendor-fixed, e.g. Ollama Cloud) -->
        <div v-if="baseUrlEditable" class="space-y-2">
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
            :placeholder="
              kind === 'azure-openai'
                ? t('providers.oauth.azureDeploymentPlaceholder')
                : t('providers.form.modelsPlaceholder')
            "
            :disabled="saving"
            data-testid="input-models"
          />
          <p class="text-xs text-muted-foreground">
            {{
              kind === 'azure-openai'
                ? t('providers.oauth.azureDeploymentHelp')
                : t('providers.form.modelsHelp')
            }}
          </p>
        </div>

        <!-- Enabled -->
        <div class="flex items-center justify-between rounded-md border px-3 py-2">
          <Label for="enabled">{{ t('providers.form.enabled') }}</Label>
          <Switch
            id="enabled"
            v-model="form.enabled"
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
            v-if="testResult.valid && testResult.status === 'ok'"
            class="h-5 w-5 text-green-500 mt-0.5"
            data-testid="test-result-icon-ok"
          />
          <AlertTriangle
            v-else-if="testResult.valid && testResult.status === 'warning'"
            class="h-5 w-5 text-yellow-600 mt-0.5"
            data-testid="test-result-icon-warning"
          />
          <AlertCircle
            v-else-if="testResult.status === 'unknown'"
            class="h-5 w-5 text-muted-foreground mt-0.5"
            data-testid="test-result-icon-unknown"
          />
          <AlertCircle
            v-else
            class="h-5 w-5 text-destructive mt-0.5"
            data-testid="test-result-icon-error"
          />
          <div class="flex-1">
            <p
              class="text-sm font-medium"
              :class="
                testResult.status === 'ok'
                  ? 'text-green-600'
                  : testResult.status === 'warning'
                    ? 'text-yellow-600'
                    : testResult.status === 'unknown'
                      ? 'text-muted-foreground'
                      : 'text-destructive'
              "
              data-testid="test-result-status"
            >
              {{
                testResult.status === 'ok'
                  ? t('providers.form.testSuccess')
                  : testResult.status === 'warning'
                    ? t('providers.form.testWarning')
                    : testResult.status === 'unknown'
                      ? t('providers.form.testUnknown')
                      : t('providers.form.testFailed')
              }}
            </p>
            <p class="text-xs text-muted-foreground mt-0.5" data-testid="test-result-message">
              {{ testResultMessage }}
            </p>
          </div>
        </div>
      </div>

      <DialogFooter class="flex-col sm:flex-row gap-2">
        <div class="flex gap-2">
          <Button
            v-if="!isOAuth"
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
            v-if="!isOAuth"
            :disabled="!canSave"
            data-testid="save-button"
            @click="handleSave"
          >
            <Loader2 v-if="saving" class="h-4 w-4 mr-2 animate-spin" />
            {{ t('providers.form.save') }}
          </Button>
          <Button
            v-if="isOAuth"
            :disabled="!canStartOAuth || oauthBusy"
            data-testid="oauth-connect-button"
            @click="handleOAuthConnect"
          >
            <Loader2 v-if="oauthBusy" class="h-4 w-4 mr-2 animate-spin" />
            {{ oauthBusy ? t('providers.oauth.waiting') : t('providers.oauth.connect') }}
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
