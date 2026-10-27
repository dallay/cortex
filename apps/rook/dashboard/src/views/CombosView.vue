<script setup lang="ts">
import {CircleDot, Pencil, Plus, Trash2, X} from "@lucide/vue";
import {computed, onMounted, ref, watch} from "vue";
import {useI18n} from "vue-i18n";
import EmptyState from "@/components/EmptyState.vue";
import ErrorBanner from "@/components/ErrorBanner.vue";
import LoadingState from "@/components/LoadingState.vue";
import PageHeader from "@/components/PageHeader.vue";
import {Button} from "@/components/ui/button";
import {Input} from "@/components/ui/input";
import {NativeSelect, NativeSelectOption} from "@/components/ui/native-select";
import {useAvailableModels} from "@/composables/useAvailableModels";
import {useCombos} from "@/composables/useCombos";
import type {ComboResponse} from "@/lib/api";
import {normalizeComboAlias, validateComboAlias} from "@/lib/comboAliasValidation";
import {
  COMBO_MAX_STEPS,
  COMBO_NAME_MAX_LENGTH,
  type ComboDraft,
  type ComboValidationIssue,
  emptyComboDraft,
  emptyComboStep,
  toCreateComboRequest,
  validateComboDraft,
} from "@/lib/comboValidation";

const {t} = useI18n();

const {
  combos,
  aliasesByComboId,
  aliasError,
  loading,
  saving,
  error,
  fetch,
  create,
  update,
  remove,
  fetchAliases,
  createAlias,
  removeAlias,
} = useCombos();
const {
  modelsByProvider,
  fetch: fetchModels,
  fetchProviders,
} = useAvailableModels();

const showForm = ref(false);
const editingId = ref<string | null>(null);
const draft = ref<ComboDraft>(emptyComboDraft());
const issues = ref<ComboValidationIssue[]>([]);
const submitError = ref<string | null>(null);
const aliasDrafts = ref<Record<string, string>>({});
const aliasIssues = ref<Record<string, string | null>>({});
const aliasOperationErrors = ref<Record<string, string | null>>({});

// Only active providers with a non-empty model catalog can feed a step.
const providerOptions = computed(() =>
  modelsByProvider.value.map((entry) => ({
    id: entry.provider.id,
    name: entry.provider.name,
    models: entry.models,
  })),
);

const hasProviders = computed(() => providerOptions.value.length > 0);
const canAddStep = computed(() => draft.value.steps.length < COMBO_MAX_STEPS);

const nameIssue = computed(() =>
  issues.value.find(
    (issue) =>
      issue.code === "name_required" || issue.code === "name_too_long",
  ),
);

function providerLabel(providerId: string): string {
  return (
    providerOptions.value.find((entry) => entry.id === providerId)?.name ??
    providerId
  );
}

function modelsFor(providerId: string): string[] {
  return (
    providerOptions.value.find((entry) => entry.id === providerId)?.models ?? []
  );
}

function sortedSteps(combo: ComboResponse) {
  return [...combo.steps].sort((a, b) => a.priority - b.priority);
}

function firstStepIssue(index: number): ComboValidationIssue | undefined {
  return issues.value.find((issue) => issue.stepIndex === index);
}

function issueMessage(issue?: ComboValidationIssue): string {
  if (!issue) return "";
  return t(`combos.errors.${issue.code}`, {priority: issue.priority ?? ""});
}

/**
 * Smallest priority that is not used yet, so adding a step never produces a
 * duplicate that the backend would reject.
 */
function nextPriority(): number {
  const used = new Set(draft.value.steps.map((step) => step.priority));
  for (let priority = 1; priority <= 255; priority += 1) {
    if (!used.has(priority)) return priority;
  }
  return 255;
}

function resetForm() {
  showForm.value = false;
  editingId.value = null;
  draft.value = emptyComboDraft();
  issues.value = [];
  submitError.value = null;
}

function startCreate() {
  resetForm();
  showForm.value = true;
}

function startEdit(combo: ComboResponse) {
  editingId.value = combo.id;
  draft.value = {
    name: combo.name,
    steps: combo.steps.map((step) => ({...step})),
  };
  issues.value = [];
  submitError.value = null;
  showForm.value = true;
}

function addStep() {
  if (!canAddStep.value) return;
  draft.value.steps.push(emptyComboStep(nextPriority()));
}

function removeStep(index: number) {
  draft.value.steps.splice(index, 1);
}

