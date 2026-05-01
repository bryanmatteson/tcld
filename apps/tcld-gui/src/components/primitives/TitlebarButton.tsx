"use client";

import React from "react";
import clsx from "clsx";

/**
 * Icon-only button for panel titlebars and toolbars. Renders the
 * canonical 22×22 hit area with hover wash + pressed accent state and
 * a CSS-only delayed tooltip via `data-tip`.
 *
 * Replaces ad-hoc icon buttons that hand-rolled their own hover state
 * and `title` attribute (which has a 1.5s OS-level delay and no styling).
 *
 * The `data-tip` pattern intentionally degrades to native `title` for
 * screen readers — both are set so assistive tech still announces the
 * label.
 *
 * Usage:
 *   <TitlebarButton tip="Step over (F10)" onClick={stepOver}>
 *     <StepForward size={12} />
 *   </TitlebarButton>
 *
 *   <TitlebarButton tip="Pin" pressed={isPinned} onClick={togglePin}>
 *     ⊕
 *   </TitlebarButton>
 */
export interface TitlebarButtonProps
  extends Omit<React.ButtonHTMLAttributes<HTMLButtonElement>, "title"> {
  /** Tooltip copy. Shown via `data-tip` (0.3s delay) and mirrored to
   *  `title` for screen readers / OS tooltips. */
  tip: string;
  /** Pressed / toggled-on state — paints the accent wash. */
  pressed?: boolean;
  /** Tighter 18×18 variant for nested toolbars (sub-panel headers, the
   *  inline tabs row). Defaults to "md" (22×22). */
  size?: "sm" | "md";
  /** Tooltip placement. Defaults to "bottom"; pass "top" for buttons
   *  near the bottom edge of the workspace. */
  tipSide?: "top" | "bottom" | "left" | "right";
  children: React.ReactNode;
}

export const TitlebarButton = React.forwardRef<
  HTMLButtonElement,
  TitlebarButtonProps
>(function TitlebarButton(
  { tip, pressed = false, size = "md", tipSide = "bottom", className, children, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type="button"
      data-tip={tip}
      data-tip-side={tipSide}
      data-pressed={pressed ? "true" : undefined}
      title={tip}
      aria-label={tip}
      aria-pressed={pressed}
      className={clsx(
        "relative inline-flex items-center justify-center",
        "rounded-[var(--radius-badge)] border border-transparent",
        "text-tcld-text-inactive font-mono",
        "transition-colors duration-100 ease-out",
        "hover:bg-tcld-bg-3 hover:border-tcld-border-dim hover:text-tcld-fg",
        // text-meta = 11px for compact toolbar buttons; text-code = 12px for
        // primary panel-titlebar buttons (icon-only at 22×22). Both come from
        // the canonical type scale — never reach for arbitrary px here.
        size === "sm" ? "h-[18px] min-w-[18px] px-1 text-meta" : "h-[22px] min-w-[22px] px-1 text-code",
        pressed &&
          "bg-tcld-accent/12 border-tcld-accent/40 text-tcld-accent hover:bg-tcld-accent/18 hover:text-tcld-accent",
        "disabled:opacity-40 disabled:cursor-not-allowed disabled:hover:bg-transparent disabled:hover:border-transparent disabled:hover:text-tcld-text-inactive",
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
});
