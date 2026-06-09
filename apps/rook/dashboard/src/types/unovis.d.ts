/**
 * Type declarations for @unovis/vue chart components.
 * Module is loaded dynamically and individual components are exported as named exports.
 */
declare module "@unovis/vue" {
  import type {Component} from "vue";

  interface VisCrosshairProps {
    color?: string;
    width?: number;

    [key: string]: unknown;
  }

  interface VisTooltipProps {
    color?: string;

    [key: string]: unknown;
  }

  export const VisCrosshair: Component<VisCrosshairProps>;
  export const VisTooltip: Component<VisTooltipProps>;
  export const Chart: Component;
  export default {Chart, VisCrosshair, VisTooltip};
}
