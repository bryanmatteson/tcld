"use client";

import type { ReactNode } from "react";
import { Chip, type ChipTone } from "./Chip";

interface MetricChipProps {
  /** Optional leading icon (lucide node or short glyph). */
  icon?: ReactNode;
  /** Primary label, e.g. "Tcl 8.6" or "1,234 inst". */
  label: ReactNode;
  /** Tone affects color + background. */
  tone?: "neutral" | "accent" | "warn" | "err" | "info";
  className?: string;
  title?: string;
}

/**
 * Thin shim over the unified Chip primitive. Kept for back-compat with
 * existing call sites; new code should reach for `<Chip size="sm" />` directly.
 */
export function MetricChip({ icon, label, tone = "neutral", className, title }: MetricChipProps) {
  return (
    <Chip size="sm" tone={tone as ChipTone} icon={icon} className={className} title={title}>
      {label}
    </Chip>
  );
}
