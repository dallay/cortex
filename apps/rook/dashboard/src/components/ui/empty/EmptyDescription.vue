<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <p
    :class="cn(
      'text-muted-foreground [&>a:hover]:text-primary text-sm/relaxed [&>a]:underline [&>a]:underline-offset-4',
      $attrs.class ?? '',
    )"
    data-slot="empty-description"
  >
    <slot/>
  </p>
</template>
