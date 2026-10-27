"use client";

import React from "react";
import clsx from "clsx";

interface FieldProps {
  /** Left-column label. */
  label: React.ReactNode;
  /** Right-column value. */
  value: React.ReactNode;
  /** Width of the label column. Defaults to 96px (compact KV). */
  labelWidth?: number;
  /** Render the value as code (monospace). Default true — most consumers
   *  are inspectors over bytecode-derived data. */
  mono?: boolean;
  /** Visual tone of the value. Maps to an accent color. */
  tone?: "default" | "string" | "number" | "keyword" | "variable" | "muted" | "err";
  /** Allow multiline value (prose). */
  wrap?: boolean;
  className?: string;
  title?: string;
  /** Pass-through for data-* attributes (parity probes, tests). */
  [dataAttr: `data-${string}`]: string | boolean | number | undefined;
}

const TONE: Record<NonNullable<FieldProps["tone"]>, string> = {
  default: "text-tcld-fg",
  string: "text-tcld-string",
  number: "text-tcld-number",
  keyword: "text-tcld-keyword font-medium",
  variable: "text-tcld-variable",
  muted: "text-tcld-fg-dim",
  err: "text-tcld-error",
};

/**
 * Single label/value row primitive. Replaces the per-panel `KvRow` in
 * DetailPanel and the bespoke `Field` in CommandResolverPanel — same shape,
 * one source of truth.
 */
export function Field({
  label,
  value,
  labelWidth = 96,
  mono = true,
  tone = "default",
  wrap = false,
  className,
  title,
  ...rest
}: FieldProps) {
  return (
    <div
      {...rest}
      title={title}
      // Row height pinned to --row-h (compact:22, regular:26, comfy:30) so
      // the inspector field row scales with the workspace density mode
      // instead of always painting at 21px. Matches the Inspector spec
      // in the canonical workbench mock.
      className={clsx(
        "flex items-baseline gap-2 leading-[var(--row-h)] py-px min-h-[var(--row-h)]",
        "text-meta",
        className,
      )}
    >
      <span
        className="flex-shrink-0 font-ui text-tcld-fg-muted whitespace-nowrap"
        style={{ width: labelWidth }}
      >
        {label}
      </span>
      <span
        className={clsx(
          "min-w-0 flex-1 text-meta",
          mono ? "font-mono" : "font-ui",
          wrap ? "break-words leading-normal py-1" : "truncate",
          TONE[tone],
        )}
      >
        {value}
      </span>
    </div>
  );
}

/**
 * Wraps a list of `<Field>`s in a consistent grid container with vertical
 * rhythm. Use inside a Collapsible body when you need a stack of KV rows.
 */
export function FieldList({
  className,
  children,
}: {
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <div className={clsx("flex flex-col gap-px py-1", className)}>{children}</div>
  );
}
