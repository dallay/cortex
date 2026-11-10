<script lang="ts" setup>
import {MoreHorizontal} from "@lucide/vue";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <li
    :class="cn('flex size-9 items-center justify-center', props.class)"
    data-slot="breadcrumb-ellipsis"
  >
    <slot>
      <MoreHorizontal class="size-4"/>
    </slot>
    <span class="sr-only">More</span>
  </li>
</template>
