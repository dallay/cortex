<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { Activity, Key, Globe, CircleDot, TrendingUp, AlertCircle, CheckCircle2 } from '@lucide/vue'
import { useHealth } from '@/composables/useHealth'

const { t } = useI18n()

const { data: health, loading, error, averageLatency, healthyProviders } = useHealth()
</script>

<template>
  <div class="space-y-6">
    <!-- Page Header -->
    <div>
      <h1 class="text-2xl font-semibold tracking-tight">{{ t('home.title') }}</h1>
      <p class="text-muted-foreground">{{ t('home.subtitle') }}</p>
    </div>

    <!-- Error State -->
    <div v-if="error" class="rounded-lg border border-destructive/50 bg-destructive/10 p-4">
      <div class="flex items-center gap-2 text-destructive">
        <AlertCircle class="h-5 w-5" />
        <span class="font-medium">{{ t('common.error') }}</span>
      </div>
      <p class="mt-1 text-sm text-muted-foreground">{{ error }}</p>
    </div>

    <!-- Stats Grid -->
    <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-4">
      <div class="rounded-lg border bg-card p-4">
        <div class="flex items-center gap-3">
          <div class="p-2 rounded-lg bg-primary/10">
            <Activity class="h-5 w-5 text-primary" />
          </div>
          <div>
            <p class="text-sm text-muted-foreground">{{ t('home.systemStatus') }}</p>
            <div class="flex items-center gap-1.5">
              <CheckCircle2 v-if="health?.status === 'healthy'" class="h-4 w-4 text-green-500" />
              <AlertCircle v-else-if="health?.status === 'degraded'" class="h-4 w-4 text-yellow-500" />
              <span class="text-lg font-semibold capitalize">
                {{ health?.status || (loading ? '...' : 'unknown') }}
              </span>
            </div>
          </div>
        </div>
      </div>

      <div class="rounded-lg border bg-card p-4">
        <div class="flex items-center gap-3">
          <div class="p-2 rounded-lg bg-green-500/10">
            <Globe class="h-5 w-5 text-green-500" />
          </div>
          <div>
            <p class="text-sm text-muted-foreground">{{ t('home.activeProviders') }}</p>
            <p class="text-2xl font-semibold">
              {{ loading ? '...' : healthyProviders.length }}
              <span class="text-sm font-normal text-muted-foreground">
                / {{ health?.providers.length || 0 }}
              </span>
            </p>
          </div>
        </div>
      </div>

      <div class="rounded-lg border bg-card p-4">
        <div class="flex items-center gap-3">
          <div class="p-2 rounded-lg bg-blue-500/10">
            <TrendingUp class="h-5 w-5 text-blue-500" />
          </div>
          <div>
            <p class="text-sm text-muted-foreground">{{ t('home.avgLatency') }}</p>
            <p class="text-2xl font-semibold">
              {{ loading ? '...' : (averageLatency ?? '—') }}
              <span v-if="averageLatency" class="text-sm font-normal text-muted-foreground">ms</span>
            </p>
          </div>
        </div>
      </div>

      <div class="rounded-lg border bg-card p-4">
        <div class="flex items-center gap-3">
          <div class="p-2 rounded-lg bg-yellow-500/10">
            <Key class="h-5 w-5 text-yellow-500" />
          </div>
          <div>
            <p class="text-sm text-muted-foreground">{{ t('home.apiKeys') }}</p>
            <p class="text-2xl font-semibold">—</p>
          </div>
        </div>
      </div>
    </div>

    <!-- Quick Actions -->
    <div class="grid gap-4 md:grid-cols-3">
      <a
        href="/endpoints"
        class="rounded-lg border bg-card p-4 hover:bg-muted/30 transition-colors group"
      >
        <CircleDot class="h-6 w-6 text-primary mb-2" />
        <h3 class="font-medium mb-1">{{ t('home.viewEndpoints') }}</h3>
        <p class="text-sm text-muted-foreground">{{ t('home.viewEndpointsDescription') }}</p>
      </a>

      <a
        href="/api-keys"
        class="rounded-lg border bg-card p-4 hover:bg-muted/30 transition-colors group"
      >
        <Key class="h-6 w-6 text-primary mb-2" />
        <h3 class="font-medium mb-1">{{ t('home.manageKeys') }}</h3>
        <p class="text-sm text-muted-foreground">{{ t('home.manageKeysDescription') }}</p>
      </a>

      <a
        href="/providers"
        class="rounded-lg border bg-card p-4 hover:bg-muted/30 transition-colors group"
      >
        <Globe class="h-6 w-6 text-primary mb-2" />
        <h3 class="font-medium mb-1">{{ t('home.viewProviders') }}</h3>
        <p class="text-sm text-muted-foreground">{{ t('home.viewProvidersDescription') }}</p>
      </a>
    </div>

    <!-- Provider Status -->
    <div v-if="health?.providers.length" class="rounded-lg border">
      <div class="px-4 py-3 border-b">
        <h2 class="font-medium">{{ t('home.providerStatus') }}</h2>
      </div>
      <div class="divide-y">
        <div
          v-for="provider in health.providers"
          :key="provider.id"
          class="px-4 py-3 flex items-center justify-between"
        >
          <div class="flex items-center gap-3">
            <CheckCircle2 v-if="provider.healthy" class="h-4 w-4 text-green-500" />
            <AlertCircle v-else class="h-4 w-4 text-destructive" />
            <span class="font-mono text-sm">{{ provider.id }}</span>
          </div>
          <div class="flex items-center gap-4 text-sm text-muted-foreground">
            <span v-if="provider.latency_ms">{{ provider.latency_ms }}ms</span>
            <span v-if="provider.last_error" class="text-destructive text-xs max-w-[200px] truncate">
              {{ provider.last_error }}
            </span>
          </div>
        </div>
      </div>
    </div>

    <!-- Empty State -->
    <div v-if="!loading && !error && health?.status === 'no_providers_configured'" class="rounded-lg border border-dashed p-8 text-center">
      <Globe class="h-12 w-12 mx-auto text-muted-foreground mb-4" />
      <h3 class="font-medium mb-2">{{ t('home.noProvidersTitle') }}</h3>
      <p class="text-sm text-muted-foreground mb-4">{{ t('home.noProvidersDescription') }}</p>
      <a
        href="/providers"
        class="inline-flex items-center gap-2 px-4 py-2 bg-primary text-primary-foreground rounded-md hover:bg-primary/90 transition-colors"
      >
        {{ t('providers.add') }}
      </a>
    </div>
  </div>
</template>
