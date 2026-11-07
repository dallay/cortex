<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {DrawerDescriptionProps} from "vaul-vue";
import {DrawerDescription} from "vaul-vue";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  DrawerDescriptionProps & { class?: HTMLAttributes["class"] }
>();

const delegatedProps = reactiveOmit(props, "class");
</script>

<template>
  <DrawerDescription
    :class="cn('text-muted-foreground text-sm', props.class)"
    data-slot="drawer-description"
    v-bind="delegatedProps"
  >
    <slot/>
  </DrawerDescription>
</template>
