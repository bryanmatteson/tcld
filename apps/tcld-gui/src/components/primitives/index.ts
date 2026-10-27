/**
 * Barrel export for the canonical workbench primitives.
 *
 * These are the building blocks of every panel — chips, toggles,
 * eyebrows, fields, titlebar buttons. If a panel needs
 * a repeated pattern, extract a primitive,
 * not "hand-roll a one-off."
 *
 * Reach for the design system spec at
 * `tcld-design-system/colors_and_type.css` before adding to this file.
 */
export { Chip } from "./Chip";
export type { ChipSize, ChipTone } from "./Chip";
export { Collapsible } from "./Collapsible";
export { Field } from "./Field";
export { MetricChip } from "./MetricChip";
export { SectionEyebrow } from "./SectionEyebrow";
export type { SectionEyebrowProps } from "./SectionEyebrow";
export { TabBadge } from "./TabBadge";
export { TitlebarButton } from "./TitlebarButton";
export type { TitlebarButtonProps } from "./TitlebarButton";
