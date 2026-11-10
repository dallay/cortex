<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {Skeleton} from "@/components/ui/skeleton";
import {cn} from "@/lib/utils";

const props = defineProps<{
  showIcon?: boolean;
  class?: HTMLAttributes["class"];
}>();

// Use a static width range for skeleton - no randomness needed for UI skeleton
const width = "70%";
</script>

<template>
  <div
    :class="cn('flex h-8 items-center gap-2 rounded-md px-2', props.class)"
    data-sidebar="menu-skeleton"
    data-slot="sidebar-menu-skeleton"
  >
    <Skeleton
      v-if="showIcon"
      class="size-4 rounded-md"
      data-sidebar="menu-skeleton-icon"
    />

    <Skeleton
      :style="{ '--skeleton-width': width }"
      class="h-4 max-w-(--skeleton-width) flex-1"
      data-sidebar="menu-skeleton-text"
    />
  </div>
</template>
