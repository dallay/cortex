<script setup lang="ts">
import { SlidersHorizontal } from '@lucide/vue'

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
import { useNavigation } from '@/composables/useNavigation'

const { navMain, navSecondary } = useNavigation()
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
      <NavMain :items="navMain" />
      <NavSecondary :items="navSecondary" />
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
