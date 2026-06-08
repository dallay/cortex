<script setup lang="ts">
/**
 * ProviderIcon — renders the branded icon for a given `ProviderKind`.
 *
 * Icon resolution strategy (in priority order):
 *
 *   1. **Iconify bundle** — for the 4 kinds that have an official mark
 *      in Simple Icons (`simple-icons` set from @iconify-json/simple-icons).
 *      Icons are bundled at build time via Vite tree-shaking — zero
 *      runtime CDN requests, fully offline-safe.
 *
 *      Coverage: openai, anthropic, ollama, gemini (as googlegemini).
 *
 *   2. **Local SVG/PNG** — for kinds not yet in Simple Icons (groq,
 *      ollama-cloud). Static files in `public/providers/<iconFile>`,
 *      served as `<img>` with lazy/eager loading and CLS-safe dimensions.
 *
 *   3. **Fallback** — Lucide `Server` icon + dev console.warn when a
 *      local file fails to load (404 or network error).
 *
 * Accessibility:
 *   - `decorative=true` (default) → `aria-hidden="true"`, `alt=""`
 *     Use when the surrounding text already names the provider.
 *   - `decorative=false` → `role="img"`, `aria-label="<DisplayName>"`
 *     Use when the icon stands alone.
 *
 * Performance:
 *   - Iconify icons render as inline `<svg>` — no extra HTTP request,
 *     no loading strategy needed, always crisp at any size.
 *   - Local `<img>` uses `loading="lazy"` by default. Pass `loading="eager"`
 *     for above-the-fold LCP candidates (e.g. detail page header).
 *   - Both paths declare explicit `width`/`height` to prevent CLS.
 */
import {computed, ref} from 'vue'
import {Icon} from '@iconify/vue'
import {addCollection} from '@iconify/vue'
import simpleIcons from '@iconify-json/simple-icons/icons.json'
import Server from '@lucide/vue/dist/esm/icons/server.mjs'
import {PROVIDER_KINDS, type ProviderKind} from '@/config/providerCatalog'

// Register the full Simple Icons set once at module load time.
// Vite tree-shakes unused icons in production — only referenced icons
// end up in the bundle (via the rollup-plugin-purge-iconify or by
// default Vite static analysis of the `icon` prop string literals).
addCollection(simpleIcons as Parameters<typeof addCollection>[0])

/**
 * Maps a ProviderKind to its Simple Icons identifier, or null if
 * the kind isn't yet in Simple Icons. When null, ProviderIcon falls
 * back to the local public/providers/<iconFile> asset.
 */
const ICONIFY_MAP: Partial<Record<ProviderKind, string>> = {
  openai: 'simple-icons:openai',
  anthropic: 'simple-icons:anthropic',
  ollama: 'simple-icons:ollama',
  'ollama-cloud': 'simple-icons:ollama', // same brand, managed variant
  gemini: 'simple-icons:googlegemini',
  // groq — not in simple-icons yet → falls back to local groq.svg
} as const

// ---------------------------------------------------------------------------

const props = withDefaults(
  defineProps<{
    /** Provider kind — must be one of the 6 known kinds. */
    kind: ProviderKind
    /**
     * Loading strategy for the local `<img>` fallback path only.
     * Use `'eager'` for above-the-fold LCP candidates (e.g. detail page
     * header). Iconify SVGs are inline — loading strategy doesn't apply.
     */
    loading?: 'eager' | 'lazy'
    /** Rendered width in px. Declared explicitly to prevent CLS. */
    width?: number | string
    /** Rendered height in px. Declared explicitly to prevent CLS. */
    height?: number | string
    /**
     * When `true` (default), the icon is purely decorative.
     * Renders `aria-hidden="true"`.
     *
     * When `false`, the icon carries meaning on its own.
     * Renders `role="img"` with `aria-label="<DisplayName>"`.
     */
    decorative?: boolean
  }>(),
  {
    loading: 'lazy',
    width: 32,
    height: 32,
    decorative: true,
  },
)

const localFailed = ref(false)

const entry = computed(() => PROVIDER_KINDS.find((p) => p.kind === props.kind) ?? null)

/** Simple Icons identifier for this kind, or null to use the local asset. */
const iconifyIcon = computed(() => ICONIFY_MAP[props.kind] ?? null)

/** Local asset path — only used when iconifyIcon is null. */
const localSrc = computed(() =>
  entry.value && !iconifyIcon.value ? `/providers/${entry.value.iconFile}` : null,
)

const displayName = computed(() => {
  if (!entry.value) return props.kind
  // Derive a readable name from the i18n key without pulling in the
  // i18n composable (avoids a dependency on the app's i18n instance).
  // The detail view has the properly translated name in its own scope.
  const parts = entry.value.displayNameKey.split('.')
  return parts[parts.length - 2] ?? props.kind
})

const iconSize = computed(() => Number(props.width) || 32)

const ariaAttrs = computed(() =>
  props.decorative
    ? ({'aria-hidden': 'true'} as const)
    : ({role: 'img', 'aria-label': displayName.value} as const),
)

function onLocalError() {
  localFailed.value = true
  if (import.meta.env.DEV) {
    console.warn(
      `[ProviderIcon] Failed to load local icon for "${props.kind}": ${localSrc.value}. ` +
      `Check that public/providers/${entry.value?.iconFile} exists.`,
    )
  }
}
</script>

<template>
  <!-- Strategy 1: Iconify bundle (openai, anthropic, ollama, gemini) -->
  <Icon
    v-if="iconifyIcon"
    :icon="iconifyIcon"
    :width="width"
    :height="height"
    v-bind="ariaAttrs"
    class="shrink-0"
  />

  <!-- Strategy 2: local SVG/PNG (groq, ollama-cloud) — and fallback sentinel -->
  <template v-else-if="localSrc && !localFailed">
    <img
      :src="localSrc"
      :width="width"
      :height="height"
      :loading="loading"
      decoding="async"
      :alt="decorative ? '' : displayName"
      v-bind="ariaAttrs"
      class="object-contain shrink-0"
      @error="onLocalError"
    />
  </template>

  <!-- Strategy 3: Lucide Server fallback -->
  <Server
    v-else
    :size="iconSize"
    v-bind="ariaAttrs"
    class="text-muted-foreground shrink-0"
  />
</template>
