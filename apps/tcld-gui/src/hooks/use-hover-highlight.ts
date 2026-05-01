"use client";

import { useEffect, type RefObject } from "react";
import { hoverBus, type HoverLocation } from "@/lib/hover-bus";

export function useHoverHighlight<T extends Element>({
  containerRef,
  rowSelector,
  keyAttribute = "data-idx",
  getKey,
}: {
  containerRef: RefObject<T | null>;
  rowSelector: string;
  keyAttribute?: string;
  getKey: (location: HoverLocation) => number | string | null;
}) {
  useEffect(() => {
    let prevEl: Element | null = null;

    return hoverBus.subscribe((location) => {
      if (prevEl) {
        prevEl.removeAttribute("data-hover");
        prevEl = null;
      }
      if (!location) return;

      const key = getKey(location);
      if (key == null) return;

      const container = containerRef.current;
      if (!container) return;

      const el = container.querySelector(
        `${rowSelector}[${keyAttribute}="${String(key)}"]`,
      );
      if (!el) return;

      el.setAttribute("data-hover", "");
      prevEl = el;
    });
  }, [containerRef, getKey, keyAttribute, rowSelector]);
}
