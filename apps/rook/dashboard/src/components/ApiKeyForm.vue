<script lang="ts" setup>
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

import {AlertTriangle, ChevronDown, Key, ShieldAlert} from "@lucide/vue";
import {computed} from "vue";
import {useI18n} from "vue-i18n";
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion";
import {Badge} from "@/components/ui/badge";
import {Button} from "@/components/ui/button";
import {Checkbox} from "@/components/ui/checkbox";
import {Input} from "@/components/ui/input";
import {NativeSelect, NativeSelectOption} from "@/components/ui/native-select";
import type {ModelsByProvider} from "@/composables/useAvailableModels";
import {PROVIDER_KINDS, type ProviderKind,} from "@/config/providerCatalog";
import type {ScopeDef, ScopeGroup} from "@/config/scopes";
import type {ProviderConnectionResponse} from "@/lib/api";

export interface ApiKeyFormState {
  label: string;
  scopes: string[];
  tier: string;
  expiresAt: string | null;
  allowedModels: string[];
  allowedProviders: string[];
  isActive?: boolean;
  spendingBudgetEnabled: boolean;
  spendingBudgetAmountUsd: string;
  spendingBudgetRequestLimitUsd: string;
  spendingBudgetWindow: "daily" | "monthly";
}

interface Props {
  modelValue: ApiKeyFormState;
  scopes: readonly ScopeDef[];
  providers: ProviderConnectionResponse[];
  modelsByProvider: ModelsByProvider[];
  tierOptions: { value: string; label: string; description: string }[];
  /** Optional group render order. Defaults to chat, providers, admin. */
  groupOrder?: readonly ScopeGroup[];
  error?: string | null;
  submitLabel?: string;
  cancelLabel?: string;
  isEdit?: boolean;
}

const {t} = useI18n();

const props = withDefaults(defineProps<Props>(), {
  groupOrder: () => ["chat", "providers", "admin"] as const,
  error: null,
  submitLabel: "Save",
  cancelLabel: "Cancel",
  isEdit: false,
});

const emit = defineEmits<{
  "update:modelValue": [value: ApiKeyFormState];
  submit: [];
  cancel: [];
}>();

const spendingLimitExceedsBudget = computed(() => {
  const amount = Number(props.modelValue.spendingBudgetAmountUsd);
  const requestLimit = Number(props.modelValue.spendingBudgetRequestLimitUsd);
  return props.modelValue.spendingBudgetEnabled
    && amount > 0
    && requestLimit > 0
    && requestLimit > amount;
});

function setBudgetEnabled(enabled: boolean) {
  update("spendingBudgetEnabled", enabled);
}

function update<K extends keyof ApiKeyFormState>(
  key: K,
  value: ApiKeyFormState[K],
) {
  emit("update:modelValue", {...props.modelValue, [key]: value});
}

function toggleScope(scopeValue: string, checked: boolean) {
  const next = checked
    ? Array.from(new Set([...props.modelValue.scopes, scopeValue]))
    : props.modelValue.scopes.filter((s) => s !== scopeValue);
  update("scopes", next);
}

function toggleProvider(providerId: string, checked: boolean) {
  const next = checked
    ? Array.from(new Set([...props.modelValue.allowedProviders, providerId]))
    : props.modelValue.allowedProviders.filter((p) => p !== providerId);
  update("allowedProviders", next);
}

function toggleModel(modelId: string, checked: boolean) {
  const next = checked
    ? Array.from(new Set([...props.modelValue.allowedModels, modelId]))
    : props.modelValue.allowedModels.filter((m) => m !== modelId);
  update("allowedModels", next);
}

const groupedScopes = computed(() => {
  return props.groupOrder
    .map((group) => ({
      group,
      scopes: props.scopes.filter((s) => s.group === group),
    }))
    .filter((entry) => entry.scopes.length > 0);
});

/** Group providers by their providerKind for collapsible display. */
const providersByKind = computed(() => {
  const groups = new Map<ProviderKind, ProviderConnectionResponse[]>();
  for (const provider of props.providers) {
    const kind = provider.providerKind;
    if (!groups.has(kind)) {
      groups.set(kind, []);
    }
    groups.get(kind)?.push(provider);
  }
  return groups;
});

/** Get catalog metadata for a provider kind (display name, icon, etc). */
function getCatalogEntry(kind: ProviderKind) {
  return PROVIDER_KINDS.find((p) => p.kind === kind) ?? null;
}

