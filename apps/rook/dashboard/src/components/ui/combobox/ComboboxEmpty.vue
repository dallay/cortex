<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {ComboboxEmptyProps} from "reka-ui";
import {ComboboxEmpty} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  ComboboxEmptyProps & { class?: HTMLAttributes["class"] }
>();

const delegatedProps = reactiveOmit(props, "class");
</script>

<template>
  <ComboboxEmpty
    :class="cn('py-6 text-center text-sm', props.class)"
    data-slot="combobox-empty"
    v-bind="delegatedProps"
  >
    <slot/>
  </ComboboxEmpty>
</template>