function onProviderChange(index: number, value: unknown) {
  const step = draft.value.steps[index];
  if (!step) return;
  step.providerId = String(value ?? "");
  step.model = modelsFor(step.providerId)[0] ?? "";
}

function onPriorityChange(index: number, value: unknown) {
  const step = draft.value.steps[index];
  if (!step) return;
  const parsed = Number(value);
  step.priority = Number.isFinite(parsed) ? parsed : Number.NaN;
}

async function submit() {
  issues.value = validateComboDraft(draft.value);
  submitError.value = null;
  if (issues.value.length > 0) return;

  const payload = toCreateComboRequest(draft.value);
  const result = editingId.value
    ? await update(editingId.value, payload)
    : await create(payload);

  if (result) {
    resetForm();
  } else {
    submitError.value = error.value ?? t("combos.saveFailed");
  }
}

async function confirmDelete(combo: ComboResponse) {
  if (!globalThis.confirm(t("combos.deleteConfirm", {name: combo.name}))) {
    return;
  }
  await remove(combo.id);
}

async function addComboAlias(combo: ComboResponse) {
  const value = aliasDrafts.value[combo.id] ?? "";
  const validation = validateComboAlias(value);
  if (validation.length > 0) {
    aliasIssues.value[combo.id] = validation[0]?.code ?? "alias_format_invalid";
    return;
  }

  aliasIssues.value[combo.id] = null;
  aliasOperationErrors.value[combo.id] = null;
  const alias = normalizeComboAlias(value);
  const created = await createAlias(combo.id, alias);
  if (created) {
    aliasDrafts.value[combo.id] = "";
  } else {
    aliasOperationErrors.value[combo.id] =
      aliasError.value[combo.id] ?? t("combos.aliasBackendError");
  }
}

async function confirmDeleteAlias(combo: ComboResponse, alias: string) {
  if (!globalThis.confirm(t("combos.aliasDeleteConfirm", {alias}))) return;
  aliasOperationErrors.value[combo.id] = null;
  const removed = await removeAlias(combo.id, alias);
  if (!removed) {
    aliasOperationErrors.value[combo.id] =
      aliasError.value[combo.id] ?? t("combos.aliasBackendError");
  }
}

async function loadAll() {
  await Promise.all([fetch(), fetchProviders(), fetchModels()]);
}

async function loadComboAliases(comboId: string) {
  await fetchAliases(comboId);
}

watch(
  combos,
  async (items) => {
    await Promise.all(items.map((combo) => loadComboAliases(combo.id)));
  },
  {immediate: true},
);

onMounted(loadAll);
</script>

