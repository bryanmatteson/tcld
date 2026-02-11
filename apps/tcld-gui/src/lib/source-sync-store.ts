"use client";

import { create } from "zustand";

interface SourceSyncState {
  linked: boolean;
  setLinked: (linked: boolean) => void;
  toggleLinked: () => void;
}

export const useSourceSyncStore = create<SourceSyncState>((set) => ({
  linked: true,
  setLinked: (linked) => set({ linked }),
  toggleLinked: () => set((state) => ({ linked: !state.linked })),
}));
