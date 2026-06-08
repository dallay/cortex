<script setup lang="ts">
/**
 * ProviderDetailsView — per-kind connection management screen.
 *
 * Reached via `/providers/:providerKind`. Lists every connection of
 * the given provider kind, with the same CRUD + test affordances
 * previously available on the legacy `ProvidersView` table, scoped to
 * this provider kind only.
 *
 * Connections are joined with `useAvailableModels().modelsByProvider`
 * to surface each connection's configured model list inline.
 *
 * If the `providerKind` route param does not match a known
 * `ProviderKind`, the view redirects to the catalog at `/providers`.
 */

import {ArrowLeft, ExternalLink, Plus} from "@lucide/vue";
import {computed, onMounted, ref, watch} from "vue";
import {useI18n} from "vue-i18n";
import {useRoute, useRouter} from "vue-router";
import AddProviderDialog from "@/components/AddProviderDialog.vue";
import ConnectionListItem from "@/components/ConnectionListItem.vue";
import EmptyState from "@/components/EmptyState.vue";
import ErrorBanner from "@/components/ErrorBanner.vue";
import LoadingState from "@/components/LoadingState.vue";
import ProviderIcon from "@/components/ProviderIcon.vue";
import {Button} from "@/components/ui/button";
import {useAvailableModels} from "@/composables/useAvailableModels";
import {useProviderCatalog} from "@/composables/useProviderCatalog";
import {useProviders} from "@/composables/useProviders";
import {
  findCatalogEntry,
  PROVIDER_KINDS,
  type ProviderKind,
} from "@/config/providerCatalog";

const route = useRoute();
const router = useRouter();
const {t} = useI18n();

const validKinds = new Set<string>(PROVIDER_KINDS.map((p) => p.kind));

const providerKindParam = computed<ProviderKind | null>(() => {
  const param = route.params.providerKind;
  if (typeof param === "string" && validKinds.has(param)) {
    return param as ProviderKind;
  }
  return null;
});

watch(
  providerKindParam,
  (value) => {
    if (value === null) {
      router.replace("/providers");
    }
  },
  {immediate: true},
);

const entry = computed(() => {
  const kind = providerKindParam.value;
  return kind ? findCatalogEntry(kind) : null;
});

const providerName = computed(() =>
  entry.value ? t(entry.value.displayNameKey) : "",
);

const {providers, loading, error, fetch, test, update, remove} =
  useProviders();
const {modelsByProvider, fetch: fetchModels} = useAvailableModels();
const {byKind} = useProviderCatalog();

onMounted(() => {
  fetch();
  fetchModels();
});

const connectionIdsForKind = computed(() => {
  const kind = providerKindParam.value;
  if (!kind) return new Set<string>();
  return new Set(
    providers.value.filter((p) => p.providerKind === kind).map((p) => p.id),
  );
});

const connections = computed(() =>
  providers.value.filter((p) => connectionIdsForKind.value.has(p.id)),
);

const modelsByConnection = computed(() => {
  const map = new Map<string, string[]>();
  for (const entry of modelsByProvider.value) {
    if (connectionIdsForKind.value.has(entry.provider.id)) {
      map.set(entry.provider.id, entry.models);
    }
  }
  return map;
});

const testingIds = ref(new Set<string>());
const busyIds = ref(new Set<string>());

function isTesting(id: string) {
  return testingIds.value.has(id);
}

function isBusy(id: string) {
  return busyIds.value.has(id);
}

async function handleTest(id: string) {
  testingIds.value = new Set([...testingIds.value, id]);
  try {
    const result = await test(id);
    if (result) {
      if (result.valid) {
        console.info(t("providers.details.testSuccess"));
      } else {
        console.warn(
          t("providers.details.testFailure", {
            error: result.error ?? "unknown",
          }),
        );
      }
    }
    await fetch();
  } finally {
    const next = new Set(testingIds.value);
    next.delete(id);
    testingIds.value = next;
  }
}

async function handleToggle({ id, enabled }: { id: string; enabled: boolean }) {
  const conn = providers.value.find((p) => p.id === id);
  if (!conn) return;
  busyIds.value = new Set([...busyIds.value, id]);
  try {
    await update(id, {expectedUpdatedAt: conn.updatedAt, isActive: enabled});
  } finally {
    const next = new Set(busyIds.value);
    next.delete(id);
    busyIds.value = next;
  }
}

