"use client";

import clsx from "clsx";
import type { ReactNode } from "react";

/**
 * The canonical ALL-CAPS letterspaced section label — "LAYOUT", "PROFILE",
 * "RECENT FILES", "INSTRUCTION". Replaces ~30 hand-rolled instances of
 * `text-label uppercase tracking-wider text-tcld-fg-muted` scattered
 * across DiagnosticsPanel, DetailPanel, ProvenancePanel, and the welcome
 * screen.
 *
 * Pairs with the `.tcld-section-eyebrow` class in globals.css so non-React
 * surfaces (rich-text, mocks) hit the same vocabulary.
 *
 * Usage:
 *   <SectionEyebrow>Linked evidence</SectionEyebrow>
 *   <SectionEyebrow as="h3">Layout</SectionEyebrow>
 *   <SectionEyebrow trailing={<Chip size="xs" tone="warn">2</Chip>}>Issues</SectionEyebrow>
 */
export interface SectionEyebrowProps {
  children: ReactNode;
  /** Optional right-aligned slot (count chip, key hint, action). */
  trailing?: ReactNode;
  /** Element tag. Defaults to <div>. Use <h2>/<h3> when the eyebrow is the
   *  semantic heading for a region of content. Use <span> when the eyebrow
   *  is rendered inside another inline-context element (e.g. a <summary>). */
  as?: "div" | "span" | "h2" | "h3" | "h4" | "label";
  /** Render with no bottom margin — for cases where the eyebrow shares a
   *  row with another control. */
  inline?: boolean;
  className?: string;
  /** Pass-through for data-* attributes (parity probes, tests). */
  [dataAttr: `data-${string}`]: string | boolean | number | undefined;
}

export function SectionEyebrow({
  children,
  trailing,
  as: Tag = "div",
  inline = false,
  className,
  ...rest
}: SectionEyebrowProps) {
  return (
    <Tag
      data-section-eyebrow
      className={clsx(
        "flex items-center gap-2",
        "font-mono font-bold tracking-[0.08em] uppercase",
        "text-[var(--text-tag)] leading-tight",
        "text-tcld-fg-muted",
        !inline && "mb-1.5",
        className,
      )}
      {...rest}
    >
      <span className="truncate">{children}</span>
      {trailing != null && (
        <span className="ml-auto flex shrink-0 items-center gap-1 normal-case tracking-normal">
          {trailing}
        </span>
      )}
    </Tag>
  );
}
