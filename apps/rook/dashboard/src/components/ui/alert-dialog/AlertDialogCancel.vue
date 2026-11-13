<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {AlertDialogCancelProps} from "reka-ui";
import {AlertDialogCancel} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {buttonVariants} from "@/components/ui/button";
import {cn} from "@/lib/utils";

const props = defineProps<
  AlertDialogCancelProps & { class?: HTMLAttributes["class"] }
>();

const delegatedProps = reactiveOmit(props, "class");
</script>

<template>
  <AlertDialogCancel
    :class="cn(
      buttonVariants({ variant: 'outline' }),
      'mt-2 sm:mt-0',
      props.class,
    )"
    v-bind="delegatedProps"
  >
    <slot/>
  </AlertDialogCancel>
</template>
