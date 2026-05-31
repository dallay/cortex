<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Globe, Key, Bell, Shield } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { useBaseUrl } from '@/composables/useBaseUrl'

const { t } = useI18n()
const { baseUrl, fullBaseUrl, isOverridden, setOverride, clearOverride } = useBaseUrl()

// Settings state
const settings = ref({
  customBaseUrl: isOverridden.value ? baseUrl.value : '',
  port: '8080',
  logLevel: 'info',
  maxConcurrentRequests: 100,
  enableAuditLog: true,
  enableMetrics: true,
})

function saveApiSettings() {
  if (settings.value.customBaseUrl) {
    setOverride(settings.value.customBaseUrl)
  } else {
    clearOverride()
  }
}

function saveGeneralSettings() {
  // TODO: call API to save settings
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
        <Globe class="h-4 w-4 text-muted-foreground" />
        <h2 class="font-medium">{{ t('settings.api') }}</h2>
      </div>
      <div class="p-4 space-y-4">
        <div>
          <label class="text-sm font-medium block mb-1.5">
            {{ t('settings.baseUrl') }}
          </label>
          <div class="flex gap-2">
            <Input
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
          <label class="text-sm font-medium block mb-1.5">
            {{ t('settings.port') }}
          </label>
          <Input v-model="settings.port" class="w-32" />
        </div>
      </div>
    </section>

    <!-- Security Settings -->
    <section class="rounded-lg border bg-card">
      <div class="px-4 py-3 border-b flex items-center gap-2">
        <Shield class="h-4 w-4 text-muted-foreground" />
        <h2 class="font-medium">{{ t('settings.security') }}</h2>
      </div>
      <div class="p-4 space-y-4">
        <div class="flex items-center justify-between">
          <div>
            <label class="text-sm font-medium block">{{ t('settings.auditLog') }}</label>
            <p class="text-xs text-muted-foreground">{{ t('settings.auditLogDescription') }}</p>
          </div>
          <input
            type="checkbox"
            v-model="settings.enableAuditLog"
            class="h-4 w-4 rounded border-input"
          />
        </div>
        <div class="flex items-center justify-between">
          <div>
            <label class="text-sm font-medium block">{{ t('settings.metrics') }}</label>
            <p class="text-xs text-muted-foreground">{{ t('settings.metricsDescription') }}</p>
          </div>
          <input
            type="checkbox"
            v-model="settings.enableMetrics"
            class="h-4 w-4 rounded border-input"
          />
        </div>
      </div>
    </section>

    <!-- General Settings -->
    <section class="rounded-lg border bg-card">
      <div class="px-4 py-3 border-b flex items-center gap-2">
        <Bell class="h-4 w-4 text-muted-foreground" />
        <h2 class="font-medium">{{ t('settings.general') }}</h2>
      </div>
      <div class="p-4 space-y-4">
        <div>
          <label class="text-sm font-medium block mb-1.5">
            {{ t('settings.logLevel') }}
          </label>
          <select
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
          <label class="text-sm font-medium block mb-1.5">
            {{ t('settings.maxConcurrent') }}
          </label>
          <Input v-model.number="settings.maxConcurrentRequests" type="number" class="w-32" />
        </div>
        <div class="pt-2">
          <Button @click="saveGeneralSettings">{{ t('common.save') }}</Button>
        </div>
      </div>
    </section>

    <!-- About -->
    <section class="rounded-lg border bg-card">
      <div class="px-4 py-3 border-b flex items-center gap-2">
        <Key class="h-4 w-4 text-muted-foreground" />
        <h2 class="font-medium">{{ t('settings.about') }}</h2>
      </div>
      <div class="p-4">
        <div class="text-sm space-y-1">
          <p><span class="text-muted-foreground">Rook</span> v0.1.0</p>
          <p><span class="text-muted-foreground">API Gateway</span> · AI Router</p>
        </div>
      </div>
    </section>
  </div>
</template>