async function handleDelete(id: string) {
  if (!window.confirm(t("providers.details.deleteConfirm"))) return;
  busyIds.value = new Set([...busyIds.value, id]);
  try {
    await remove(id);
  } finally {
    const next = new Set(busyIds.value);
    next.delete(id);
    busyIds.value = next;
  }
}

// AddProviderDialog state
const addDialogOpen = ref(false);
const editDialogOpen = ref(false);
const editingId = ref<string | null>(null);

function openAddDialog() {
  addDialogOpen.value = true;
}

function openEditDialog(id: string) {
  editingId.value = id;
  editDialogOpen.value = true;
}

function onDialogSaved() {
  addDialogOpen.value = false;
  editDialogOpen.value = false;
  editingId.value = null;
  fetch();
}

function onDialogDeleted() {
  addDialogOpen.value = false;
  editDialogOpen.value = false;
  editingId.value = null;
  fetch();
}

// Suppress unused byKind reference (kept for future per-kind stats).
void byKind;
</script>

<template>
  <div v-if="entry" class="space-y-6">
    <div class="flex items-center gap-2">
      <Button
        variant="ghost"
        size="sm"
        :data-testid="'back-to-catalog'"
        @click="router.push('/providers')"
      >
        <ArrowLeft class="h-4 w-4 mr-1" />
        {{ t('providers.details.backToCatalog') }}
      </Button>
    </div>

    <div class="flex items-start justify-between gap-3">
      <div>
        <!-- Title: link to the provider's official site when brandUrl is set -->
        <a
          v-if="entry.brandUrl"
          :href="entry.brandUrl"
          target="_blank"
          rel="noopener noreferrer"
          :aria-label="`${providerName} — opens in new tab`"
          class="inline-flex items-center gap-1.5 text-2xl font-semibold tracking-tight hover:underline focus-visible:underline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary rounded-sm"
        >
          <ProviderIcon
            :kind="entry.kind"
            loading="eager"
            :width="28"
            :height="28"
            :decorative="false"
          />
          <span>{{ providerName }}</span>
          <ExternalLink :size="16" class="opacity-60 shrink-0" aria-hidden="true"/>
        </a>
        <h1
          v-else
          class="text-2xl font-semibold tracking-tight inline-flex items-center gap-1.5"
        >
          <ProviderIcon
            :kind="entry.kind"
            loading="eager"
            :width="28"
            :height="28"
            :decorative="false"
          />
          <span>{{ providerName }}</span>
        </h1>
        <p class="text-muted-foreground mt-1">
          {{ t('providers.details.subtitle', { providerName }) }}
        </p>
      </div>
      <Button :data-testid="'add-connection'" @click="openAddDialog">
        <Plus class="h-4 w-4 mr-1" />
        {{ t('providers.details.addConnection') }}
      </Button>
    </div>

    <ErrorBanner v-if="error" :message="error" @retry="fetch" />

    <LoadingState v-if="loading && connections.length === 0" />

    <div v-else-if="connections.length > 0" class="space-y-3">
      <ConnectionListItem
        v-for="conn in connections"
        :key="conn.id"
        :connection="conn"
        :models="modelsByConnection.get(conn.id)"
        :testing="isTesting(conn.id)"
        :busy="isBusy(conn.id)"
        @test="handleTest"
        @edit="openEditDialog"
        @delete="handleDelete"
        @toggle="handleToggle"
      />
    </div>

    <EmptyState
      v-else
      :title="t('providers.details.noConnections', { providerName })"
      :data-testid="'provider-empty-state'"
    >
      <template #actions>
        <Button @click="openAddDialog">
          <Plus class="h-4 w-4 mr-1" />
          {{ t('providers.details.addConnection') }}
        </Button>
      </template>
    </EmptyState>

    <AddProviderDialog
      v-model:open="addDialogOpen"
      :provider-kind="providerKindParam ?? undefined"
      mode="create"
      @saved="onDialogSaved"
      @deleted="onDialogDeleted"
    />

    <AddProviderDialog
      :open="editDialogOpen"
      :connection-id="editingId ?? undefined"
      :provider-kind="providerKindParam ?? undefined"
      mode="edit"
      @update:open="editDialogOpen = $event"
      @saved="onDialogSaved"
      @deleted="onDialogDeleted"
    />
  </div>
</template>
