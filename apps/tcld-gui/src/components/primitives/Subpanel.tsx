"use client";

import React from "react";
import clsx from "clsx";

/**
 * VSCode-style stacked sub-panel from the tcld workbench mock. Used by
 * the Inspector (Provenance / Variables / Stack / Coverage), the Run
 * Configuration form sections, and any panel that wants the
 * "stacked accordion inside a panel" vibe.
 *
 * The styling is in globals.css under `.tcld-subpanels` / `.tcld-subpanel*`
 * — this primitive provides the React shape that emits that markup. Do not
 * reach for Tailwind here; the CSS already pins height (22px head),
 * typography (9.5px ALL-CAPS letterspaced), colors, and the active
 * accent border-top.
 *
 * For a softer sentence-case collapsible (used in detail dialogs), reach
 * for <Collapsible /> instead.
 *
 * Usage:
 *
 *   <Subpanels>
 *     <Subpanel id="provenance" title="Provenance" meta="2 passes" open onToggle={…}>
 *       <SubpanelRow k="opcode" v={<><span className="kw">invokeWide</span> arity=2</>} />
 *       <SubpanelRow k="source" v="scan:124 · col 9" />
 *     </Subpanel>
 *
 *     <Subpanel id="variables" title="Variables" meta="3 in scope" open active onToggle={…}>
 *       …
 *     </Subpanel>
 *   </Subpanels>
 */

export function Subpanels({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return <div className={clsx("tcld-subpanels", className)}>{children}</div>;
}

/** Extra `data-*` attributes callers can pin to Subpanel DOM (parity/test markers). */
export type SubpanelDataAttrs = Record<`data-${string}`, string | number | undefined>;

export interface SubpanelProps {
  id: string;
  title: string;
  /** Right-aligned meta string (count, status). Sentence case, not ALL-CAPS. */
  meta?: React.ReactNode;
  /** Optional left-side icon glyph. */
  icon?: React.ReactNode;
  /** Right-aligned action buttons (rendered after meta). */
  actions?: React.ReactNode;
  /** Controlled open state. Pair with onToggle. */
  open: boolean;
  onToggle?: () => void;
  /** "Active" — paints the accent border-top + accent title. Use for the
   *  sub-panel that owns the current selection (e.g. the variable scope
   *  matching the cursor). */
  active?: boolean;
  /** Initial body height in px. The user can drag the bottom edge to
   *  resize via native CSS resize:vertical (registered in globals.css). */
  defaultHeight?: number;
  /** Extra data-* markers for the root <section> — stays in the DOM even
   *  when the panel is collapsed, so tests can assert closed state. */
  sectionAttrs?: SubpanelDataAttrs;
  /** Extra data-* markers for the head toggle button. */
  headAttrs?: SubpanelDataAttrs;
  className?: string;
  children: React.ReactNode;
}

export function Subpanel({
  id,
  title,
  meta,
  icon,
  actions,
  open,
  onToggle,
  active = false,
  defaultHeight,
  sectionAttrs,
  headAttrs,
  className,
  children,
}: SubpanelProps) {
  const contentId = `subpanel-${id}-body`;
  return (
    <section
      data-subpanel={id}
      data-open={open ? "true" : "false"}
      data-active={active ? "true" : undefined}
      {...sectionAttrs}
      className={clsx(
        "tcld-subpanel",
        active && "is-active",
        !open && "is-collapsed",
        className,
      )}
    >
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        aria-controls={contentId}
        data-panel-interactive="true"
        {...headAttrs}
        className="tcld-subpanel-head"
      >
        <span className="chev" aria-hidden>
          {open ? "▾" : "▸"}
        </span>
        {icon && (
          <span aria-hidden className="inline-flex h-3 w-3 items-center justify-center">
            {icon}
          </span>
        )}
        <span className="name">{title}</span>
        {meta != null && <span className="meta">{meta}</span>}
        {actions && (
          <span
            data-panel-interactive="true"
            className="actions"
            onClick={(e) => e.stopPropagation()}
          >
            {actions}
          </span>
        )}
      </button>
      {open && (
        <div
          id={contentId}
          data-subpanel-body={id}
          style={defaultHeight ? { height: defaultHeight } : undefined}
          className="tcld-subpanel-body"
        >
          {children}
        </div>
      )}
    </section>
  );
}

/**
 * The canonical key/value row inside a sub-panel body. Renders the `.kv`
 * shape from globals.css: 22px tall, 100px gutter for the key, ellipsizing
 * value column. Don't reach for grid utilities when this shape is what
 * you want — use this.
 */
export function SubpanelRow({
  k,
  v,
  className,
}: {
  k: React.ReactNode;
  v: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={clsx("kv", className)}>
      <span className="k">{k}</span>
      <span className="v">{v}</span>
    </div>
  );
}
