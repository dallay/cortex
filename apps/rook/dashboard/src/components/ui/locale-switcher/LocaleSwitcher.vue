<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { setLocale } from '@/i18n'
import type { AcceptableValue } from 'reka-ui'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

const { locale } = useI18n()

const locales = [
  { code: 'en', label: 'English' },
  { code: 'es', label: 'Español' },
] as const

function handleChange(value: AcceptableValue) {
  if (typeof value === 'string') {
    setLocale(value as 'en' | 'es')
  }
}
</script>

<template>
  <Select :model-value="locale" @update:model-value="handleChange">
    <SelectTrigger class="h-8 w-[130px]">
      <SelectValue />
    </SelectTrigger>
    <SelectContent>
      <SelectItem v-for="loc in locales" :key="loc.code" :value="loc.code">
        {{ loc.label }}
      </SelectItem>
    </SelectContent>
  </Select>
</template>
