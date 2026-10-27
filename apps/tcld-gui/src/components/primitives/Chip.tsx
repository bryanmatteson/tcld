"use client";

import clsx from "clsx";
import type { HTMLAttributes, ReactNode } from "react";
import { compactCount } from "@/lib/format";

/**
 * Single Chip family that replaces MetricChip, TabBadge, and the StatusBar
 * chip. Three sizes map to three roles:
 *
 *   - xs (16px high, no border, label-size text): in-tab badges, count
 *     pills that sit inside another bordered container.
 *   - sm (20px high, bordered, label-size text): file-tab metrics, breadcrumb
 *     pins, panel-header counts.
 *   - md (24px high, bordered, meta-size text): status-bar tags, sticky
 *     anchored chips.
 *
 * Tones are semantic: neutral / accent / warn / err / info / success / group.
 * Each variant pairs a faint background tint with an AA-passing foreground.
 */
export type ChipSize = "xs" | "sm" | "md";
export type ChipTone =
  | "neutral"
  | "accent"
  | "warn"
  | "err"
  | "info"
  | "success"
  | "group";

const SIZE: Record<ChipSize, string> = {
  // 16px — drops the border so the chip dissolves into its parent surface.
  // rounded uses --radius-badge (3px) so the family stays in sync with the
  // canonical tcld-design-system spec; never hardcode the radius here.
  xs: "h-4 min-w-4 px-1 text-label rounded-[var(--radius-badge)]",
  // 20px — bordered chip; the workhorse for metadata and counts.
  sm: "h-5 px-1.5 text-label rounded-[var(--radius-badge)] gap-1 border",
  // 24px — bordered chip with extra padding for status-bar use.
  md: "h-6 px-2 text-meta rounded-[var(--radius-badge)] gap-1.5 border",
};

const TONE_BORDERED: Record<ChipTone, string> = {
  neutral: "border-tcld-border-dim bg-tcld-bg-2 text-tcld-text-inactive",
  accent: "border-tcld-accent/40 bg-tcld-accent/10 text-tcld-accent",
  warn: "border-tcld-warning/40 bg-tcld-warning/10 text-tcld-warning",
  err: "border-tcld-error/40 bg-tcld-error/10 text-tcld-error",
  info: "border-tcld-variable/40 bg-tcld-variable/10 text-tcld-variable",
  success: "border-tcld-success/40 bg-tcld-success/10 text-tcld-success",
  group: "border-tcld-group/40 bg-tcld-group/10 text-tcld-group",
};

// Borderless tones for size=xs. Keep the foreground but drop the bg-tint so
// the chip reads as a quiet count, not a competing pill.
const TONE_QUIET: Record<ChipTone, string> = {
  neutral: "text-tcld-text-inactive",
  accent: "text-tcld-accent",
  warn: "text-tcld-warning",
  err: "text-tcld-error",
  info: "text-tcld-variable",
  success: "text-tcld-success",
  group: "text-tcld-group",
};

interface ChipProps extends HTMLAttributes<HTMLSpanElement> {
  size?: ChipSize;
  tone?: ChipTone;
  icon?: ReactNode;
  /** When true and tone="neutral", uses the inactive ramp (for unfocused tabs). */
  muted?: boolean;
  /** Numeric values are passed through compactCount (1234 → "1.2k"). */
  children: ReactNode | number;
  className?: string;
  title?: string;
}

export function Chip({
  size = "sm",
  tone = "neutral",
  icon,
  muted = false,
  children,
  className,
  title,
  ...rest
}: ChipProps) {
  const display = typeof children === "number" ? compactCount(children) : children;
  if (display == null || display === "") return null;

  const toneClasses = size === "xs" ? TONE_QUIET[tone] : TONE_BORDERED[tone];
  const mutedClass = muted && tone === "neutral" ? "opacity-80" : "";

  return (
    <span
      // Forward remaining attributes (aria-*, data-* parity markers like
      // data-diagnostics-count / data-tab-badge) onto the rendered span.
      {...rest}
      title={title}
      className={clsx(
        "inline-flex items-center justify-center font-mono leading-none whitespace-nowrap tabular-nums",
        SIZE[size],
        toneClasses,
        mutedClass,
        className,
      )}
    >
      {icon && (
        <span
          className={clsx(
            "flex shrink-0 items-center justify-center",
            size === "xs" ? "h-2.5 w-2.5" : size === "sm" ? "h-3 w-3" : "h-3.5 w-3.5",
          )}
          aria-hidden
        >
          {icon}
        </span>
      )}
      <span className="truncate">{display}</span>
    </span>
  );
}