/** Toggle all providers in a specific kind group. */
function toggleKindGroup(kind: ProviderKind, checked: boolean) {
  const providersOfKind = providersByKind.value.get(kind) ?? [];
  const providerIds = providersOfKind.map((p) => p.id);
  if (checked) {
    // Add all provider IDs of this kind
    const newSet = Array.from(
      new Set([...props.modelValue.allowedProviders, ...providerIds]),
    );
    update("allowedProviders", newSet);
  } else {
    // Remove all provider IDs of this kind
    const newSet = props.modelValue.allowedProviders.filter(
      (id) => !providerIds.includes(id),
    );
    update("allowedProviders", newSet);
  }
}

/** Check if all providers of a given kind are selected. */
function isKindGroupChecked(kind: ProviderKind): boolean {
  const providersOfKind = providersByKind.value.get(kind) ?? [];
  if (providersOfKind.length === 0) return false;
  return providersOfKind.every((p) =>
    props.modelValue.allowedProviders.includes(p.id),
  );
}

/** Check if some (but not all) providers of a kind are selected. */
function isKindGroupIndeterminate(kind: ProviderKind): boolean {
  const providersOfKind = providersByKind.value.get(kind) ?? [];
  if (providersOfKind.length === 0) return false;
  const checkedCount = providersOfKind.filter((p) =>
    props.modelValue.allowedProviders.includes(p.id),
  ).length;
  return checkedCount > 0 && checkedCount < providersOfKind.length;
}

const groupLabel: Record<ScopeGroup, string> = {
  chat: "Chat",
  providers: "Providers",
  admin: "Administrative",
};

function isScopeChecked(scope: ScopeDef): boolean {
  return props.modelValue.scopes.includes(scope.value);
}

function isProviderChecked(providerId: string): boolean {
  return props.modelValue.allowedProviders.includes(providerId);
}

function isModelChecked(modelId: string): boolean {
  return props.modelValue.allowedModels.includes(modelId);
}

/** Slugify a scope value so it is safe for use in a data-testid attribute.
 *  e.g. 'chat:read' -> 'chat-read', 'admin' -> 'admin'. */
function scopeSlug(value: string): string {
  return value.replace(/[^a-zA-Z0-9_-]/g, "-");
}
</script>

