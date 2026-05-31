<script setup lang="ts">
import type { LucideIcon } from '@lucide/vue'
import { LifeBuoy, Send } from '@lucide/vue'
import {
  SidebarGroup,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from '@/components/ui/sidebar'

defineProps<{
  items: { title: string; url: string; icon: LucideIcon }[]
}>()
</script>

<template>
  <SidebarGroup class="mt-auto">
    <SidebarGroupLabel>Support</SidebarGroupLabel>
    <SidebarMenu>
      <SidebarMenuItem v-for="item in items" :key="item.title">
        <SidebarMenuButton as-child :tooltip="item.title">
          <a :href="item.url">
            <component :is="item.icon" />
            <span>{{ item.title }}</span>
          </a>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  </SidebarGroup>
</template>