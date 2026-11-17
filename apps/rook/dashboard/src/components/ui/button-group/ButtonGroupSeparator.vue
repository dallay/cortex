<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {SeparatorProps} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {Separator} from "@/components/ui/separator";
import {cn} from "@/lib/utils";

const props = withDefaults(
  defineProps<SeparatorProps & { class?: HTMLAttributes["class"] }>(),
  {
    orientation: "vertical",
  },
);
const delegatedProps = reactiveOmit(props, "class");
</script>

<template>
  <Separator
    :class="cn(
      'bg-input relative !m-0 self-stretch data-[orientation=vertical]:h-auto',
      props.class,
    )"
    :orientation="props.orientation"
    data-slot="button-group-separator"
    v-bind="delegatedProps"
  />
</template>
