// ─── tcld Data Store ─────────────────────────────────────────────
// Central state for loaded bytecode data. Wraps Tauri API calls
// and provides data to all components via React context.

"use client";

import { createContext, useContext, useState, useCallback, useMemo, useRef, useEffect, type ReactNode } from "react";
import type {
  InstructionDetail,
} from "@/types";
import { loadFile as apiLoadFile, loadText as apiLoadText } from "@/lib/tauri-api";
import { invalidate as invalidateQuery, query } from "@/lib/tauri-query";
import {
  buildInstructionDetailFromData,
  buildSessionDocument,
  closeSessionDocument,
  createSessionState,
  selectActiveDocument,
  tcldDataFromLoadResult,
  upsertSessionDocument,
  updateSessionDocumentWorkspaceState,
  type DocumentWorkspaceState,
  type SessionState,
  type TcldData,
} from "@/lib/store-data";
export type { TcldData } from "@/lib/store-data";

interface StoreState {
  loaded: boolean;
  loading: boolean;
  error: string | null;
  data: TcldData | null;
  detailCache: Map<number, InstructionDetail>;
  session: SessionState;
}

interface StoreActions {
  loadFile: (path: string) => Promise<TcldData | null>;
  /** Load a Tcl source snippet directly from text (no path required). */
  loadText: (source: string, label: string) => Promise<TcldData | null>;
  getDetail: (index: number) => Promise<InstructionDetail | null>;
  /**
   * Synchronous detail accessor: returns the cached row if present, or
   * computes it on the spot when the source data is loaded. Returns null
   * when no file is loaded. Use this instead of `getDetail` when you can
   * tolerate a one-off compute (cheap; just a struct rebuild from the
   * already-loaded TcldData) but cannot tolerate the React re-render that
   * a Promise-then-setState round trip imposes.
   */
  getDetailSync: (index: number) => InstructionDetail | null;
  setData: (data: TcldData) => void;
  selectDocument: (documentId: string) => void;
  closeDocument: (documentId: string) => void;
  updateActiveDocumentWorkspaceState: (patch: Partial<DocumentWorkspaceState>) => void;
  reset: () => void;
}

type Store = StoreState & StoreActions;

const defaultState: StoreState = {
  loaded: false,
  loading: false,
  error: null,
  data: null,
  detailCache: new Map(),
  session: createSessionState(),
};

const StoreContext = createContext<Store>({
  ...defaultState,
  loadFile: async () => null,
  loadText: async () => null,
  getDetail: async () => null,
  getDetailSync: () => null,
  setData: () => { },
  selectDocument: () => { },
  closeDocument: () => { },
  updateActiveDocumentWorkspaceState: () => { },
  reset: () => { },
});

