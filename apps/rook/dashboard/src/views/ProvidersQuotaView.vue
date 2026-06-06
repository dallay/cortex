<script setup lang="ts">
/**
 * ProvidersQuotaView — placeholder for per-provider quota tracking.
 *
 * Per the design (`openspec/changes/providers-ui-3-screen-refactor/design.md`
 * §D7), the quota view is a low-effort placeholder until the backend
 * exposes a quota endpoint. The page makes the "coming soon" status
 * explicit so users land here and understand why the data is missing.
 *
 * It also renders a per-kind summary table sourced from
 * `useProviderCatalog` so the view is not entirely empty while the
 * real integration is pending. The summary answers "how many
 * connections and which models are configured per provider?" without
 * showing actual token usage.
 */
import { onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { Fuel, Info } from '@lucide/vue'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table'
import { useProviders } from '@/composables/useProviders'
import { useAvailableModels } from '@/composables/useAvailableModels'
import { useProviderCatalog } from '@/composables/useProviderCatalog'

const { t } = useI18n()

const { fetch, loading } = useProviders()
const { fetch: fetchModels } = useAvailableModels()
const { items } = useProviderCatalog()

onMounted(() => {
  fetch()
  fetchModels()
})

// TODO: replace with the real implementation once the backend exposes
// a quota endpoint. Track the follow-up work in the GitHub issue
// linked from the spec/design doc.
</script>

<template>
  <div class="space-y-6">
    <div>
      <h1 class="text-2xl font-semibold tracking-tight flex items-center gap-2">
        <Fuel class="h-6 w-6 text-primary" />
        {{ t('providers.quota.title') }}
      </h1>
      <p class="text-muted-foreground">
        {{ t('providers.quota.subtitle') }}
      </p>
    </div>

    <Alert>
      <Info class="h-4 w-4" />
      <AlertTitle>{{ t('providers.quota.comingSoon') }}</AlertTitle>
      <AlertDescription>
        {{ t('providers.quota.banner') }}
        <!-- TODO: link to the implementation tracking issue once filed.
             Do not hardcode a GitHub URL here until the issue exists. -->
      </AlertDescription>
    </Alert>

    <div class="rounded-md border">
      <Table>
        <TableHeader>
          <TableRow class="bg-muted/50">
            <TableHead class="px-4 py-3 text-left text-sm font-medium">
              {{ t('providers.name') }}
            </TableHead>
            <TableHead class="px-4 py-3 text-right text-sm font-medium">
              {{ t('providers.status') }}
            </TableHead>
            <TableHead class="px-4 py-3 text-left text-sm font-medium">
              {{ t('providers.form.models') }}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <template v-if="loading && items.length === 0">
            <TableRow>
              <TableCell colspan="3" class="px-4 py-8 text-center text-sm text-muted-foreground">
                {{ t('common.loading') }}
              </TableCell>
            </TableRow>
          </template>
          <template v-else>
            <TableRow
              v-for="item in items"
              :key="item.kind"
              class="hover:bg-muted/30"
              :data-testid="`quota-row-${item.kind}`"
            >
              <TableCell class="px-4 py-3 font-medium">
                {{ t(item.displayNameKey) }}
              </TableCell>
              <TableCell class="px-4 py-3 text-right">
                <Badge :variant="item.hasActiveConnections ? 'default' : 'secondary'">
                  {{
                    item.hasActiveConnections
                      ? t('providers.catalog.active')
                      : t('providers.catalog.notConfigured')
                  }}
                </Badge>
              </TableCell>
              <TableCell class="px-4 py-3 text-sm text-muted-foreground">
                <span v-if="item.configuredModels.length > 0" class="font-mono text-xs">
                  {{ item.configuredModels.slice(0, 3).join(', ') }}
                  <span v-if="item.configuredModels.length > 3">
                    +{{ item.configuredModels.length - 3 }}
                  </span>
                </span>
                <span v-else>—</span>
              </TableCell>
            </TableRow>
          </template>
        </TableBody>
      </Table>
    </div>
  </div>
</template>
