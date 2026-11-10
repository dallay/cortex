<script lang="ts" setup>
import {ChevronRight} from "@lucide/vue";
import type {Component} from "vue";
import {useI18n} from "vue-i18n";
import {RouterLink} from "vue-router";
import {Collapsible, CollapsibleContent, CollapsibleTrigger,} from "@/components/ui/collapsible";
import {
  SidebarGroup,
  SidebarGroupLabel,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubButton,
  SidebarMenuSubItem,
} from "@/components/ui/sidebar";

const {t} = useI18n();

defineProps<{
  items: {
    title: string;
    url: string;
    icon: Component;
    isActive?: boolean;
    items?: {
      title: string;
      url: string;
    }[];
  }[];
}>();
</script>

<template>
  <SidebarGroup>
    <SidebarGroupLabel>{{ t('sidebar.platform') }}</SidebarGroupLabel>
    <SidebarMenu>
      <Collapsible v-for="item in items" :key="item.title" :default-open="item.isActive" as-child>
        <SidebarMenuItem>
          <SidebarMenuButton :tooltip="item.title" as-child>
            <RouterLink :to="item.url">
              <component :is="item.icon"/>
              <span>{{ item.title }}</span>
            </RouterLink>
          </SidebarMenuButton>
          <template v-if="item.items?.length">
            <CollapsibleTrigger as-child>
              <SidebarMenuAction class="data-[state=open]:rotate-90">
                <ChevronRight/>
                <span class="sr-only">Toggle</span>
              </SidebarMenuAction>
            </CollapsibleTrigger>
            <CollapsibleContent>
              <SidebarMenuSub>
                <SidebarMenuSubItem v-for="subItem in item.items" :key="subItem.title">
                  <SidebarMenuSubButton as-child>
                    <RouterLink :to="subItem.url">
                      <span>{{ subItem.title }}</span>
                    </RouterLink>
                  </SidebarMenuSubButton>
                </SidebarMenuSubItem>
              </SidebarMenuSub>
            </CollapsibleContent>
          </template>
        </SidebarMenuItem>
      </Collapsible>
    </SidebarMenu>
  </SidebarGroup>
</template>
