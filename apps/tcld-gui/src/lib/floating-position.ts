import { useLayoutEffect, useState, type RefObject } from "react";

export type FloatingPlacement = "bottom-start" | "bottom-end" | "top-start" | "top-end";

export interface FloatingRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export interface FloatingOptions {
  placement?: FloatingPlacement;
  offset?: number;
  margin?: number;
}

export interface FloatingPosition {
  left: number;
  top: number;
  maxHeight: number;
  ready: boolean;
}

const DEFAULT_OPTIONS: Required<FloatingOptions> = {
  placement: "bottom-start",
  offset: 4,
  margin: 8,
};

export function computeFloatingPosition(
  anchor: FloatingRect,
  floating: Pick<FloatingRect, "width" | "height">,
  viewport: Pick<FloatingRect, "width" | "height">,
  options: FloatingOptions = {},
): FloatingPosition {
  const { placement, offset, margin } = { ...DEFAULT_OPTIONS, ...options };
  const preferBottom = placement.startsWith("bottom");
  const alignEnd = placement.endsWith("end");
  const bottomTop = anchor.top + anchor.height + offset;
  const topTop = anchor.top - floating.height - offset;
  const spaceBelow = viewport.height - margin - bottomTop;
  const spaceAbove = anchor.top - margin - offset;
  const useTop = preferBottom
    ? floating.height > spaceBelow && spaceAbove > spaceBelow
    : !(floating.height > spaceAbove && spaceBelow > spaceAbove);

  const rawTop = useTop ? topTop : bottomTop;
  const rawLeft = alignEnd ? anchor.left + anchor.width - floating.width : anchor.left;
  const maxLeft = Math.max(margin, viewport.width - floating.width - margin);
  const left = clamp(rawLeft, margin, maxLeft);
  const top = clamp(rawTop, margin, Math.max(margin, viewport.height - floating.height - margin));
  const availableHeight = useTop ? anchor.top - margin - offset : viewport.height - bottomTop - margin;

  return {
    left,
    top,
    maxHeight: Math.max(80, Math.min(floating.height, availableHeight > 0 ? availableHeight : viewport.height - margin * 2)),
    ready: true,
  };
}

export function useAnchoredFloatingPosition(
  open: boolean,
  anchorRef: RefObject<HTMLElement | null>,
  floatingRef: RefObject<HTMLElement | null>,
  options: FloatingOptions = {},
): FloatingPosition {
  const [position, setPosition] = useState<FloatingPosition>({ left: 0, top: 0, maxHeight: 0, ready: false });
  const { placement, offset, margin } = { ...DEFAULT_OPTIONS, ...options };

  useLayoutEffect(() => {
    if (!open) {
      return;
    }

    const update = () => {
      const anchor = anchorRef.current;
      const floating = floatingRef.current;
      if (!anchor || !floating) return;
      setPosition(computeFloatingPosition(
        anchor.getBoundingClientRect(),
        floating.getBoundingClientRect(),
        { width: window.innerWidth, height: window.innerHeight },
        { placement, offset, margin },
      ));
    };

    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
    };
  }, [anchorRef, floatingRef, margin, offset, open, placement]);

  return position;
}

function clamp(value: number, min: number, max: number): number {
  if (max < min) return min;
  return Math.min(Math.max(value, min), max);
}
