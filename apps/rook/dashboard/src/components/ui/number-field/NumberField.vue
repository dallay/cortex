<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {NumberFieldRootEmits, NumberFieldRootProps} from "reka-ui";
import {NumberFieldRoot, useForwardPropsEmits} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  NumberFieldRootProps & { class?: HTMLAttributes["class"] }
>();
const emits = defineEmits<NumberFieldRootEmits>();

const delegatedProps = reactiveOmit(props, "class");

const forwarded = useForwardPropsEmits(delegatedProps, emits);
</script>

<template>
  <NumberFieldRoot v-slot="slotProps" :class="cn('grid gap-1.5', props.class)" v-bind="forwarded">
    <slot v-bind="slotProps"/>
  </NumberFieldRoot>
</template>
