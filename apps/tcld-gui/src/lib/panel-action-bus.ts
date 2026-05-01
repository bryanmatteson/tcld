"use client";

type PanelActionHandler = () => void;

const handlers = new Map<string, Set<PanelActionHandler>>();

export function subscribePanelAction(actionId: string, handler: PanelActionHandler): () => void {
  const existing = handlers.get(actionId) ?? new Set<PanelActionHandler>();
  existing.add(handler);
  handlers.set(actionId, existing);
  return () => {
    existing.delete(handler);
    if (existing.size === 0) handlers.delete(actionId);
  };
}

export function dispatchPanelAction(actionId: string): boolean {
  const matching = handlers.get(actionId);
  if (!matching || matching.size === 0) return false;
  for (const handler of [...matching]) handler();
  return true;
}
