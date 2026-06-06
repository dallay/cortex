<script setup lang="ts">
/**
 * ConnectionListItem — single row in the ProviderDetailsView connection
 * list. Represents one `ProviderConnectionResponse` with edit / test /
 * delete / enable-disable affordances.
 *
 * Status icon mirrors the visual language of the legacy `ProvidersView`
 * table: green for `active`, red for `unhealthy`, yellow for everything
 * else (e.g. `unknown`, `untested`).
 *
 * Models are passed in as a prop (joined by the parent from
 * `useAvailableModels().modelsByProvider`) so this component stays a
 * pure presentational unit. The list is truncated to 3 entries; any
 * overflow is surfaced via a "+N more" badge.
 */
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Pencil, Trash2, RefreshCw, CheckCircle2, AlertCircle, Activity, Loader2 } from '@lucide/vue'
import { Switch } from '@/components/ui/switch'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import type { ProviderConnectionResponse } from '@/lib/api'

const props = defineProps<{
  connection: ProviderConnectionResponse
  models?: readonly string[]
  testing?: boolean
  busy?: boolean
}>()

const emit = defineEmits<{
  test: [id: string]
  edit: [id: string]
  delete: [id: string]
  toggle: [payload: { id: string; enabled: boolean }]
}>()

const { t } = useI18n()

const statusClass = computed(() => {
  const status = props.connection.testStatus.status
  if (status === 'active') return 'text-green-500'
  if (status === 'unhealthy') return 'text-destructive'
  return 'text-yellow-500'
})

const visibleModels = computed(() => props.models?.slice(0, 3) ?? [])

const overflowCount = computed(() =>
  Math.max((props.models?.length ?? 0) - visibleModels.value.length, 0),
)

const enabled = computed(() => props.connection.isActive)
</script>

<template>
  <div
    class="rounded-lg border p-4 space-y-3"
    :data-testid="`connection-row-${connection.id}`"
  >
    <div class="flex items-start justify-between gap-3">
      <div class="space-y-1 min-w-0 flex-1">
        <div class="flex items-center gap-2 flex-wrap">
          <span class="font-medium truncate">{{ connection.name }}</span>
          <Badge v-if="!enabled" variant="secondary">
            {{ t('providers.form.enabled') }}
          </Badge>
        </div>
        <div class="flex items-center gap-2 text-xs text-muted-foreground flex-wrap">
          <Loader2 v-if="testing" class="h-3 w-3 animate-spin" />
          <AlertCircle v-else-if="connection.testStatus.status === 'unhealthy'" :class="['h-3 w-3', statusClass]" />
          <CheckCircle2 v-else :class="['h-3 w-3', statusClass]" />
          <span class="capitalize">{{ connection.testStatus.status }}</span>
          <span v-if="connection.testStatus.latencyMs != null" class="inline-flex items-center gap-1">
            <Activity class="h-3 w-3" />
            {{ connection.testStatus.latencyMs }}ms
          </span>
        </div>
        <p
          v-if="connection.testStatus.error"
          class="text-xs text-destructive truncate max-w-[400px]"
        >
          {{ connection.testStatus.error }}
        </p>
      </div>
      <div class="flex items-center gap-1 shrink-0">
        <Button
          variant="ghost"
          size="sm"
          :disabled="busy || testing"
          :data-testid="`connection-test-${connection.id}`"
          @click="emit('test', connection.id)"
        >
          <RefreshCw class="h-4 w-4" />
        </Button>
        <Button
          variant="ghost"
          size="sm"
          :disabled="busy"
          :data-testid="`connection-edit-${connection.id}`"
          @click="emit('edit', connection.id)"
        >
          <Pencil class="h-4 w-4" />
        </Button>
        <Button
          variant="ghost"
          size="sm"
          class="text-destructive hover:text-destructive"
          :disabled="busy"
          :data-testid="`connection-delete-${connection.id}`"
          @click="emit('delete', connection.id)"
        >
          <Trash2 class="h-4 w-4" />
        </Button>
        <Switch
          :checked="enabled"
          :disabled="busy"
          :data-testid="`connection-toggle-${connection.id}`"
          @update:checked="(value: boolean) => emit('toggle', { id: connection.id, enabled: value })"
        />
      </div>
    </div>
    <div v-if="visibleModels.length > 0" class="flex flex-wrap items-center gap-1.5">
      <Badge
        v-for="model in visibleModels"
        :key="model"
        variant="outline"
        class="font-mono text-xs"
      >
        {{ model }}
      </Badge>
      <Badge v-if="overflowCount > 0" variant="secondary" class="text-xs">
        +{{ overflowCount }} more
      </Badge>
    </div>
  </div>
</template>
