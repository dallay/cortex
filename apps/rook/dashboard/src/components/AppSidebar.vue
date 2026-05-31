<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import {
  CircleDot,
  Globe,
  Home,
  Key,
  LifeBuoy,
  ListOrdered,
  Settings,
  SlidersHorizontal,
  Send,
} from '@lucide/vue'

import NavMain from '@/components/NavMain.vue'
import NavSecondary from '@/components/NavSecondary.vue'
import NavUser from '@/components/NavUser.vue'
import { ThemeToggle } from '@/components/ui/theme-toggle'
import { LocaleSwitcher } from '@/components/ui/locale-switcher'
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarHeader,
} from '@/components/ui/sidebar'

const { t } = useI18n()

const data = computed(() => ({
  navMain: [
    {
      title: t('nav.home'),
      url: '/',
      icon: Home,
      isActive: true,
    },
    {
      title: t('nav.apiKeys'),
      url: '/api-keys',
      icon: Key,
      items: [
        { title: t('nav.apiKeysList'), url: '/api-keys' },
        { title: t('nav.apiKeysCreate'), url: '/api-keys/new' },
      ],
    },
    {
      title: t('nav.endpoints'),
      url: '/endpoints',
      icon: ListOrdered,
    },
    {
      title: t('nav.providers'),
      url: '/providers',
      icon: Globe,
      items: [
        { title: t('nav.providersList'), url: '/providers' },
        { title: t('nav.providersQuotes'), url: '/providers/quotes' },
      ],
    },
    {
      title: t('nav.combos'),
      url: '/combos',
      icon: CircleDot,
    },
    {
      title: t('nav.settings'),
      url: '/settings',
      icon: Settings,
    },
  ],
  navSecondary: [
    { title: 'Support', url: '#', icon: LifeBuoy },
    { title: 'Feedback', url: '#', icon: Send },
  ],
}))
</script>

<template>
  <Sidebar>
    <SidebarHeader>
      <div class="flex items-center gap-2 px-2 py-1">
        <div class="flex aspect-square size-8 items-center justify-center rounded-lg bg-sidebar-primary text-sidebar-primary-foreground">
          <SlidersHorizontal class="size-4" />
        </div>
        <div class="grid flex-1 text-left text-sm leading-tight">
          <span class="truncate font-medium">Rook</span>
          <span class="truncate text-xs text-muted-foreground">API Gateway</span>
        </div>
      </div>
    </SidebarHeader>
    <SidebarContent>
      <NavMain :items="data.navMain" />
      <NavSecondary :items="data.navSecondary" />
    </SidebarContent>
    <SidebarFooter>
      <div class="flex items-center justify-between gap-2 px-2 py-1">
        <LocaleSwitcher />
        <ThemeToggle />
      </div>
      <NavUser />
    </SidebarFooter>
  </Sidebar>
</template>