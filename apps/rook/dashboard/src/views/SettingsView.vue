<script lang="ts" setup>
import {Bell, Globe, Key, Shield} from "@lucide/vue";
import {ref} from "vue";
import {useI18n} from "vue-i18n";
import {Button} from "@/components/ui/button";
import {Input} from "@/components/ui/input";
import {useBaseUrl} from "@/composables/useBaseUrl";
import {useHealth} from "@/composables/useHealth";

const {t} = useI18n();
const {baseUrl, fullBaseUrl, isOverridden, setOverride, clearOverride} =
  useBaseUrl();
const {appName, version} = useHealth();

// Settings state
const settings = ref({
  customBaseUrl: isOverridden.value ? baseUrl.value : "",
  port: "8080",
  logLevel: "info",
  maxConcurrentRequests: 100,
  enableAuditLog: true,
  enableMetrics: true,
});

function saveApiSettings() {
  if (settings.value.customBaseUrl) {
    setOverride(settings.value.customBaseUrl);
  } else {
    clearOverride();
  }
}

function saveGeneralSettings() {
  // NOTE: settings persistence will call the API once the backend endpoint lands.
}
</script>

<template>
  <div class="space-y-6 max-w-2xl">
    <!-- Page Header -->
    <div>
      <h1 class="text-2xl font-semibold tracking-tight">{{ t('nav.settings') }}</h1>
      <p class="text-muted-foreground">{{ t('settings.description') }}</p>
    </div>

    <!-- API Settings -->
    <section class="rounded-lg border bg-card">
      <div class="px-4 py-3 border-b flex items-center gap-2">
        <Globe class="h-4 w-4 text-muted-foreground"/>
        <h2 class="font-medium">{{ t('settings.api') }}</h2>
      </div>
      <div class="p-4 space-y-4">
        <div>
          <label class="text-sm font-medium block mb-1.5" for="settings-base-url">
            {{ t('settings.baseUrl') }}
          </label>
          <div class="flex gap-2">
            <Input
              id="settings-base-url"
              v-model="settings.customBaseUrl"
              :placeholder="fullBaseUrl"
              class="flex-1"
            />
            <Button @click="saveApiSettings">{{ t('common.save') }}</Button>
          </div>
          <p class="text-xs text-muted-foreground mt-1.5">
            {{ isOverridden ? t('settings.baseUrlCustom') : t('settings.baseUrlAuto') }}
          </p>
        </div>
        <div>
          <label class="text-sm font-medium block mb-1.5" for="settings-port">
            {{ t('settings.port') }}
          </label>
          <Input id="settings-port" v-model="settings.port" class="w-32"/>
        </div>
      </div>
    </section>

    <!-- Security Settings -->
    <section class="rounded-lg border bg-card">
      <div class="px-4 py-3 border-b flex items-center gap-2">
        <Shield class="h-4 w-4 text-muted-foreground"/>
        <h2 class="font-medium">{{ t('settings.security') }}</h2>
      </div>
      <div class="p-4 space-y-4">
        <div class="flex items-center justify-between">
          <div>
            <label class="text-sm font-medium block"
                   for="settings-audit-log">{{ t('settings.auditLog') }}</label>
            <p class="text-xs text-muted-foreground">{{ t('settings.auditLogDescription') }}</p>
          </div>
          <input
            id="settings-audit-log"
            v-model="settings.enableAuditLog"
            class="h-4 w-4 rounded border-input"
            type="checkbox"
          />
        </div>
        <div class="flex items-center justify-between">
          <div>
            <label class="text-sm font-medium block" for="settings-metrics">{{
                t('settings.metrics')
              }}</label>
            <p class="text-xs text-muted-foreground">{{ t('settings.metricsDescription') }}</p>
          </div>
          <input
            id="settings-metrics"
            v-model="settings.enableMetrics"
            class="h-4 w-4 rounded border-input"
            type="checkbox"
          />
        </div>
      </div>
    </section>

    <!-- General Settings -->
    <section class="rounded-lg border bg-card">
      <div class="px-4 py-3 border-b flex items-center gap-2">
        <Bell class="h-4 w-4 text-muted-foreground"/>
        <h2 class="font-medium">{{ t('settings.general') }}</h2>
      </div>
      <div class="p-4 space-y-4">
        <div>
          <label class="text-sm font-medium block mb-1.5" for="settings-log-level">
            {{ t('settings.logLevel') }}
          </label>
          <select
            id="settings-log-level"
            v-model="settings.logLevel"
            class="w-full h-9 rounded-md border border-input bg-transparent px-3 text-sm"
          >
            <option value="debug">Debug</option>
            <option value="info">Info</option>
            <option value="warn">Warn</option>
            <option value="error">Error</option>
          </select>
        </div>
        <div>
          <label class="text-sm font-medium block mb-1.5" for="settings-max-concurrent">
            {{ t('settings.maxConcurrent') }}
          </label>
          <Input id="settings-max-concurrent" v-model.number="settings.maxConcurrentRequests"
                 class="w-32" type="number"/>
        </div>
        <div class="pt-2">
          <Button @click="saveGeneralSettings">{{ t('common.save') }}</Button>
        </div>
      </div>
    </section>

    <!-- About -->
    <section class="rounded-lg border bg-card">
      <div class="px-4 py-3 border-b flex items-center gap-2">
        <Key class="h-4 w-4 text-muted-foreground"/>
        <h2 class="font-medium">{{ t('settings.about') }}</h2>
      </div>
      <div class="p-4">
        <div class="text-sm space-y-1">
          <p><span class="text-muted-foreground">{{ appName }}</span>
            <template v-if="version"> {{ version }}</template>
          </p>
          <p><span class="text-muted-foreground">API Gateway</span> · AI Router</p>
        </div>
      </div>
    </section>
  </div>
</template>