<template>
  <div class="space-y-6">
    <PageHeader
      :title="t('nav.combos')"
      :description="t('combos.description')"
    >
      <Button
        data-testid="combos-create"
        type="button"
        :disabled="!hasProviders"
        @click="startCreate"
      >
        <Plus class="mr-2 h-4 w-4" />
        {{ t("combos.create") }}
      </Button>
    </PageHeader>

    <p v-if="!hasProviders" class="text-sm text-muted-foreground">
      {{ t("combos.noProviders") }}
    </p>

    <ErrorBanner v-if="error && !showForm" :message="error">
      <Button
        data-testid="combos-retry"
        type="button"
        variant="outline"
        size="sm"
        @click="loadAll"
      >
        {{ t("combos.retry") }}
      </Button>
    </ErrorBanner>

    <LoadingState
      v-if="loading && combos.length === 0"
      :message="t('common.loading')"
    />

    <EmptyState
      v-else-if="combos.length === 0 && !showForm"
      :title="t('combos.empty')"
      :description="t('combos.emptyDescription')"
      :icon="CircleDot"
    />

    <div v-else class="space-y-4">
      <article
        v-for="combo in combos"
        :key="combo.id"
        data-testid="combo-card"
        class="rounded-lg border bg-card p-4"
      >
        <div class="flex flex-wrap items-start justify-between gap-3">
          <div class="space-y-1">
            <h2 class="text-lg font-semibold">{{ combo.name }}</h2>
            <p class="text-xs text-muted-foreground">
              {{ t("combos.strategy") }}: {{ combo.strategy }}
            </p>
          </div>
          <div class="flex gap-2">
            <Button
              :data-testid="`combo-edit-${combo.id}`"
              type="button"
              variant="outline"
              size="sm"
              @click="startEdit(combo)"
            >
              <Pencil class="mr-1 h-3.5 w-3.5" />
              {{ t("common.edit") }}
            </Button>
            <Button
              :data-testid="`combo-delete-${combo.id}`"
              type="button"
              variant="destructive"
              size="sm"
              @click="confirmDelete(combo)"
            >
              <Trash2 class="mr-1 h-3.5 w-3.5" />
              {{ t("common.delete") }}
            </Button>
          </div>
        </div>

        <ol class="mt-4 space-y-2">
          <li
            v-for="step in sortedSteps(combo)"
            :key="step.priority"
            class="flex items-center gap-3 rounded-md border px-3 py-2 text-sm"
          >
            <span
              class="inline-flex h-6 w-6 items-center justify-center rounded-full bg-muted text-xs font-medium"
            >
              {{ step.priority }}
            </span>
            <span class="font-medium">{{ providerLabel(step.providerId) }}</span>
            <span class="text-muted-foreground">{{ step.model }}</span>
          </li>
        </ol>

        <section
          class="mt-4 space-y-3 border-t pt-4"
          :aria-label="t('combos.aliases')"
        >
          <h3 class="text-sm font-semibold">{{ t("combos.aliases") }}</h3>

          <ErrorBanner
            v-if="aliasOperationErrors[combo.id]"
            :message="aliasOperationErrors[combo.id] ?? ''"
          />
          <ErrorBanner
            v-if="aliasError[combo.id] && !aliasOperationErrors[combo.id]"
            :message="aliasError[combo.id] ?? t('combos.aliasLoadFailed')"
          />

          <ul
            v-if="aliasesByComboId[combo.id]?.length"
            class="space-y-2"
          >
            <li
              v-for="item in aliasesByComboId[combo.id]"
              :key="item.alias"
              class="flex items-center justify-between gap-2 rounded-md bg-muted px-3 py-2 text-sm"
            >
              <code>{{ item.alias }}</code>
              <Button
                :data-testid="`combo-alias-delete-${combo.id}-${item.alias.replace('/', '-')}`"
                type="button"
                variant="ghost"
                size="sm"
                :aria-label="`${t('combos.aliasDelete')}: ${item.alias}`"
                @click="confirmDeleteAlias(combo, item.alias)"
              >
                <Trash2 class="mr-1 h-3.5 w-3.5" />
                {{ t("combos.aliasDelete") }}
              </Button>
            </li>
          </ul>

          <form
            :data-testid="`combo-alias-form-${combo.id}`"
            class="flex flex-col gap-2 sm:flex-row"
            @submit.prevent="addComboAlias(combo)"
          >
            <div class="flex-1">
              <label class="sr-only" :for="`combo-alias-input-${combo.id}`">
                {{ t("combos.aliasPlaceholder") }}
              </label>
              <Input
                :id="`combo-alias-input-${combo.id}`"
                :data-testid="`combo-alias-input-${combo.id}`"
                :model-value="aliasDrafts[combo.id] ?? ''"
                :placeholder="t('combos.aliasPlaceholder')"
                autocomplete="off"
                @update:model-value="(value) => (aliasDrafts[combo.id] = String(value))"
              />
              <p
                v-if="aliasIssues[combo.id]"
                class="mt-1 text-xs text-destructive"
              >
                {{ t(`combos.aliasErrors.${aliasIssues[combo.id]}`) }}
              </p>
            </div>
            <Button
              :data-testid="`combo-alias-add-${combo.id}`"
              type="submit"
              variant="outline"
              size="sm"
            >
              <Plus class="mr-1 h-3.5 w-3.5" />
              {{ t("combos.aliasAdd") }}
            </Button>
          </form>
        </section>
      </article>
    </div>

    <form
      v-if="showForm"
      data-testid="combo-form"
      class="space-y-5 rounded-lg border bg-card p-4"
      @submit.prevent="submit"
    >
      <div class="flex items-center justify-between">
        <h2 class="text-lg font-semibold">
          {{ editingId ? t("combos.editTitle") : t("combos.createNew") }}
        </h2>
        <Button
          data-testid="combo-cancel"
          type="button"
          variant="ghost"
          size="sm"
          @click="resetForm"
        >
          <X class="mr-1 h-4 w-4" />
          {{ t("common.cancel") }}
        </Button>
      </div>

      <div class="space-y-1">
        <label class="text-sm font-medium" for="combo-name">
          {{ t("combos.name") }}
        </label>
        <Input
          id="combo-name"
          data-testid="combo-name"
          :model-value="draft.name"
          :maxlength="COMBO_NAME_MAX_LENGTH"
          :placeholder="t('combos.namePlaceholder')"
          @update:model-value="(value) => (draft.name = String(value))"
        />
        <p
          v-if="nameIssue"
          data-testid="combo-name-error"
          class="text-sm text-destructive"
        >
          {{ issueMessage(nameIssue) }}
        </p>
      </div>

      <p class="text-sm text-muted-foreground">
        {{ t("combos.strategy") }}:
        <span class="font-medium">priority</span> —
        {{ t("combos.strategyPriority") }}
      </p>

      <fieldset class="space-y-3">
        <legend class="text-sm font-medium">{{ t("combos.steps") }}</legend>

        <div
          v-for="(step, index) in draft.steps"
          :key="index"
          class="grid gap-3 rounded-md border p-3 sm:grid-cols-[1fr_1fr_7rem_auto]"
        >
          <div class="space-y-1">
            <label
              class="text-xs font-medium"
              :for="`combo-step-provider-${index}`"
            >
              {{ t("combos.provider") }}
            </label>
            <NativeSelect
              :id="`combo-step-provider-${index}`"
              :data-testid="`combo-step-provider-${index}`"
              :model-value="step.providerId"
              @update:model-value="(value) => onProviderChange(index, value)"
            >
              <NativeSelectOption value="">
                {{ t("combos.providerPlaceholder") }}
              </NativeSelectOption>
              <NativeSelectOption
                v-for="option in providerOptions"
                :key="option.id"
                :value="option.id"
              >
                {{ option.name }}
              </NativeSelectOption>
            </NativeSelect>
          </div>

          <div class="space-y-1">
            <label
              class="text-xs font-medium"
              :for="`combo-step-model-${index}`"
            >
              {{ t("combos.model") }}
            </label>
            <NativeSelect
              :id="`combo-step-model-${index}`"
              :data-testid="`combo-step-model-${index}`"
              :model-value="step.model"
              :disabled="modelsFor(step.providerId).length === 0"
              @update:model-value="(value) => (step.model = String(value))"
            >
              <NativeSelectOption value="">
                {{ t("combos.modelPlaceholder") }}
              </NativeSelectOption>
              <NativeSelectOption
                v-for="model in modelsFor(step.providerId)"
                :key="model"
                :value="model"
              >
                {{ model }}
              </NativeSelectOption>
            </NativeSelect>
          </div>

          <div class="space-y-1">
            <label
              class="text-xs font-medium"
              :for="`combo-step-priority-${index}`"
            >
              {{ t("combos.priority") }}
            </label>
            <Input
              :id="`combo-step-priority-${index}`"
              :data-testid="`combo-step-priority-${index}`"
              type="number"
              min="1"
              max="255"
              step="1"
              :model-value="step.priority"
              @update:model-value="(value) => onPriorityChange(index, value)"
            />
          </div>

          <div class="flex items-end">
            <Button
              :data-testid="`combo-step-remove-${index}`"
              type="button"
              variant="ghost"
              size="icon"
              :disabled="draft.steps.length === 1"
              :aria-label="t('combos.removeStep')"
              @click="removeStep(index)"
            >
              <Trash2 class="h-4 w-4" />
            </Button>
          </div>

          <p
            v-if="firstStepIssue(index)"
            class="text-xs text-destructive sm:col-span-4"
          >
            {{ t("combos.stepNumber", {index: index + 1}) }}:
            {{ issueMessage(firstStepIssue(index)) }}
          </p>
        </div>

        <Button
          data-testid="combo-step-add"
          type="button"
          variant="outline"
          size="sm"
          :disabled="!canAddStep"
          @click="addStep"
        >
          <Plus class="mr-1 h-3.5 w-3.5" />
          {{ t("combos.addStep") }}
        </Button>
      </fieldset>

      <ErrorBanner v-if="submitError" :message="submitError" />

      <div class="flex justify-end gap-2">
        <Button type="button" variant="outline" @click="resetForm">
          {{ t("common.cancel") }}
        </Button>
        <Button
          data-testid="combo-submit"
          type="submit"
          :disabled="saving"
        >
          {{ saving ? t("common.loading") : t("combos.save") }}
        </Button>
      </div>
    </form>
  </div>
</template>
