<script setup lang="ts">
import type {Component} from "vue";
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";

defineProps<{
  title: string;
  description?: string;
  icon?: Component | null;
}>();
</script>

<template>
  <Empty>
    <EmptyHeader>
      <EmptyMedia v-if="icon" variant="icon">
        <component :is="icon" />
      </EmptyMedia>
      <EmptyTitle v-if="title">{{ title }}</EmptyTitle>
      <EmptyDescription v-if="description">{{ description }}</EmptyDescription>
    </EmptyHeader>
    <EmptyContent v-if="$slots.default || $slots.actions">
      <slot />
      <slot name="actions" />
    </EmptyContent>
  </Empty>
</template>
