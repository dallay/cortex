/**
 * Type declarations for @lucide/vue individual icon imports.
 * These are loaded dynamically via their .mjs files for tree-shaking.
 */
declare module "@lucide/vue/dist/esm/icons/*" {
  import type {Component} from "vue";

  const icon: Component;
  export default icon;
}
