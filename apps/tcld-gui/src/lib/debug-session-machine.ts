import type { DebugSessionState } from "@/types";

const ACTIVE_STATES = new Set<DebugSessionState>([
  "launching",
  "connected",
  "running",
  "pausing",
  "paused",
]);

const RUNTIME_DATA_STATES = new Set<DebugSessionState>(["paused", "running"]);

const BACKEND_SYNC_STATES = new Set<DebugSessionState>([
  "idle",
  "launching",
  "connected",
  "running",
  "paused",
  "terminated",
]);

export function isActiveDebugSession(state: DebugSessionState): boolean {
  return ACTIVE_STATES.has(state);
}

export function canLaunchDebugSession(state: DebugSessionState): boolean {
  return state === "idle" || state === "terminated";
}

export function canStopDebugSession(state: DebugSessionState): boolean {
  return state !== "idle" && state !== "terminated";
}

export function acceptsReadyDebugSession(state: DebugSessionState): boolean {
  return state === "launching" || state === "connected";
}

export function acceptsRuntimeDataDebugSession(state: DebugSessionState): boolean {
  return RUNTIME_DATA_STATES.has(state);
}

export function normalizeBackendDebugState(raw: string): DebugSessionState | null {
  if (BACKEND_SYNC_STATES.has(raw as DebugSessionState)) {
    return raw as DebugSessionState;
  }
  return null;
}
