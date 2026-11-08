<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <li
    :class="cn('text-foreground font-normal', props.class)"
    aria-current="page"
    data-slot="breadcrumb-page"
  >
    <slot/>
  </li>
</template>
