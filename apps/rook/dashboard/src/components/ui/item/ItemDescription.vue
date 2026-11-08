<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <p
    :class="cn(
      'text-muted-foreground line-clamp-2 text-sm leading-normal font-normal text-balance',
      '[&>a:hover]:text-primary [&>a]:underline [&>a]:underline-offset-4',
      props.class,
    )"
    data-slot="item-description"
  >
    <slot/>
  </p>
</template>
