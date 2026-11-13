<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {TabsContentProps} from "reka-ui";
import {TabsContent} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  TabsContentProps & { class?: HTMLAttributes["class"] }
>();

const delegatedProps = reactiveOmit(props, "class");
</script>

<template>
  <TabsContent
    :class="cn('flex-1 outline-none', props.class)"
    data-slot="tabs-content"
    v-bind="delegatedProps"
  >
    <slot/>
  </TabsContent>
</template>
