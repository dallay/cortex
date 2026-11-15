<script setup lang="ts">
/**
 * CompatibleConnectionsSection — special section for compatible providers
 * that displays individual connections with custom names.
 *
 * Shows:
 * - Section header with "Add OpenAI Compatible" and "Add Anthropic Compatible" buttons
 * - List of connections with display name, protocol badge, status, toggle, and test button
 * - Empty state when no connections exist
 */
import {Code, Loader2, Play, Plus } from "@lucide/vue";
import {computed, ref} from "vue";
import {useI18n} from "vue-i18n";
import {Button} from "@/components/ui/button";
import {Switch} from "@/components/ui/switch";
import type {useProviders} from "@/composables/useProviders";
import type {ProviderKind} from "@/config/providerCatalog";

const COMPATIBLE_KINDS: ProviderKind[] = ["openai-compatible", "anthropic-compatible"];

const {t} = useI18n();
type ProviderActions = ReturnType<typeof useProviders>;
const props = defineProps<{
  providers: ProviderActions["providers"]["value"];
  update: ProviderActions["update"];
  test: ProviderActions["test"];
  refresh: ProviderActions["fetch"];
}>();

const emit = defineEmits<{
  add: [kind: ProviderKind];
}>();

const testingIds = ref(new Set<string>());
const testResults = ref<Record<string, NonNullable<Awaited<ReturnType<ProviderActions["test"]>>> | null>>({});
const testingAll = ref(false);
const busyIds = ref(new Set<string>());

const compatibleConnections = computed(() =>
  props.providers.filter((p) => COMPATIBLE_KINDS.includes(p.providerKind)),
);

function isTesting(id: string) {
  return testingIds.value.has(id);
}

function isBusy(id: string) {
  return busyIds.value.has(id);
}

function getProtocolLabel(kind: string) {
  if (kind === "openai-compatible") return t("providers.compatible.protocol.openai");
  if (kind === "anthropic-compatible") return t("providers.compatible.protocol.anthropic");
  return kind;
}

async function handleTest(id: string) {
  if (isTesting(id)) return;
  testingIds.value = new Set([...testingIds.value, id]);
  try {
    const result = await props.test(id);
    testResults.value[id] = result;
    if (result === null) return;
    await props.refresh();
  } finally {
    const next = new Set(testingIds.value);
    next.delete(id);
    testingIds.value = next;
  }
}

async function handleTestAll() {
  if (testingAll.value) return;
  testingAll.value = true;
  try {
    for (const conn of compatibleConnections.value) {
      await handleTest(conn.id);
    }
  } finally {
    testingAll.value = false;
  }
}

async function handleToggle({id, enabled}: {id: string; enabled: boolean}) {
  const conn = props.providers.find((p) => p.id === id);
  if (!conn) return;
  busyIds.value = new Set([...busyIds.value, id]);
  try {
    await props.update(id, {expectedUpdatedAt: conn.updatedAt, isActive: enabled});
  } finally {
    const next = new Set(busyIds.value);
    next.delete(id);
    busyIds.value = next;
  }
}

</script>

<template>
  <section :data-testid="'catalog-section-compatible'" class="space-y-3">
    <div class="flex items-start justify-between gap-3">
      <div>
        <h2 class="text-lg font-semibold tracking-tight">
          {{ t('providers.category.compatible.name') }}
        </h2>
        <p class="text-sm text-muted-foreground">
          {{ t('providers.category.compatible.description') }}
        </p>
      </div>
      <div class="flex items-center gap-2 shrink-0">
        <Button
          variant="outline"
          size="sm"
          :data-testid="'test-all-compatible'"
          :disabled="testingAll || testingIds.size > 0 || compatibleConnections.length === 0"
          @click="handleTestAll"
        >
          <Play class="h-4 w-4 mr-1" />
          {{ t('providers.compatible.testAll') }}
        </Button>
        <Button
          variant="default"
          size="sm"
          :data-testid="'add-anthropic-compatible'"
          @click="emit('add', 'anthropic-compatible')"
        >
          <Plus class="h-4 w-4 mr-1" />
          {{ t('providers.compatible.addAnthropic') }}
        </Button>
        <Button
          variant="default"
          size="sm"
          :data-testid="'add-openai-compatible'"
          @click="emit('add', 'openai-compatible')"
        >
          <Plus class="h-4 w-4 mr-1" />
          {{ t('providers.compatible.addOpenAi') }}
        </Button>
      </div>
    </div>

    <div v-if="compatibleConnections.length > 0" class="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
      <div
        v-for="conn in compatibleConnections"
        :key="conn.id"
        :data-testid="'compatible-connection-card'"
        class="rounded-lg border bg-card p-4 space-y-3"
      >
        <div class="flex items-start justify-between">
          <div class="flex items-start gap-3 min-w-0">
            <div class="rounded-lg bg-primary/10 p-2 shrink-0">
              <Code class="h-5 w-5 text-primary" />
            </div>
            <div class="min-w-0">
              <h3 class="text-sm font-semibold truncate">{{ conn.name || t('providers.compatible.untitled') }}</h3>
              <div class="flex items-center gap-2 mt-1">
                <span class="inline-flex items-center rounded-md bg-secondary px-2 py-0.5 text-xs font-medium">
                  {{ getProtocolLabel(conn.providerKind) }}
                </span>
              </div>
            </div>
          </div>
          <div
            v-if="conn.isActive"
            class="h-2 w-2 rounded-full bg-green-500 shrink-0 mt-1"
            :title="t('providers.catalog.active')"
          />
        </div>

        <p v-if="conn.id in testResults" :role="testResults[conn.id]?.valid ? 'status' : 'alert'" class="text-xs">
          {{ testResults[conn.id]?.valid ? t('providers.testSuccess') : (testResults[conn.id]?.error || t('providers.testFailed')) }}
        </p>

        <div class="flex items-center justify-between">
          <span class="text-xs text-muted-foreground">
            {{ conn.isActive ? t('providers.compatible.connected') : t('providers.compatible.disabled') }}
          </span>
          <div class="flex items-center gap-2">
            <Switch
              :model-value="conn.isActive"
              :disabled="isBusy(conn.id)"
              @update:model-value="(val: boolean) => handleToggle({ id: conn.id, enabled: val })"
            />
            <Button
              variant="outline"
              size="sm"
              :disabled="testingAll || isTesting(conn.id)"
              @click="handleTest(conn.id)"
            >
              <Loader2 v-if="isTesting(conn.id)" class="h-3 w-3 animate-spin" />
              <Play v-else class="h-3 w-3" />
              {{ t('providers.compatible.test') }}
            </Button>
          </div>
        </div>
      </div>
    </div>

    <div v-else class="rounded-lg border-2 border-dashed border-muted-foreground/25 p-8 text-center">
      <p class="text-sm text-muted-foreground">
        {{ t('providers.compatible.noConnections') }}
      </p>
    </div>
  </section>
</template>

