<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {Textarea} from "@/components/ui/textarea";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <Textarea
    :class="cn(
      'flex-1 resize-none rounded-none border-0 bg-transparent py-3 shadow-none focus-visible:ring-0 dark:bg-transparent',
      props.class,
    )"
    data-slot="input-group-control"
  />
</template>
