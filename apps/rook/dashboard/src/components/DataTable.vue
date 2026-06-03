<script setup lang="ts">
import {
  Table,
} from '@/components/ui/table'
</script>

<template>
  <div class="rounded-md border">
    <Table>
      <slot />
    </Table>
  </div>
</template>
