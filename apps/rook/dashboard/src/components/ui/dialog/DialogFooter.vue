<script lang="ts" setup>
import {DialogClose} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {Button} from "@/components/ui/button";
import {cn} from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    class?: HTMLAttributes["class"];
    showCloseButton?: boolean;
  }>(),
  {
    showCloseButton: false,
  },
);
</script>

<template>
  <div
    :class="cn('flex flex-col-reverse gap-2 sm:flex-row sm:justify-end', props.class)"
    data-slot="dialog-footer"
  >
    <slot/>
    <DialogClose v-if="showCloseButton" as-child>
      <Button variant="outline">
        Close
      </Button>
    </DialogClose>
  </div>
</template>
