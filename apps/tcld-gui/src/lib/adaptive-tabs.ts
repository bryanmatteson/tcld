export type AdaptiveTabMode = "full" | "ellipsis" | "iconCount" | "iconOnly";

export interface AdaptiveTabInput {
  label: string;
  active?: boolean;
  badge?: string | number | null;
  dot?: boolean;
}

const CHAR_WIDTH = 6.5;
const TAB_PADDING = 16;
const INNER_GAP = 6;
const ICON_WIDTH = 11;
const DOT_WIDTH = 6;
const BADGE_PADDING = 4;
const OUTER_GAP = 4;
// Reserved on the right for the panel-header-actions cluster (the actions
// like add-view / maximize / close sit immediately after the tab strip).
// Bumped from 20px because the actions cluster is now 3 buttons × 28px + gaps.
const HEADER_PADDING = 36;
// Close X button width (16px hit area + small breathing room) — visible on
// the active tab always, and on hovered inactive tabs.  Was missing from
// per-tab width estimates, which let tabs overflow into the actions area.
const CLOSE_X_WIDTH = 18;

function estimateBadgeWidth(badge: string | number): number {
  return Math.ceil(String(badge).length * CHAR_WIDTH + BADGE_PADDING);
}

export function estimateAdaptiveTabWidth(tab: AdaptiveTabInput, mode: AdaptiveTabMode): number {
  let width = TAB_PADDING + ICON_WIDTH;

  if (mode === "full" || mode === "ellipsis") {
    const label = mode === "ellipsis"
      ? `${tab.label.slice(0, Math.ceil(tab.label.length / 2))}…`
      : tab.label;
    width += INNER_GAP + label.length * CHAR_WIDTH;
  }

  if ((mode === "full" || mode === "ellipsis" || mode === "iconCount") && tab.badge != null) {
    width += INNER_GAP + estimateBadgeWidth(tab.badge);
  }

  if (tab.dot) {
    width += INNER_GAP + DOT_WIDTH;
  }

  // Close X is rendered when the tab has a label slot (full/ellipsis), and
  // is always rendered for the active tab even in tighter modes — so reserve
  // its width in those cases.
  if (mode === "full" || mode === "ellipsis" || tab.active) {
    width += INNER_GAP + CLOSE_X_WIDTH;
  }

  return Math.ceil(width + OUTER_GAP);
}

export function adaptiveTabsHardMinimum(tabs: AdaptiveTabInput[]): number {
  return tabs.reduce((sum, tab) => sum + estimateAdaptiveTabWidth(tab, "iconOnly"), HEADER_PADDING);
}

export function fitAdaptiveTabs(tabs: AdaptiveTabInput[], available: number): AdaptiveTabMode[] {
  const modes: AdaptiveTabMode[] = tabs.map(() => "full");
  const fits = () =>
    tabs.reduce((sum, tab, index) => sum + estimateAdaptiveTabWidth(tab, modes[index]), HEADER_PADDING) <= available;

  if (fits()) return [...modes];

  for (let index = 0; index < tabs.length; index += 1) {
    if (!tabs[index].active) modes[index] = "iconCount";
    if (fits()) return [...modes];
  }

  for (let index = 0; index < tabs.length; index += 1) {
    if (!tabs[index].active) modes[index] = "iconOnly";
    if (fits()) return [...modes];
  }

  for (let index = 0; index < tabs.length; index += 1) {
    if (tabs[index].active) modes[index] = "ellipsis";
    if (fits()) return [...modes];
  }

  for (let index = 0; index < tabs.length; index += 1) {
    if (tabs[index].active) modes[index] = "iconCount";
    if (fits()) return [...modes];
  }

  return tabs.map(() => "iconOnly");
}

export function renderAdaptiveTabLabel(label: string, mode: AdaptiveTabMode): string {
  if (mode !== "ellipsis") return label;
  return `${label.slice(0, Math.ceil(label.length / 2))}…`;
}
