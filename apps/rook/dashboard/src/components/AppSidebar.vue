<script setup lang="ts">
import NavMain from "@/components/NavMain.vue";
import NavSecondary from "@/components/NavSecondary.vue";
import NavUser from "@/components/NavUser.vue";
import {
	Sidebar,
	SidebarContent,
	SidebarFooter,
	SidebarHeader,
} from "@/components/ui/sidebar";
import { useNavigation } from "@/composables/useNavigation";

const { navMain, navSecondary } = useNavigation();
</script>

<template>
  <Sidebar>
    <SidebarHeader>
      <div class="flex items-center gap-2 px-2 py-1">
        <div
          class="flex aspect-square size-8 items-center justify-center rounded-full bg-sidebar-primary-foreground">
          <img src="/rook-icon.svg" alt="Rook"
               class="size-6 dark:brightness-200 dark:contrast-125"/>
        </div>
        <div class="grid flex-1 text-left text-2xl leading-tight">
          <span class="truncate font-medium">Rook</span>
        </div>
      </div>
    </SidebarHeader>
    <SidebarContent>
      <NavMain :items="navMain" />
      <NavSecondary :items="navSecondary" />
    </SidebarContent>
    <SidebarFooter>
      <NavUser />
    </SidebarFooter>
  </Sidebar>
</template>
