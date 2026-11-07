<script lang="ts" setup>
import type {SeparatorProps} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {Separator} from "@/components/ui/separator";
import {cn} from "@/lib/utils";

const props = defineProps<
  SeparatorProps & { class?: HTMLAttributes["class"] }
>();
</script>

<template>
  <Separator
    :class="cn('my-0', props.class)"
    data-slot="item-separator"
    orientation="horizontal"
  />
</template>
