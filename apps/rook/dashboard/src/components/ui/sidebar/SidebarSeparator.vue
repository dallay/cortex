<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {Separator} from "@/components/ui/separator";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <Separator
    :class="cn('bg-sidebar-border mx-2 w-auto', props.class)"
    data-sidebar="separator"
    data-slot="sidebar-separator"
  >
    <slot/>
  </Separator>
</template>
