import { createI18n } from 'vue-i18n'
import type { WritableComputedRef } from 'vue'
import en from '@/locales/en.json'
import es from '@/locales/es.json'

type MessageSchema = typeof en

const STORAGE_KEY = 'rook-locale'

function getInitialLocale(): string {
  const stored = localStorage.getItem(STORAGE_KEY)
  if (stored && ['en', 'es'].includes(stored)) return stored
  const browser = navigator.language.slice(0, 2)
  return ['en', 'es'].includes(browser) ? browser : 'en'
}

export const i18n = createI18n<[MessageSchema], 'en' | 'es'>({
  legacy: false,
  locale: getInitialLocale(),
  fallbackLocale: 'en',
  messages: { en, es },
})

export function setLocale(locale: 'en' | 'es') {
  ;(i18n.global.locale as unknown as WritableComputedRef<string>).value = locale
  localStorage.setItem(STORAGE_KEY, locale)
}