<script setup lang="ts">
import { computed, onScopeDispose, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import ErrorBanner from "@/components/ErrorBanner.vue";
import LoadingState from "@/components/LoadingState.vue";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { type CodexAccountsResponse, useApi } from "@/lib/api";
import { isLoopbackBrowser } from "@/lib/browserHost";

const open = defineModel<boolean>("open", { required: true });
const { t } = useI18n();
const api = useApi();
const snapshot = ref<CodexAccountsResponse>();
const login = ref<{ id: string; authUrl: string }>();
const busy = ref(false);
const error = ref(false);
const browserIsLocal = isLoopbackBrowser();
const pending = computed(() => snapshot.value?.attempt?.status === "pending" ? snapshot.value.attempt : undefined);
const link = computed(() => pending.value?.id === login.value?.id ? login.value?.authUrl : undefined);
let timer: ReturnType<typeof setTimeout> | undefined;
let activeRequest: AbortController | undefined;
let dialogGeneration = 0;

function stopRefresh() {
  clearTimeout(timer);
  activeRequest?.abort();
}

async function refresh() {
  stopRefresh();
  const controller = new AbortController();
  activeRequest = controller;
  try {
    const result = await api.getCodexAccounts(controller.signal);
    if (controller.signal.aborted) return;
    snapshot.value = result;
    if (result.attempt?.status !== "pending") login.value = undefined;
    if (result.attempt?.status === "pending") timer = setTimeout(refresh, 2000);
  } catch {
    if (!controller.signal.aborted) error.value = true;
  }
}

async function mutate(action: () => Promise<void>) {
  if (busy.value) return;
  busy.value = true;
  error.value = false;
  stopRefresh();
  try { await action(); }
  catch { error.value = true; }
  finally {
    busy.value = false;
    if (open.value) await refresh();
  }
}

function start() {
  const generation = dialogGeneration;
  void mutate(async () => {
    const result = await api.startCodexLogin();
    const url = new URL(result.authUrl);
    if (url.origin !== "https://auth.openai.com" || url.pathname !== "/oauth/authorize" || url.username || url.password) {
      await api.cancelCodexLogin(result.id);
      throw new Error("Invalid Codex authorization URL");
    }
    if (open.value && generation === dialogGeneration) login.value = { id: result.id, authUrl: url.href };
  });
}

function cancel() {
  const id = pending.value?.id;
  if (id) void mutate(() => api.cancelCodexLogin(id));
}

function retry() {
  error.value = false;
  void refresh();
}

watch(open, (value) => {
  if (value) retry();
  else {
    dialogGeneration++;
    login.value = undefined;
    stopRefresh();
  }
}, { immediate: true });
onScopeDispose(() => { dialogGeneration++; stopRefresh(); });
</script>

<template>
  <Dialog v-model:open="open">
    <DialogContent>
      <DialogHeader>
        <DialogTitle>{{ t('providers.codex.title') }}</DialogTitle>
        <DialogDescription>{{ t('providers.codex.description') }}</DialogDescription>
      </DialogHeader>
      <ErrorBanner v-if="error" :message="t('providers.codex.error')" @retry="retry" />
      <LoadingState v-if="!snapshot && !error" />
      <template v-if="snapshot">
        <p v-if="!snapshot.enabled" class="text-sm text-muted-foreground">
          {{ t('providers.codex.disabled') }}
        </p>
        <p v-else-if="!browserIsLocal" class="text-sm text-muted-foreground">
          {{ t('providers.codex.remoteBrowser') }}
        </p>
        <p v-if="snapshot.attempt" role="status" class="text-sm">
          {{ t(`providers.codex.status.${snapshot.attempt.status}`) }}
        </p>
        <ul v-if="snapshot.accounts.length" :aria-label="t('providers.codex.accounts')" class="space-y-2">
          <li v-for="account in snapshot.accounts" :key="account.id" class="rounded-md border p-3 text-sm">
            <p class="break-all font-medium">{{ t('providers.codex.accountId', {id: account.id}) }}</p>
          </li>
        </ul>
        <p v-else class="text-sm text-muted-foreground">{{ t('providers.codex.empty') }}</p>
        <div class="flex flex-wrap gap-2">
          <Button v-if="snapshot.enabled && browserIsLocal && !pending" type="button" :disabled="busy" @click="start">
            {{ t('providers.codex.start') }}
          </Button>
          <Button v-if="link && browserIsLocal" as-child>
            <a :href="link" target="_blank" rel="noopener noreferrer">{{ t('providers.codex.continue') }}</a>
          </Button>
          <Button v-if="pending" type="button" variant="outline" :disabled="busy" @click="cancel">
            {{ t('common.cancel') }}
          </Button>
        </div>
      </template>
    </DialogContent>
  </Dialog>
</template>