export function TcldStoreProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<StoreState>(defaultState);

  const loadFile = useCallback(async (path: string) => {
    setState((s) => ({ ...s, loading: true, error: null }));
    try {
      // Route the LoadResult through the tauri-query cache so two simultaneous
      // openers of the same path (e.g. command palette + drag-drop) share one
      // round-trip. TTL is generous (5min) — invalidation happens on error,
      // on file reload (via reset() below), and on document close.
      const result = await query(
        ["load-file", path],
        () => apiLoadFile(path),
        { ttlMs: 5 * 60_000 },
      );
      const data = tcldDataFromLoadResult(result);
      const document = buildSessionDocument(data, {
        role: "open",
        label: data.metadata.filename,
        sourcePath: path,
      });
      setState((s) => ({
        loaded: true,
        loading: false,
        error: null,
        data,
        detailCache: new Map(),
        session: upsertSessionDocument(s.session, document),
      }));
      return data;
    } catch (err) {
      // On error, drop the cache entry so the next attempt actually re-fires
      // the backend call instead of returning the failure result.
      invalidateQuery("load-file");
      setState((s) => ({
        ...s,
        loading: false,
        error: err instanceof Error ? err.message : String(err),
        detailCache: new Map(),
      }));
      return null;
    }
  }, []);

  const loadText = useCallback(async (source: string, label: string) => {
    setState((s) => ({ ...s, loading: true, error: null }));
    try {
      // Same dedup story as loadFile — two simultaneous loadText calls with
      // the same (label, source) pair share one in-flight Promise.
      const result = await query(
        ["load-text", label, source],
        () => apiLoadText(source, label),
        { ttlMs: 5 * 60_000 },
      );
      const data = tcldDataFromLoadResult(result);
      const document = buildSessionDocument(data, { role: "open", label: data.metadata.filename || label });
      setState((s) => ({
        loaded: true,
        loading: false,
        error: null,
        data,
        detailCache: new Map(),
        session: upsertSessionDocument(s.session, document),
      }));
      return data;
    } catch (err) {
      invalidateQuery("load-text");
      setState((s) => ({
        ...s,
        loading: false,
        error: err instanceof Error ? err.message : String(err),
        detailCache: new Map(),
      }));
      return null;
    }
  }, []);

  // Use ref to avoid recreating getDetail on every cache update
  const cacheRef = useRef(state.detailCache);
  const dataRef = useRef(state.data);
  useEffect(() => {
    cacheRef.current = state.detailCache;
  }, [state.detailCache]);
  useEffect(() => {
    dataRef.current = state.data;
  }, [state.data]);

  const getDetail = useCallback(
    async (index: number): Promise<InstructionDetail | null> => {
      if (cacheRef.current.has(index)) {
        return cacheRef.current.get(index)!;
      }
      const data = dataRef.current;
      if (!data) {
        return null;
      }
      const detail = buildInstructionDetailFromData(data, index);
      if (!detail) return null;
      setState((s) => {
        const cache = new Map(s.detailCache);
        cache.set(index, detail);
        return { ...s, detailCache: cache };
      });
      return detail;
    },
    [],
  );

  // Synchronous variant — see StoreActions.getDetailSync for rationale.
  // Reads from the same cache + source-of-truth refs as getDetail above so
  // both paths produce identical results.
  const getDetailSync = useCallback(
    (index: number): InstructionDetail | null => {
      const cached = cacheRef.current.get(index);
      if (cached) return cached;
      const data = dataRef.current;
      if (!data) return null;
      const detail = buildInstructionDetailFromData(data, index);
      if (!detail) return null;
      // Best-effort cache write; we deliberately do NOT trigger a setState
      // here because the synchronous caller is mid-render. The async path
      // (getDetail) will warm the cache on the next call.
      cacheRef.current.set(index, detail);
      return detail;
    },
    [],
  );

  const setData = useCallback((data: TcldData) => {
    const role = data.provenance?.kind === "DebuggerChunk" ? "live" : "open";
    const document = buildSessionDocument(data, { role, label: data.metadata.filename });
    setState((s) => ({
      loaded: true,
      loading: false,
      error: null,
      data,
      detailCache: new Map(),
      session: upsertSessionDocument(s.session, document),
    }));
  }, []);

  const selectDocument = useCallback((documentId: string) => {
    setState((s) => {
      const session = selectActiveDocument(s.session, documentId);
      const active = session.documents.find((doc) => doc.id === session.activeDocumentId);
      if (!active) return s;
      return {
        ...s,
        loaded: true,
        data: active.data,
        detailCache: new Map(),
        session,
      };
    });
  }, []);

  const closeDocument = useCallback((documentId: string) => {
    setState((s) => {
      const session = closeSessionDocument(s.session, documentId);
      const active = session.documents.find((doc) => doc.id === session.activeDocumentId);
      return {
        ...s,
        loaded: active != null,
        data: active?.data ?? null,
        detailCache: new Map(),
        session,
      };
    });
  }, []);

  const updateActiveDocumentWorkspaceState = useCallback((patch: Partial<DocumentWorkspaceState>) => {
    setState((s) => {
      if (!s.session.activeDocumentId) return s;
      const session = updateSessionDocumentWorkspaceState(s.session, s.session.activeDocumentId, patch);
      return session === s.session ? s : { ...s, session };
    });
  }, []);

  const reset = useCallback(() => {
    // Clear the request cache too — otherwise a "reset and reopen the same
    // file" workflow would replay the cached LoadResult instead of going
    // back to disk. We invalidate both prefixes since either could have
    // been the source of the loaded data.
    invalidateQuery("load-file");
    invalidateQuery("load-text");
    setState(defaultState);
  }, []);

  const value = useMemo(
    () => ({ ...state, loadFile, loadText, getDetail, getDetailSync, setData, selectDocument, closeDocument, updateActiveDocumentWorkspaceState, reset }),
    [state, loadFile, loadText, getDetail, getDetailSync, setData, selectDocument, closeDocument, updateActiveDocumentWorkspaceState, reset],
  );

  return (
    <StoreContext.Provider value={value}>
      {children}
    </StoreContext.Provider>
  );
}

export function useTcldStore(): Store {
  return useContext(StoreContext);
}
