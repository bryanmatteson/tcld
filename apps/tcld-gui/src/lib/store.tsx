// ─── tcld Data Store ─────────────────────────────────────────────
// Central state for loaded bytecode data. Wraps Tauri API calls
// and provides data to all components via React context.

"use client";

import { createContext, useContext, useState, useCallback, useMemo, useRef, type ReactNode } from "react";
import type {
  InstructionDetail,
} from "@/types";
import { loadFile as apiLoadFile, loadText as apiLoadText, loadExample as apiLoadExample, type LoadResult } from "@/lib/tauri-api";
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
  loadExample: (id: string) => Promise<TcldData | null>;
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
  dismissError: () => void;
}

type Store = StoreState & StoreActions;

/**
 * The loaded artifact without session bookkeeping. Panels read this through
 * `useTcldData` so per-cursor document workspace updates do not re-render
 * them; it changes only when the active document, load status, or error do.
 */
export type TcldDataStore = Pick<Store, "loaded" | "loading" | "error" | "data" | "getDetail" | "getDetailSync"> & {
  /** Disk path of the active document, when it came from one. */
  activeSourcePath: string | null;
};

const defaultState: StoreState = {
  loaded: false,
  loading: false,
  error: null,
  data: null,
  detailCache: new Map(),
  session: createSessionState(),
};

const DataContext = createContext<TcldDataStore>({
  loaded: false,
  loading: false,
  error: null,
  data: null,
  activeSourcePath: null,
  getDetail: async () => null,
  getDetailSync: () => null,
});

const StoreContext = createContext<Store>({
  ...defaultState,
  loadFile: async () => null,
  loadText: async () => null,
  loadExample: async () => null,
  getDetail: async () => null,
  getDetailSync: () => null,
  setData: () => { },
  selectDocument: () => { },
  closeDocument: () => { },
  updateActiveDocumentWorkspaceState: () => { },
  reset: () => { },
  dismissError: () => { },
});

export function TcldStoreProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<StoreState>(defaultState);
  const loadGeneration = useRef(0);

  const loadFile = useCallback(async (path: string) => {
    const generation = ++loadGeneration.current;
    setState((s) => ({ ...s, loading: true, error: null }));
    try {
      // Deduplicate concurrent reads, but always re-read disk on a later open.
      const result = await query(
        ["load-file", path],
        () => apiLoadFile(path),
        { ttlMs: 0 },
      );
      if (generation !== loadGeneration.current) return null;
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
      if (generation !== loadGeneration.current) return null;
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

  const loadContent = useCallback(async (key: readonly [string, ...unknown[]], label: string, fetch: () => Promise<LoadResult>) => {
    const generation = ++loadGeneration.current;
    setState((s) => ({ ...s, loading: true, error: null }));
    try {
      const result = await query(key, fetch, { ttlMs: 0 });
      if (generation !== loadGeneration.current) return null;
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
      if (generation !== loadGeneration.current) return null;
      invalidateQuery(key[0]);
      setState((s) => ({
        ...s,
        loading: false,
        error: err instanceof Error ? err.message : String(err),
        detailCache: new Map(),
      }));
      return null;
    }
  }, []);

  const loadText = useCallback((source: string, label: string) =>
    loadContent(["load-text", label, source], label, () => apiLoadText(source, label)), [loadContent]);
  const loadExample = useCallback((id: string) =>
    loadContent(["load-example", id], id, () => apiLoadExample(id)), [loadContent]);

  // Use ref to avoid recreating getDetail on every cache update
  const cacheRef = useRef(state.detailCache);
  const dataRef = useRef(state.data);
  cacheRef.current = state.detailCache;
  dataRef.current = state.data;

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
        if (s.data !== data) return s;
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
    ++loadGeneration.current;
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
    ++loadGeneration.current;
    setState((s) => {
      const session = selectActiveDocument(s.session, documentId);
      const active = session.documents.find((doc) => doc.id === session.activeDocumentId);
      if (!active) return s;
      return {
        ...s,
        loaded: true,
        loading: false,
        error: null,
        data: active.data,
        detailCache: new Map(),
        session,
      };
    });
  }, []);

  const closeDocument = useCallback((documentId: string) => {
    ++loadGeneration.current;
    setState((s) => {
      const session = closeSessionDocument(s.session, documentId);
      const active = session.documents.find((doc) => doc.id === session.activeDocumentId);
      return {
        ...s,
        loading: false,
        error: null,
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

  const dismissError = useCallback(() => {
    setState((s) => ({ ...s, error: null }));
  }, []);

  const reset = useCallback(() => {
    ++loadGeneration.current;
    // Clear the request cache too — otherwise a "reset and reopen the same
    // file" workflow would replay the cached LoadResult instead of going
    // back to disk. We invalidate both prefixes since either could have
    // been the source of the loaded data.
    invalidateQuery("load-file");
    invalidateQuery("load-text");
    invalidateQuery("load-example");
    setState(defaultState);
  }, []);

  const value = useMemo(
    () => ({ ...state, loadFile, loadText, loadExample, getDetail, getDetailSync, setData, selectDocument, closeDocument, updateActiveDocumentWorkspaceState, reset, dismissError }),
    [state, loadFile, loadText, loadExample, getDetail, getDetailSync, setData, selectDocument, closeDocument, updateActiveDocumentWorkspaceState, reset, dismissError],
  );

  const { loaded, loading, error, data } = state;
  const activeSourcePath = state.session.documents
    .find((doc) => doc.id === state.session.activeDocumentId)?.sourcePath ?? null;
  const dataValue = useMemo(
    () => ({ loaded, loading, error, data, activeSourcePath, getDetail, getDetailSync }),
    [loaded, loading, error, data, activeSourcePath, getDetail, getDetailSync],
  );

  return (
    <StoreContext.Provider value={value}>
      <DataContext.Provider value={dataValue}>
        {children}
      </DataContext.Provider>
    </StoreContext.Provider>
  );
}

export function useTcldStore(): Store {
  return useContext(StoreContext);
}

/** Loaded artifact only; see `TcldDataStore`. */
export function useTcldData(): TcldDataStore {
  return useContext(DataContext);
}
