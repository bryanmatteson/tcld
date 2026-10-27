"use client";

import { useEffect, useState } from "react";

/**
 * Keeps an overlay mounted for `exitMs` after `visible` turns false so a CSS
 * exit animation can play (`data-state="closed"`), then unmounts it.
 */
export function usePresence(visible: boolean, exitMs: number): boolean {
  const [mounted, setMounted] = useState(visible);
  if (visible && !mounted) setMounted(true);

  useEffect(() => {
    if (visible || !mounted) return;
    const timer = window.setTimeout(() => setMounted(false), exitMs);
    return () => window.clearTimeout(timer);
  }, [exitMs, mounted, visible]);

  return mounted;
}
