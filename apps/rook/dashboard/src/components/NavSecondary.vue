<script setup lang="ts">
import type {Component} from "vue";
import {RouterLink} from "vue-router";
import {
  SidebarGroup,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar";

defineProps<{
  items: { title: string; url: string; icon: Component }[];
}>();
</script>

<template>
  <SidebarGroup class="mt-auto">
    <SidebarGroupLabel>Support</SidebarGroupLabel>
    <SidebarMenu>
      <SidebarMenuItem v-for="item in items" :key="item.title">
        <SidebarMenuButton as-child :tooltip="item.title">
          <RouterLink :to="item.url">
            <component :is="item.icon" />
            <span>{{ item.title }}</span>
          </RouterLink>
        </SidebarMenuButton>
      </SidebarMenuItem>
    </SidebarMenu>
  </SidebarGroup>
</template>
