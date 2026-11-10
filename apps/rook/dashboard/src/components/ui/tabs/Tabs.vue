<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {TabsRootEmits, TabsRootProps} from "reka-ui";
import {TabsRoot, useForwardPropsEmits} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  TabsRootProps & { class?: HTMLAttributes["class"] }
>();
const emits = defineEmits<TabsRootEmits>();

const delegatedProps = reactiveOmit(props, "class");
const forwarded = useForwardPropsEmits(delegatedProps, emits);
</script>

<template>
  <TabsRoot
    v-slot="slotProps"
    :class="cn('flex flex-col gap-2', props.class)"
    data-slot="tabs"
    v-bind="forwarded"
  >
    <slot v-bind="slotProps"/>
  </TabsRoot>
</template>
