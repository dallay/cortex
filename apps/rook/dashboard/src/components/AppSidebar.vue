<script lang="ts" setup>
import NavMain from "@/components/NavMain.vue";
import NavSecondary from "@/components/NavSecondary.vue";
import NavUser from "@/components/NavUser.vue";
import {Sidebar, SidebarContent, SidebarFooter, SidebarHeader,} from "@/components/ui/sidebar";
import {useHealth} from "@/composables/useHealth";
import {useNavigation} from "@/composables/useNavigation";

const {navMain, navSecondary} = useNavigation();
const {appName} = useHealth();
</script>

<template>
  <Sidebar>
    <SidebarHeader>
      <div class="flex items-center gap-3 px-3 py-2">
        <div class="flex size-8 shrink-0 items-center justify-center">
          <img alt="Rook" class="block size-5 dark:hidden" src="/rook-isotype.svg"/>
          <img alt="Rook" class="hidden size-5 dark:block" src="/rook-isotype-dark.svg"/>
        </div>
        <div class="grid flex-1 text-left leading-tight">
          <span class="truncate text-xl font-semibold tracking-tight">{{ appName }}</span>
        </div>
      </div>
    </SidebarHeader>
    <SidebarContent>
      <NavMain :items="navMain"/>
      <NavSecondary :items="navSecondary"/>
    </SidebarContent>
    <SidebarFooter>
      <NavUser/>
    </SidebarFooter>
  </Sidebar>
</template>
