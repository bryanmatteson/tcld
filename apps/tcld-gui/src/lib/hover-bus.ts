"use client";

import type { Location } from "@/lib/location-resolve";

export type HoverLocation = Partial<Location>;
type HoverListener = (location: HoverLocation | null) => void;

class HoverBus {
  private listeners = new Set<HoverListener>();
  private current: HoverLocation | null = null;
  private rafId: number | null = null;

  subscribe(listener: HoverListener): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  emit(location: HoverLocation | null) {
    this.current = location;
    if (this.rafId != null) return;

    this.rafId = requestAnimationFrame(() => {
      this.rafId = null;
      for (const listener of this.listeners) {
        listener(this.current);
      }
    });
  }

  peek(): HoverLocation | null {
    return this.current;
  }
}

export const hoverBus = new HoverBus();