<template>
  <form
    class="space-y-6"
    data-testid="api-key-form"
    @submit.prevent="emit('submit')"
  >
    <!-- Label -->
    <div class="space-y-2">
      <label class="text-sm font-medium" for="api-key-label">Label</label>
      <Input
        id="api-key-label"
        :model-value="modelValue.label"
        data-testid="input-api-key-label"
        placeholder="e.g., opencode-agent, hermes-agent"
        @update:model-value="(v) => update('label', String(v ?? ''))"
      />
    </div>

    <!-- Scopes — grouped, with descriptions and danger indicators -->
    <div class="space-y-2" data-testid="api-key-scopes">
      <p class="text-sm font-medium">Scopes</p>
      <p class="text-xs text-muted-foreground">
        Select what this key is allowed to do. Defaults to least-privilege — enable Admin explicitly
        if needed.
      </p>
      <div class="space-y-4 pt-2">
        <div
          v-for="entry in groupedScopes"
          :key="entry.group"
          :data-testid="`scope-group-${entry.group}`"
          class="space-y-2"
        >
          <p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {{ groupLabel[entry.group] }}
          </p>
          <div class="space-y-2">
            <label
              v-for="scope in entry.scopes"
              :key="scope.value"
              :class="scope.danger ? 'border-destructive/30 bg-destructive/5' : ''"
              :data-scope-value="scope.value"
              :data-testid="`scope-row-${scopeSlug(scope.value)}`"
              :for="`scope-checkbox-${scopeSlug(scope.value)}`"
              class="flex items-start gap-3 rounded-md border p-3 hover:bg-muted/30 cursor-pointer"
            >
              <Checkbox
                :id="`scope-checkbox-${scopeSlug(scope.value)}`"
                :aria-label="scope.label"
                :data-testid="`scope-checkbox-${scopeSlug(scope.value)}`"
                :model-value="isScopeChecked(scope)"
                @update:model-value="(v) => toggleScope(scope.value, v === true)"
              />
              <div class="flex-1 space-y-0.5">
                <div class="flex items-center gap-2">
                  <code class="text-sm font-mono font-semibold">{{ scope.label }}</code>
                  <Badge
                    v-if="scope.danger"
                    :data-testid="`scope-danger-${scopeSlug(scope.value)}`"
                    class="gap-1"
                    variant="destructive"
                  >
                    <ShieldAlert class="h-3 w-3"/>
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
      <label class="text-sm font-medium" for="api-key-tier">Rate limit tier</label>
      <NativeSelect
        id="api-key-tier"
        :model-value="modelValue.tier"
        aria-label="Rate limit tier"
        data-testid="api-key-tier"
        @update:model-value="(v) => update('tier', String(v ?? modelValue.tier))"
      >
        <NativeSelectOption
          v-for="opt in tierOptions"
          :key="opt.value"
          :value="opt.value"
        >
          {{ opt.label }} — {{ opt.description }}
        </NativeSelectOption>
      </NativeSelect>
      <p class="text-xs text-muted-foreground">
        Controls the request-rate bucket for this key. Default: highest tier.
      </p>
    </div>

    <!-- Allowed Providers (grouped by providerKind) -->
    <div class="space-y-2" data-testid="api-key-providers">
      <p class="text-sm font-medium">Allowed providers</p>
      <p class="text-xs text-muted-foreground">
        Leave empty to allow all providers.
      </p>
      <div v-if="providers.length === 0" class="text-xs text-muted-foreground italic">
        No providers configured.
      </div>
      <Accordion v-else class="w-full" type="multiple">
        <AccordionItem
          v-for="[kind, kindProviders] in providersByKind"
          :key="kind"
          :value="kind"
        >
          <AccordionTrigger class="py-2">
            <div class="flex items-center gap-2 w-full pr-2">
              <!-- Group-level select all checkbox -->
              <Checkbox
                :data-testid="`provider-kind-checkbox-${kind}`"
                :indeterminate="isKindGroupIndeterminate(kind)"
                :model-value="isKindGroupChecked(kind)"
                @update:model-value="(checked: boolean | 'indeterminate') => { if (typeof checked === 'boolean') toggleKindGroup(kind, checked); }"
              />
              <span class="text-sm font-medium">
                {{
                  getCatalogEntry(kind)?.displayNameKey
                    ? $t(getCatalogEntry(kind)?.displayNameKey ?? kind)
                    : kind
                }}
              </span>
              <Badge class="text-xs" variant="secondary">
                {{ kindProviders.length }}
              </Badge>
              <!-- Show selected count -->
              <span class="text-xs text-muted-foreground ml-auto">
                {{
                  kindProviders.filter(p => isProviderChecked(p.id)).length
                }}/{{ kindProviders.length }}
              </span>
              <ChevronDown class="h-4 w-4 text-muted-foreground shrink-0"/>
            </div>
          </AccordionTrigger>
          <AccordionContent>
            <div class="pl-6 space-y-1 py-2">
              <label
                v-for="provider in kindProviders"
                :key="provider.id"
                :data-testid="`provider-row-${provider.id}`"
                :for="`provider-checkbox-${provider.id}`"
                class="flex items-center gap-2 cursor-pointer"
              >
                <Checkbox
                  :id="`provider-checkbox-${provider.id}`"
                  :aria-label="provider.name"
                  :data-testid="`provider-checkbox-${provider.id}`"
                  :model-value="isProviderChecked(provider.id)"
                  @update:model-value="(v) => toggleProvider(provider.id, v === true)"
                />
                <span class="text-sm">{{ provider.name }}</span>
              </label>
            </div>
          </AccordionContent>
        </AccordionItem>
      </Accordion>
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
          :data-testid="`model-group-${entry.provider.id}`"
          class="space-y-1"
        >
          <p class="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
            {{ entry.provider.name }}
            <code class="ml-1 normal-case">({{ entry.provider.providerKind }})</code>
          </p>
          <div class="grid grid-cols-2 gap-1 pl-2">
            <label
              v-for="model in entry.models"
              :key="model"
              :data-testid="`model-row-${entry.provider.id}-${model}`"
              :for="`model-checkbox-${entry.provider.id}-${model}`"
              class="flex items-center gap-2"
            >
              <Checkbox
                :id="`model-checkbox-${entry.provider.id}-${model}`"
                :aria-label="model"
                :data-testid="`model-checkbox-${entry.provider.id}-${model}`"
                :model-value="isModelChecked(model)"
                @update:model-value="(v) => toggleModel(model, v === true)"
              />
              <code class="text-xs font-mono">{{ model }}</code>
            </label>
          </div>
        </div>
      </div>
    </div>

    <!-- Strict spending budget -->
    <fieldset class="space-y-3 rounded-md border border-border p-4"
              data-testid="spending-budget-controls">
      <legend class="px-1 text-sm font-medium">{{ t('apiKeys.budgetTitle') }}</legend>
      <p class="text-xs text-muted-foreground">{{ t('apiKeys.budgetExplanation') }}</p>
      <label class="flex items-center gap-2 text-sm" for="spending-budget-enabled">
        <Checkbox
          id="spending-budget-enabled"
          :aria-label="t('apiKeys.budgetEnabled')"
          :model-value="modelValue.spendingBudgetEnabled"
          data-testid="checkbox-spending-budget-enabled"
          @update:model-value="(v) => setBudgetEnabled(v === true)"
        />
        {{ t('apiKeys.budgetEnabled') }}
      </label>
      <div :aria-disabled="!modelValue.spendingBudgetEnabled" class="grid gap-3 sm:grid-cols-2">
        <div class="space-y-2">
          <label class="text-sm font-medium"
                 for="spending-budget-amount">{{ t('apiKeys.budgetAmount') }}</label>
          <Input
            id="spending-budget-amount"
            :disabled="!modelValue.spendingBudgetEnabled"
            :model-value="modelValue.spendingBudgetAmountUsd"
            data-testid="input-spending-budget-amount"
            inputmode="decimal"
            min="0.000001"
            step="0.000001"
            type="number"
            @update:model-value="(v) => update('spendingBudgetAmountUsd', String(v ?? ''))"
          />
        </div>
        <div class="space-y-2">
          <label class="text-sm font-medium"
                 for="spending-budget-request-limit">{{ t('apiKeys.budgetRequestLimit') }}</label>
          <Input
            id="spending-budget-request-limit"
            :disabled="!modelValue.spendingBudgetEnabled"
            :model-value="modelValue.spendingBudgetRequestLimitUsd"
            data-testid="input-spending-budget-request-limit"
            inputmode="decimal"
            min="0.000001"
            step="0.000001"
            type="number"
            @update:model-value="(v) => update('spendingBudgetRequestLimitUsd', String(v ?? ''))"
          />
        </div>
        <div class="space-y-2 sm:col-span-2">
          <label class="text-sm font-medium"
                 for="spending-budget-window">{{ t('apiKeys.budgetWindow') }}</label>
          <NativeSelect
            id="spending-budget-window"
            :aria-label="t('apiKeys.budgetWindow')"
            :disabled="!modelValue.spendingBudgetEnabled"
            :model-value="modelValue.spendingBudgetWindow"
            data-testid="select-spending-budget-window"
            @update:model-value="(v) => update('spendingBudgetWindow', v === 'monthly' ? 'monthly' : 'daily')"
          >
            <NativeSelectOption value="daily">{{ t('apiKeys.budgetDaily') }}</NativeSelectOption>
            <NativeSelectOption value="monthly">{{
                t('apiKeys.budgetMonthly')
              }}
            </NativeSelectOption>
          </NativeSelect>
        </div>
      </div>
      <p v-if="spendingLimitExceedsBudget" class="text-sm text-destructive" role="alert">
        {{ t('apiKeys.budgetValidation') }}
      </p>
    </fieldset>

    <!-- Expires at -->
    <div class="space-y-2">
      <label class="text-sm font-medium" for="api-key-expires">Expires at (optional)</label>
      <Input
        id="api-key-expires"
        :model-value="modelValue.expiresAt ?? ''"
        data-testid="input-api-key-expires"
        type="datetime-local"
        @update:model-value="(v) => update('expiresAt', v ? String(v) : null)"
      />
    </div>

    <!-- Error display -->
    <div
      v-if="error"
      class="rounded-md border border-destructive/30 bg-destructive/5 p-3 text-sm text-destructive"
      data-testid="api-key-form-error"
    >
      <div class="flex items-start gap-2">
        <AlertTriangle class="h-4 w-4 mt-0.5"/>
        <span>{{ error }}</span>
      </div>
    </div>

    <!-- Footer -->
    <div class="flex items-center justify-end gap-2 pt-2">
      <Button :data-testid="'api-key-cancel'" type="button" variant="outline"
              @click="emit('cancel')">
        {{ cancelLabel }}
      </Button>
      <Button :data-testid="'api-key-submit'" type="submit">
        <Key class="h-4 w-4 mr-1"/>
        {{ submitLabel }}
      </Button>
    </div>
  </form>
</template>
