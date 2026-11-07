<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {RadioGroupRootEmits, RadioGroupRootProps} from "reka-ui";
import {RadioGroupRoot, useForwardPropsEmits} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  RadioGroupRootProps & { class?: HTMLAttributes["class"] }
>();
const emits = defineEmits<RadioGroupRootEmits>();

const delegatedProps = reactiveOmit(props, "class");

const forwarded = useForwardPropsEmits(delegatedProps, emits);
</script>

<template>
  <RadioGroupRoot
    v-slot="slotProps"
    :class="cn('grid gap-3', props.class)"
    data-slot="radio-group"
    v-bind="forwarded"
  >
    <slot v-bind="slotProps"/>
  </RadioGroupRoot>
</template>
