<script setup lang="ts">
import { computed, onScopeDispose, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import ErrorBanner from "@/components/ErrorBanner.vue";
import LoadingState from "@/components/LoadingState.vue";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { type CodexAccountsResponse, useApi } from "@/lib/api";
import { isLoopbackBrowser } from "@/lib/browserHost";

const open = defineModel<boolean>("open", { required: true });
const { t } = useI18n();
const api = useApi();
const snapshot = ref<CodexAccountsResponse>();
const login = ref<{ id: string; authUrl: string }>();
const activeAttemptId = ref<string | null>(null);
const busy = ref(false);
const error = ref(false);
const disconnectDialogOpen = ref(false);
const disconnectingAccount = ref<string | null>(null);
const browserIsLocal = isLoopbackBrowser();
const pending = computed(() => snapshot.value?.attempt?.status === "pending" ? snapshot.value.attempt : undefined);
const link = computed(() => pending.value?.id === login.value?.id ? login.value?.authUrl : undefined);
let timer: ReturnType<typeof setTimeout> | undefined;
let activeRequest: AbortController | undefined;
let dialogGeneration = 0;
let disposed = false;

const emit = defineEmits<{
  linked: [snapshot: CodexAccountsResponse];
}>();

function stopRefresh() {
  clearTimeout(timer);
  activeRequest?.abort();
}

async function refresh() {
  if (disposed || !open.value) return;
  stopRefresh();
  const controller = new AbortController();
  activeRequest = controller;
  try {
    const result = await api.getCodexAccounts(controller.signal);
    if (controller.signal.aborted || disposed) return;
    snapshot.value = result;
    const attempt = result.attempt;
    if (attempt?.id === activeAttemptId.value && attempt.status === "linked") {
      activeAttemptId.value = null;
      login.value = undefined;
      emit("linked", result);
      open.value = false;
      return;
    }
    if (attempt?.id === activeAttemptId.value && attempt.status !== "pending") {
      activeAttemptId.value = null;
      login.value = undefined;
    }
    if (result.attempt?.status === "pending") timer = setTimeout(refresh, 2000);
  } catch {
    if (!controller.signal.aborted && !disposed) error.value = true;
  }
}

async function mutate(action: () => Promise<void>): Promise<boolean> {
  if (busy.value || disposed) return false;
  busy.value = true;
  error.value = false;
  stopRefresh();
  let succeeded = false;
  try {
    await action();
    succeeded = true;
  }
  catch {
    if (!disposed) error.value = true;
  }
  finally {
    if (!disposed) busy.value = false;
    if (open.value && !disposed) await refresh();
  }
  return succeeded;
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
    if (open.value && generation === dialogGeneration) {
      activeAttemptId.value = result.id;
      login.value = { id: result.id, authUrl: url.href };
    }
  });
}

function cancel() {
  const id = pending.value?.id;
  if (id) void mutate(() => api.cancelCodexLogin(id));
}

function retry() {
  if (disposed) return;
  error.value = false;
  void refresh();
}

function requestDisconnect(id: string) {
  disconnectingAccount.value = id;
  disconnectDialogOpen.value = true;
}

function setDisconnectDialogOpen(value: boolean) {
  disconnectDialogOpen.value = value;
  if (!value) disconnectingAccount.value = null;
}

async function confirmDisconnect() {
  const id = disconnectingAccount.value;
  if (id && await mutate(() => api.disconnectCodexAccount(id))) {
    setDisconnectDialogOpen(false);
  }
}

watch(open, (value) => {
  if (value) retry();
  else {
    dialogGeneration++;
    login.value = undefined;
    activeAttemptId.value = null;
    stopRefresh();
  }
}, { immediate: true });
onScopeDispose(() => { disposed = true; dialogGeneration++; stopRefresh(); });
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
            <div class="flex flex-wrap items-center justify-between gap-2">
              <p class="break-all font-medium">{{ t('providers.codex.accountId', {id: account.id}) }}</p>
              <Button :data-testid="`codex-disconnect-${account.id}`" type="button" variant="destructive" size="sm" :disabled="busy" @click="requestDisconnect(account.id)">
                {{ t('providers.codex.disconnect') }}
              </Button>
            </div>
          </li>
        </ul>
        <p v-else class="text-sm text-muted-foreground">{{ t('providers.codex.empty') }}</p>
        <div class="flex flex-wrap gap-2">
          <Button v-if="snapshot.enabled && browserIsLocal && !pending" data-testid="codex-start" type="button" :disabled="busy" @click="start">
            {{ t('providers.codex.start') }}
          </Button>
          <Button v-if="link && browserIsLocal" as-child>
            <a :href="link" target="_blank" rel="noopener noreferrer">{{ t('providers.codex.continue') }}</a>
          </Button>
          <Button v-if="pending" data-testid="codex-cancel" type="button" variant="outline" :disabled="busy" @click="cancel">
            {{ t('common.cancel') }}
          </Button>
        </div>
      </template>
    </DialogContent>
  </Dialog>
  <AlertDialog :open="disconnectDialogOpen" @update:open="setDisconnectDialogOpen">
    <AlertDialogContent>
      <AlertDialogHeader>
        <AlertDialogTitle>{{ t('providers.codex.disconnectTitle') }}</AlertDialogTitle>
        <AlertDialogDescription>{{ t('providers.codex.disconnectDescription') }}</AlertDialogDescription>
      </AlertDialogHeader>
      <AlertDialogFooter>
        <AlertDialogCancel>{{ t('common.cancel') }}</AlertDialogCancel>
        <AlertDialogAction data-testid="codex-disconnect-confirm" :disabled="busy" @click="confirmDisconnect">
          {{ t('providers.codex.disconnect') }}
        </AlertDialogAction>
      </AlertDialogFooter>
    </AlertDialogContent>
  </AlertDialog>
</template>
