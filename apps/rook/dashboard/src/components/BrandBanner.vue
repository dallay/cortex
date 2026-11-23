<script lang="ts" setup>
</script>

<template>
  <div class="relative overflow-hidden rounded-lg border bg-gradient-to-r from-background to-muted/30 p-6">
    <div class="flex flex-col gap-4">
      <!-- Header -->
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-3">
          <img alt="Rook" class="block h-8 w-auto dark:hidden" src="/rook-lockup-horizontal.svg"/>
          <img alt="Rook" class="hidden h-8 w-auto dark:block" src="/rook-lockup-horizontal-dark.svg"/>
        </div>
        <div class="text-right">
          <p class="text-xs font-medium text-muted-foreground">Cortex Ecosystem</p>
        </div>
      </div>

      <!-- Product Cards -->
      <div class="grid grid-cols-1 gap-3 md:grid-cols-3">
        <!-- Rook -->
        <div class="flex items-center gap-3 rounded-md border bg-card/50 p-3">
          <div class="flex size-8 shrink-0 items-center justify-center">
            <img alt="Rook" class="block size-5 dark:hidden" src="/rook-isotype.svg"/>
            <img alt="Rook" class="hidden size-5 dark:block" src="/rook-isotype-dark.svg"/>
          </div>
          <div>
            <p class="text-sm font-semibold text-foreground">Rook</p>
            <p class="text-xs text-muted-foreground">AI Gateway</p>
          </div>
        </div>

        <!-- Huginn -->
        <div class="flex items-center gap-3 rounded-md border bg-card/50 p-3">
          <div class="flex size-8 shrink-0 items-center justify-center">
            <img alt="Huginn" class="block size-5 dark:hidden" src="/huginn-isotype.svg"/>
            <img alt="Huginn" class="hidden size-5 dark:block" src="/huginn-isotype-dark.svg"/>
          </div>
          <div>
            <p class="text-sm font-semibold text-foreground">Huginn</p>
            <p class="text-xs text-muted-foreground">Local Agent</p>
          </div>
        </div>

        <!-- Muninn -->
        <div class="flex items-center gap-3 rounded-md border bg-card/50 p-3">
          <div class="flex size-8 shrink-0 items-center justify-center">
            <img alt="Muninn" class="block size-5 dark:hidden" src="/muninn-isotype.svg"/>
            <img alt="Muninn" class="hidden size-5 dark:block" src="/muninn-isotype-dark.svg"/>
          </div>
          <div>
            <p class="text-sm font-semibold text-foreground">Muninn</p>
            <p class="text-xs text-muted-foreground">Data Pipeline</p>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
