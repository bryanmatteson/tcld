// ─── App capability handshake state ──────────────────────────────
// Single source of truth for the backend capability handshake
// (`get_app_capabilities`). A Rust backend built without the `debugger`
// feature reports `debugger_available: false`; the UI consumes this flag to
// degrade explicitly (disabled run controls, unavailable panel states)
// instead of surfacing raw IPC "command not found" errors.
//
// The default is "available" so normal builds render with zero flicker —
// the flag only flips once the handshake resolves with an explicit false.

import { getAppCapabilities } from "@/lib/tauri-api";

/**
 * Canonical user-facing copy for every surface that degrades when the
 * backend lacks the debugger feature (tooltips, empty states, refusal
 * messages). Keep the string identical everywhere so users read one
 * consistent explanation.
 */
export const DEBUGGER_UNAVAILABLE_MESSAGE = "This build does not include the debugger.";

export interface AppCapabilitiesState {
  debuggerAvailable: boolean;
}

const DEFAULT_STATE: AppCapabilitiesState = { debuggerAvailable: true };

let state: AppCapabilitiesState = DEFAULT_STATE;
let loadPromise: Promise<void> | null = null;
const listeners = new Set<() => void>();

function setState(next: AppCapabilitiesState): void {
  if (next.debuggerAvailable === state.debuggerAvailable) return;
  state = next;
  for (const listener of listeners) listener();
}

/** Subscribe to capability changes (useSyncExternalStore contract). */
export function subscribeAppCapabilities(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Current capability snapshot (stable reference until a value changes). */
export function getAppCapabilitiesSnapshot(): AppCapabilitiesState {
  return state;
}

/**
 * Server snapshot for useSyncExternalStore — prerendered markup must match
 * the optimistic client default so normal builds hydrate without flicker.
 */
export function getAppCapabilitiesServerSnapshot(): AppCapabilitiesState {
  return DEFAULT_STATE;
}

/** Synchronous read for non-React code paths (store actions, guards). */
export function isDebuggerAvailable(): boolean {
  return state.debuggerAvailable;
}

/**
 * Resolve the handshake once per app lifetime. Multiple mount points may
 * call this (the provider tree remounts across window modes); only the
 * first call performs IPC. `getAppCapabilities` never rejects — backends
 * without the command resolve to the historical default (available).
 */
export function loadAppCapabilities(): Promise<void> {
  loadPromise ??= getAppCapabilities().then((capabilities) => {
    setState({ debuggerAvailable: capabilities.debugger_available });
  });
  return loadPromise;
}

/** Test-only: reset module state so unit tests can re-run the handshake. */
export function __resetAppCapabilitiesForTests(): void {
  state = DEFAULT_STATE;
  loadPromise = null;
}
