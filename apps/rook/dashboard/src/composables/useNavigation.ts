/**
 * Navigation composable with lazy-loaded icons.
 *
 * Icons are loaded on-demand via defineAsyncComponent.
 * Each icon is imported from its individual file for proper tree-shaking.
 *
 * To add a new icon:
 * 1. Add it to iconRegistry with path to its individual .mjs file
 * 2. Use its registry key in src/config/navigation.ts
 */

import type {Component} from "vue";
import {computed, defineAsyncComponent} from "vue";
import {useI18n} from "vue-i18n";

import {navigationConfig} from "@/config/navigation";

// Lazy-loaded icon registry — each icon imported from its individual file
// Key must match the `icon` field in navigation.ts config
// Icons are default exports from their individual .mjs files
const iconRegistry: Record<string, () => Promise<Component>> = {
  CircleDot: () =>
    import("@lucide/vue/dist/esm/icons/circle-dot.mjs").then((m) => m.default),
  FileSliders: () =>
    import("@lucide/vue/dist/esm/icons/file-sliders.mjs").then(
      (m) => m.default,
    ),
  Globe: () =>
    import("@lucide/vue/dist/esm/icons/globe.mjs").then((m) => m.default),
  Home: () =>
    import("@lucide/vue/dist/esm/icons/house.mjs").then((m) => m.default),
  Key: () =>
    import("@lucide/vue/dist/esm/icons/key.mjs").then((m) => m.default),
  LifeBuoy: () =>
    import("@lucide/vue/dist/esm/icons/life-buoy.mjs").then((m) => m.default),
  ListOrdered: () =>
    import("@lucide/vue/dist/esm/icons/list-ordered.mjs").then(
      (m) => m.default,
    ),
  Send: () =>
    import("@lucide/vue/dist/esm/icons/send.mjs").then((m) => m.default),
  Settings: () =>
    import("@lucide/vue/dist/esm/icons/settings.mjs").then((m) => m.default),
};

function resolveIcon(name: string): Component {
  const loader = iconRegistry[name];
  if (!loader) {
    console.warn(
      `[useNavigation] Icon "${name}" not found in registry. Falling back to Home.`,
    );
    return defineAsyncComponent(iconRegistry.Home!);
  }
  return defineAsyncComponent(loader);
}

export interface ResolvedNavItem {
  title: string;
  url: string;
  icon: Component;
  isActive?: boolean;
  items?: { title: string; url: string }[];
}

export function useNavigation() {
  const {t} = useI18n();

  const navMain = computed<ResolvedNavItem[]>(() =>
    navigationConfig.main.map((item) => ({
      title: t(item.titleKey),
      url: item.url,
      icon: resolveIcon(item.icon),
      isActive: item.isActive,
      items: item.items?.map((sub) => ({
        title: t(sub.titleKey),
        url: sub.url,
      })),
    })),
  );

  const navSecondary = computed<ResolvedNavItem[]>(() =>
    navigationConfig.secondary.map((item) => ({
      title: t(item.titleKey),
      url: item.url,
      icon: resolveIcon(item.icon),
    })),
  );

  return {
    navMain,
    navSecondary,
  };
}
