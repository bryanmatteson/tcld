"use client";

import { useState, useCallback } from "react";
import type { ContextMenuItem } from "@/components/overlays/ContextMenu";

interface ContextMenuState {
  items: ContextMenuItem[];
  position: { x: number; y: number };
}

export function useContextMenu() {
  const [state, setState] = useState<ContextMenuState | null>(null);

  const showMenu = useCallback(
    (e: React.MouseEvent, items: ContextMenuItem[]) => {
      e.preventDefault();
      e.stopPropagation();
      setState({ items, position: { x: e.clientX, y: e.clientY } });
    },
    [],
  );

  const closeMenu = useCallback(() => {
    setState(null);
  }, []);

  return {
    menuState: state,
    showMenu,
    closeMenu,
  };
}
