<script generic="Type extends 'text' | 'number' = 'text'" lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {PinInputRootEmits, PinInputRootProps} from "reka-ui";
import {PinInputRoot, useForwardPropsEmits} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = withDefaults(
  defineProps<PinInputRootProps<Type> & { class?: HTMLAttributes["class"] }>(),
  {
    otp: true,
  },
);
const emits = defineEmits<PinInputRootEmits<Type>>();

const delegatedProps = reactiveOmit(props, "class");

const forwarded = useForwardPropsEmits(delegatedProps, emits);
</script>

<template>
  <PinInputRoot
    :class="cn('flex items-center gap-2 has-disabled:opacity-50 disabled:cursor-not-allowed', props.class)"
    :otp="props.otp"
    data-slot="pin-input"
    v-bind="forwarded"
  >
    <slot/>
  </PinInputRoot>
</template>
