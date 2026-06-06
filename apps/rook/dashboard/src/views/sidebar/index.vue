<script setup lang="ts">
/**
 * Sidebar layout — owns the top bar with the 3-level breadcrumb.
 *
 * Breadcrumb shape:
 *   - 1-level  (root `/`)                : just "Home"
 *   - 2-level  (e.g. `/combos`)          : Home → <Page>
 *   - 3-level  (e.g. `/providers/ollama`): Home → Providers → Ollama
 *
 * The 3-level mode is opt-in via `meta.breadcrumb: true` on the route
 * AND `route.matched.length >= 3`. The matched-length guard is a
 * defense-in-depth check; the opt-in is the actual switch. Without
 * the opt-in, every 2-level page (`/combos`, `/settings`,
 * `/providers`, `/providers/quota`) keeps its existing 2-level shape.
 */
import { computed } from 'vue'
import type { RouteLocationRaw } from 'vue-router'
import { useRoute, RouterLink } from 'vue-router'
import { useI18n } from 'vue-i18n'
import AppSidebar from '@/components/AppSidebar.vue'
import ThemeToggle from '@/components/ui/theme-toggle/ThemeToggle.vue'
import { LocaleSwitcher } from '@/components/ui/locale-switcher'
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
} from '@/components/ui/breadcrumb'
import { Separator } from '@/components/ui/separator'
import {
  SidebarInset,
  SidebarProvider,
  SidebarTrigger,
} from '@/components/ui/sidebar'

interface Crumb {
  /** Resolved label (already passed through `t()`). */
  label: string
  /** Router target. The last crumb typically points to the current route. */
  to: RouteLocationRaw
  /** `true` for the last crumb — renders as `BreadcrumbPage`, not a link. */
  isCurrent: boolean
}

const route = useRoute()
const { t } = useI18n()

/**
 * Resolve the label for the *last* crumb in the breadcrumb.
 *
 * Most pages use `route.meta.title` directly. The provider-details page
 * is special: its title is a parametrized template (`"{providerName}
 * connections"`) but the breadcrumb wants the kind's display name
 * alone (e.g. "Ollama"). We look it up via the static `providers.kind.*`
 * i18n keys, which already exist for every `ProviderKind`.
 */
function lastCrumbLabel(): string {
  if (route.meta.breadcrumb === true) {
    const kind = route.params.providerKind
    if (typeof kind === 'string' && kind.length > 0) {
      return t(`providers.kind.${kind}`)
    }
  }
  const titleKey = route.meta.title as string | undefined
  if (titleKey) return t(titleKey)
  return (route.name as string | undefined) ?? t('nav.home')
}

/**
 * Section (middle) crumb label for 3-level rendering. Sourced from the
 * parent route's `meta.title` so any future nested section can opt in
 * by setting both `title` and `breadcrumb: true` on its children.
 */
function sectionCrumbLabel(): string {
  // matched[0] = Sidebar layout, matched[1] = provider section parent
  const sectionMeta = route.matched[1]?.meta
  const titleKey = (sectionMeta?.title as string | undefined)
    ?? 'nav.providersCatalog'
  return t(titleKey)
}

const breadcrumbs = computed<Crumb[]>(() => {
  // Root page — no breadcrumb at all.
  if (route.matched.length <= 1) return []

  const wantsThreeLevels =
    route.matched.length >= 3 && route.meta.breadcrumb === true

  if (wantsThreeLevels) {
    return [
      { label: t('nav.home'), to: { name: 'Home' }, isCurrent: false },
      { label: sectionCrumbLabel(), to: { name: 'Providers' }, isCurrent: false },
      { label: lastCrumbLabel(), to: route, isCurrent: true },
    ]
  }

  // 2-level — Home + current page.
  return [
    { label: t('nav.home'), to: { name: 'Home' }, isCurrent: false },
    { label: lastCrumbLabel(), to: route, isCurrent: true },
  ]
})
</script>

<template>
  <SidebarProvider>
    <AppSidebar />
    <SidebarInset>
      <header class="flex h-16 shrink-0 items-center gap-2 px-4">
        <div class="flex items-center gap-2">
          <SidebarTrigger class="-ml-1" />
          <Separator
            orientation="vertical"
            class="mr-2 data-[orientation=vertical]:h-4"
          />
          <Breadcrumb v-if="breadcrumbs.length > 0">
            <BreadcrumbList>
              <template v-for="(crumb, idx) in breadcrumbs" :key="idx">
                <BreadcrumbItem>
                  <BreadcrumbLink v-if="!crumb.isCurrent" as-child>
                    <RouterLink :to="crumb.to">{{ crumb.label }}</RouterLink>
                  </BreadcrumbLink>
                  <BreadcrumbPage v-else>{{ crumb.label }}</BreadcrumbPage>
                </BreadcrumbItem>
                <BreadcrumbSeparator v-if="idx < breadcrumbs.length - 1" />
              </template>
            </BreadcrumbList>
          </Breadcrumb>
        </div>
        <div class="ml-auto flex items-center gap-2">
          <LocaleSwitcher />
          <ThemeToggle />
        </div>
      </header>
      <div class="flex flex-1 flex-col gap-4 p-4 pt-0">
        <RouterView />
      </div>
    </SidebarInset>
  </SidebarProvider>
</template>
