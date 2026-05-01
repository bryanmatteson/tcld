"use client";

import {
  createContext,
  useContext,
  useState,
  useCallback,
  useEffect,
  useRef,
  useMemo,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import type {
  DebugSessionState,
  DebugStoppedEvent,
  DebugOutputEvent,
  DebugExitedEvent,
  DebugReadyEvent,
  DebugResumedEvent,
  DebugBytecodeChunkEvent,
  DebugCommandErrorEvent,
  DebugVariable,
  DebugStackFrame,
  DebugEvalResult,
  DebugBreakpoint,
  WatchpointInfo,
  DebugWatchpointHitEvent,
} from "@/types";
import { RESERVED_DEBUG_EVENT_KINDS } from "@/types";
import {
  debugLaunch,
  debugStop,
  debugStepInto,
  debugStepInstruction,
  debugStepOverInstruction,
  debugStepOutInstruction,
  debugRunToPc,
  debugStepOver,
  debugStepOut,
  debugContinue,
  debugPause,
  debugSetBreakpoint,
  debugClearBreakpoint,
  debugSetBreakpointEnabled,
  debugListBreakpoints,
  debugEval,
  debugGetVariables,
  debugGetStack,
  debugSetVariable,
  debugGetState,
  debugDisassemble,
  debugSetWatchpoint,
  debugClearWatchpoint,
  debugListWatchpoints,
  debugClearAllWatchpoints,
  listen,
  emit,
  isTauri,
} from "@/lib/tauri-api";
import type { DebugLaunchOptions, DebugStateInfo } from "@/lib/tauri-api";
import { useTcldStore, type TcldData } from "@/lib/store";
import {
  acceptsReadyDebugSession,
  acceptsRuntimeDataDebugSession,
  canLaunchDebugSession,
  canStopDebugSession,
  isActiveDebugSession,
  normalizeBackendDebugState,
} from "@/lib/debug-session-machine";
import { ingestLiveLoadSnapshot, precompiledStepAdvisory } from "@/lib/debug-workflows";
import {
  DEBUGGER_UNAVAILABLE_MESSAGE,
  getAppCapabilitiesServerSnapshot,
  getAppCapabilitiesSnapshot,
  isDebuggerAvailable,
  loadAppCapabilities,
  subscribeAppCapabilities,
} from "@/lib/capabilities";

// ─── Output line ────────────────────────────────────────────────

export interface OutputLine {
  stream: "stdout" | "stderr" | "eval" | "error" | "system" | "diag";
  text: string;
  timestamp: number;
}

const MAX_OUTPUT_LINES = 10_000;

function appendOutput(prev: OutputLine[], line: OutputLine): OutputLine[] {
  const next = [...prev, line];
  return next.length > MAX_OUTPUT_LINES ? next.slice(-MAX_OUTPUT_LINES) : next;
}

function splitDisassembly(text: string): { header: string; instructions: string } {
  const lines = text.split("\n");
  const idx = lines.findIndex((l) => /^\s+Command \d+:/.test(l));
  if (idx <= 0) return { header: "", instructions: text };
  return {
    header: lines.slice(0, idx).join("\n"),
    instructions: lines.slice(idx).join("\n"),
  };
}

type DebugStackFramePayload = {
  level: number;
  type: string;
  proc?: string | null;
  procName?: string | null;
  file?: string | null;
  line?: number | null;
  cmd?: string | null;
};

function normalizeDebugStackFrames(frames: DebugStackFramePayload[]): DebugStackFrame[] {
  return frames.map((frame) => ({
    level: frame.level,
    type: frame.type,
    proc: frame.proc ?? frame.procName ?? null,
    file: frame.file ?? null,
    line: frame.line ?? null,
    cmd: frame.cmd ?? null,
  }));
}

// ─── Store shape ────────────────────────────────────────────────

interface DebugStore {
  sessionState: DebugSessionState;
  isBusy: boolean;
  /**
   * Backend capability handshake (`get_app_capabilities`). False when the
   * Rust backend was built without the `debugger` feature — launch controls
   * disable, debug panels show an unavailable state, and `launch` refuses
   * with DEBUGGER_UNAVAILABLE_MESSAGE instead of invoking IPC. Defaults to
   * true until the handshake resolves so normal builds never flicker.
   */
  debuggerAvailable: boolean;
  scriptPath: string;
  tclVersion: string | null;
  debugCapabilities: Record<string, boolean>;
  canStepInstruction: boolean;
  pid: number | null;
  stoppedAt: DebugStoppedEvent | null;
  variables: DebugVariable[];
  variablesLevel: number;
  stack: DebugStackFrame[];
  output: OutputLine[];
  breakpoints: DebugBreakpoint[];
  watchpoints: WatchpointInfo[];
  watchpointHit: DebugWatchpointHitEvent | null;
  evalHistory: string[];
  disassemblyText: string | null;
  disassemblyHeader: string | null;
  liveDecompileText: string | null;
  bytecodeChunks: DebugBytecodeChunkEvent[];

  launch: (
    path: string,
    args?: string[],
    tclshPath?: string,
    launchOptions?: DebugLaunchOptions,
  ) => Promise<boolean>;
  stop: () => Promise<void>;
  stepInto: () => Promise<void>;
  stepInstruction: () => Promise<void>;
  stepOverInstruction: () => Promise<void>;
  stepOutInstruction: () => Promise<void>;
  runToPc: (pc: number) => Promise<void>;
  stepOver: () => Promise<void>;
  stepOut: () => Promise<void>;
  stepToNextGroup: () => Promise<void>;
  stepToPrevGroup: () => Promise<void>;
  resume: () => Promise<void>;
  runToCursor: (
    file: string,
    line: number,
    command?: { offset: number; len: number },
  ) => Promise<void>;
  pause: () => Promise<void>;
  toggleBreakpoint: (
    file: string,
    line: number,
    command?: { offset: number; len: number },
  ) => Promise<void>;
  setBreakpoint: (
    file: string,
    line: number,
    condition?: string,
    command?: { offset: number; len: number },
  ) => Promise<void>;
  clearBreakpoint: (
    file: string,
    line: number,
    command?: { offset: number; len: number },
  ) => Promise<void>;
  evalScript: (script: string, level?: number) => Promise<void>;
  fetchVariables: (level?: number) => Promise<void>;
  fetchStack: () => Promise<void>;
  setVariable: (name: string, value: string, level?: number) => Promise<void>;
  clearAllBreakpoints: () => Promise<void>;
  refreshBreakpoints: () => Promise<void>;
  setWatchpoint: (name: string, scope?: string, level?: number, condition?: string) => Promise<void>;
  clearWatchpoint: (name: string) => Promise<void>;
  clearAllWatchpoints: () => Promise<void>;
  refreshWatchpoints: () => Promise<void>;
  clearOutput: () => void;
  resetSession: () => void;
  restart: () => Promise<void>;
}

const DebugContext = createContext<DebugStore | null>(null);

function parseCapabilityFlags(raw: string | null | undefined): Record<string, boolean> {
  if (!raw) return {};
  const tokens = raw.trim().split(/\s+/);
  const flags: Record<string, boolean> = {};
  for (let i = 0; i + 1 < tokens.length; i += 2) {
    const key = tokens[i];
    const val = tokens[i + 1]?.toLowerCase();
    if (val === "1" || val === "true") {
      flags[key] = true;
    } else if (val === "0" || val === "false") {
      flags[key] = false;
    }
  }
  return flags;
}

function filesLikelyMatch(a: string, b: string): boolean {
  if (!a || !b) return false;
  const norm = (s: string) => s.replace(/\\/g, "/").toLowerCase();
  const na = norm(a);
  const nb = norm(b);
  if (na === nb) return true;
  const ta = na.split("/").pop();
  const tb = nb.split("/").pop();
  return !!ta && ta === tb;
}

function firstInstructionForGroup(
  instructions: TcldData["instructions"],
  group: number | null,
): number | null {
  if (group == null) return null;
  const first = instructions.find((i) => i.group === group && i.groupStart);
  return first ? first.idx : null;
}

function isSourceBreakpoint(
  bp: DebugBreakpoint,
): bp is DebugBreakpoint & { file: string; line: number } {
  return (
    bp.kind !== "instruction" &&
    typeof bp.file === "string" &&
    typeof bp.line === "number" &&
    Number.isFinite(bp.line)
  );
}

function resolveStoppedInstructionIndex(
  stoppedAt: DebugStoppedEvent,
  scriptPath: string,
  data: TcldData,
): number | null {
  const instructions = data.instructions;
  const decompileLines = data.decompileLines;
  if (instructions.length === 0) return null;

  if (
    stoppedAt.file
    && scriptPath
    && !filesLikelyMatch(stoppedAt.file, scriptPath)
  ) {
    return null;
  }

  if (stoppedAt.instructionIndex != null) {
    return (
      stoppedAt.instructionIndex >= 0 && stoppedAt.instructionIndex < instructions.length
        ? stoppedAt.instructionIndex
        : null
    );
  }

  if (stoppedAt.line === 0) {
    // Line 0 means "at the very start" — find the first instruction that belongs to a command group
    const firstGrouped = instructions.findIndex((i) => i.group != null);
    return firstGrouped >= 0 ? firstGrouped : 0;
  }

  if (stoppedAt.line > 0 && decompileLines.length > 0) {
    const sourceLine = stoppedAt.line;

    for (const dl of decompileLines) {
      if (
        dl.group != null
        && dl.sourceLineStart != null
        && dl.sourceLineEnd != null
        && sourceLine >= dl.sourceLineStart
        && sourceLine <= dl.sourceLineEnd
      ) {
        return firstInstructionForGroup(instructions, dl.group);
      }
    }

    const exact = decompileLines[sourceLine - 1];
    const exactIdx = firstInstructionForGroup(instructions, exact?.group ?? null);
    if (exactIdx != null) return exactIdx;

    const searchRadius = Math.min(decompileLines.length, 20);
    for (let offset = 1; offset < searchRadius; offset += 1) {
      for (const dir of [-1, 1]) {
        const idx = sourceLine - 1 + dir * offset;
        if (idx >= 0 && idx < decompileLines.length) {
          const near = firstInstructionForGroup(instructions, decompileLines[idx].group);
          if (near != null) return near;
        }
      }
    }
  }

  // Last resort: first instruction with a command group
  const firstGrouped = instructions.findIndex((i) => i.group != null);
  if (firstGrouped >= 0) return firstGrouped;

  return null;
}

export function DebugStoreProvider({ children }: { children: ReactNode }) {
  const { data: tcldData, reset: resetTcldData, setData: setTcldData, loadFile: loadStaticFile } = useTcldStore();
  // Capability handshake — module-level state so store actions can read it
  // synchronously (isDebuggerAvailable) while React re-renders on change.
  const { debuggerAvailable } = useSyncExternalStore(
    subscribeAppCapabilities,
    getAppCapabilitiesSnapshot,
    getAppCapabilitiesServerSnapshot,
  );
  const [sessionState, setSessionState] = useState<DebugSessionState>("idle");
  const [scriptPath, setScriptPathState] = useState("");
  const [tclVersion, setTclVersion] = useState<string | null>(null);
  const [debugCapabilities, setDebugCapabilities] = useState<Record<string, boolean>>({});
  const [pid, setPid] = useState<number | null>(null);
  const [stoppedAt, setStoppedAt] = useState<DebugStoppedEvent | null>(null);
  const [variables, setVariables] = useState<DebugVariable[]>([]);
  const [variablesLevel, setVariablesLevel] = useState(1);
  const [stack, setStack] = useState<DebugStackFrame[]>([]);
  const [output, setOutput] = useState<OutputLine[]>([]);
  const [breakpoints, setBreakpoints] = useState<DebugBreakpoint[]>([]);
  const [watchpoints, setWatchpoints] = useState<WatchpointInfo[]>([]);
  const [watchpointHit, setWatchpointHit] = useState<DebugWatchpointHitEvent | null>(null);
  const [evalHistory, setEvalHistory] = useState<string[]>([]);
  const [disassemblyText, setDisassemblyTextRaw] = useState<string | null>(null);
  const [disassemblyHeader, setDisassemblyHeader] = useState<string | null>(null);
  const [liveDecompileText, setLiveDecompileText] = useState<string | null>(null);

  const setDisassemblyText = useCallback((text: string | null) => {
    if (text === null) {
      setDisassemblyTextRaw(null);
      setDisassemblyHeader(null);
    } else {
      const { header, instructions } = splitDisassembly(text);
      setDisassemblyHeader(header || null);
      setDisassemblyTextRaw(instructions);
    }
  }, []);
  const [bytecodeChunks, setBytecodeChunks] = useState<DebugBytecodeChunkEvent[]>([]);
  const [isBusy, setIsBusy] = useState(false);
  const canStepInstruction = useMemo(
    () => debugCapabilities.instruction_step === true,
    [debugCapabilities],
  );

  // ── Refs (synchronously readable) ───────────────────────────
  const sessionStateRef = useRef(sessionState);
  const scriptPathRef = useRef(scriptPath);
  const tcldDataRef = useRef<TcldData | null>(tcldData);
  const variablesLevelRef = useRef(variablesLevel);
  const breakpointsRef = useRef(breakpoints);
  const stoppedAtRef = useRef(stoppedAt);
  const lastLaunchRef = useRef<{
    path: string;
    args: string[];
    tclshPath?: string;
    launchOptions?: DebugLaunchOptions;
  } | null>(null);
  const resetTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const actionChainRef = useRef<Promise<void>>(Promise.resolve());
  const actionDepthRef = useRef(0);
  const tempBreakpointsRef = useRef(
    new Map<string, { file: string; line: number; offset?: number; len?: number }>(),
  );
  // Capabilities mirror for the listener effect, whose closure is created
  // once and never re-runs on capability change — reading the React state
  // directly there would see a stale (usually empty) snapshot.
  const debugCapabilitiesRef = useRef(debugCapabilities);
  // Fires the "precompiled bytecode can't source-step here" advisory at most
  // once per session. Reset on launch / resetSession.
  const precompiledStepAdvisoryShownRef = useRef(false);

  // Wrapper that updates both state and ref synchronously
  const setSession = useCallback((next: DebugSessionState) => {
    sessionStateRef.current = next;
    setSessionState(next);
  }, []);

  const setScriptPath = useCallback((next: string) => {
    scriptPathRef.current = next;
    setScriptPathState(next);
  }, []);

  const setStoppedAtSynced = useCallback((next: DebugStoppedEvent | null) => {
    stoppedAtRef.current = next;
    setStoppedAt(next);
  }, []);

  // Keep other refs in sync
  useEffect(() => { variablesLevelRef.current = variablesLevel; }, [variablesLevel]);
  useEffect(() => { debugCapabilitiesRef.current = debugCapabilities; }, [debugCapabilities]);
  useEffect(() => { scriptPathRef.current = scriptPath; }, [scriptPath]);
  useEffect(() => { tcldDataRef.current = tcldData; }, [tcldData]);
  useEffect(() => { breakpointsRef.current = breakpoints; }, [breakpoints]);

  const pushOutput = useCallback(
    (stream: OutputLine["stream"], text: string) => {
      setOutput((prev) =>
        appendOutput(prev, { stream, text, timestamp: Date.now() }),
      );
    },
    [],
  );

  const enqueueAction = useCallback(
    async <T,>(action: () => Promise<T>): Promise<T> => {
      const wrapped = async () => {
        actionDepthRef.current += 1;
        setIsBusy(true);
        try {
          return await action();
        } finally {
          actionDepthRef.current -= 1;
          if (actionDepthRef.current <= 0) {
            actionDepthRef.current = 0;
            setIsBusy(false);
          }
        }
      };
      const run = actionChainRef.current.then(wrapped, wrapped);
      actionChainRef.current = run.then(
        () => undefined,
        () => undefined,
      );
      return run;
    },
    [],
  );

  const tempBreakpointKey = useCallback(
    (file: string, line: number, offset?: number) =>
      `${file}:${line}:${offset == null ? "line" : offset}`,
    [],
  );

  const clearTempBreakpoints = useCallback(async () => {
    const pending = [...tempBreakpointsRef.current.values()];
    if (pending.length === 0) return;

    for (const bp of pending) {
      try {
        await debugClearBreakpoint(bp.file, bp.line, bp.offset);
      } catch {
        // Ignore clear failures; we still clear local temp bookkeeping below.
      }
    }

    const keys = new Set(
      pending.map((bp) => tempBreakpointKey(bp.file, bp.line, bp.offset)),
    );
    tempBreakpointsRef.current.clear();
    setBreakpoints((bps) =>
      bps.filter(
        (bp) =>
          !isSourceBreakpoint(bp)
          || !keys.has(tempBreakpointKey(bp.file, bp.line, bp.offset)),
      ));
  }, [tempBreakpointKey]);

  const applyBackendState = useCallback((snapshot: DebugStateInfo) => {
    const normalized = normalizeBackendDebugState(snapshot.state);
    if (normalized) {
      setSession(normalized);
    }
    setScriptPath(snapshot.scriptPath ?? "");
    setTclVersion(snapshot.tclVersion ?? null);
    setPid(snapshot.pid ?? null);
  }, [setScriptPath, setSession]);

  const syncFromBackendState = useCallback(
    async (): Promise<DebugSessionState | null> => {
      if (!isTauri()) return null;
      try {
        const backend = await debugGetState();
        applyBackendState(backend);
        return normalizeBackendDebugState(backend.state);
      } catch {
        // Ignore sync failures here; callers decide what to do.
      }
      return null;
    },
    [applyBackendState],
  );

  // Cancel any pending auto-reset timer
  const cancelResetTimer = useCallback(() => {
    if (resetTimerRef.current !== null) {
      clearTimeout(resetTimerRef.current);
      resetTimerRef.current = null;
    }
  }, []);

  useEffect(() => {
    void syncFromBackendState();
  }, [syncFromBackendState]);

  // Resolve the backend capability handshake once at store bootstrap
  // (mirrors how appearance settings hydrate). Until it resolves the UI
  // keeps the optimistic default (available) to avoid flicker on normal
  // builds; a debugger-less backend flips the flag and the UI degrades.
  useEffect(() => {
    if (!isTauri()) return;
    void loadAppCapabilities();
  }, []);

  // ── Emit state-changed for Tauri menu enable/disable ────────
  useEffect(() => {
    if (!isTauri()) return;
    emit("debug:state-changed", {
      sessionState,
      hasBps: breakpoints.length > 0,
      hasStoppedAt: stoppedAt !== null,
    });
  }, [sessionState, breakpoints.length, stoppedAt]);

  // ── Event subscriptions (stable — no deps that change) ──────
  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    const unlisteners: (() => void)[] = [];

    const setup = async () => {
      const uState = await listen<DebugStateInfo>("debugger:state", (payload) => {
        if (cancelled) return;
        applyBackendState(payload);
      });
      if (cancelled) { uState(); return; }
      unlisteners.push(uState);

      const u1 = await listen<DebugReadyEvent>("debugger:ready", (payload) => {
        if (cancelled) return;
        if (!acceptsReadyDebugSession(sessionStateRef.current)) return;
        setDebugCapabilities(parseCapabilityFlags(payload.debugExtensionCapabilities));
        setTclVersion(payload.tclVersion);
        setPid(payload.pid);
        pushOutput("system", `Connected to tclsh ${payload.tclVersion} (pid ${payload.pid})`);
        const sp = scriptPathRef.current;
        if (sp) {
          debugDisassemble(sp).then((text) => {
            setDisassemblyText(text);
            pushOutput("system", `Fetched disassembly (${text.split("\n").length} lines)`);
          }).catch(() => { });
        }
      });
      if (cancelled) { u1(); return; }
      unlisteners.push(u1);

      const u2 = await listen<DebugStoppedEvent>("debugger:stopped", (payload) => {
        if (cancelled) return;
        if (!isActiveDebugSession(sessionStateRef.current)) return;
        void clearTempBreakpoints();
        setStoppedAtSynced(payload);
        const location = payload.file ? `${payload.file}:${payload.line}` : `line ${payload.line}`;
        pushOutput("system", `Paused (${payload.reason}) at ${location}`);
        // Explain (once per session) why the marker won't leave line 1 when the
        // target is precompiled bytecode and the runtime can't step opcodes —
        // otherwise stepping looks silently broken. Read capabilities through
        // the ref: this listener's closure predates the handshake result.
        if (!precompiledStepAdvisoryShownRef.current) {
          const advisory = precompiledStepAdvisory(
            payload,
            scriptPathRef.current,
            debugCapabilitiesRef.current,
          );
          if (advisory) {
            precompiledStepAdvisoryShownRef.current = true;
            pushOutput("system", advisory);
          }
        }
        // Hit counts (and any backend-side breakpoint normalization) only
        // change when the target stops, so this is the one sync point for
        // the breakpoint list. Routed through enqueueAction so it serializes
        // BEHIND an in-flight launch — the launch action replays pre-launch
        // breakpoints to the backend first, and this refresh then reads the
        // authoritative post-replay list instead of racing it.
        void enqueueAction(async () => {
          try {
            setBreakpoints(await debugListBreakpoints());
          } catch { /* keep the local list when the refresh round-trip fails */ }
        });
        // Note: variables and stack arrive on the `debugger:variables` and
        // `debugger:stack` event channels (see u11/u12 below). The previous
        // synchronous debugGetVariables/debugGetStack calls here raced the
        // event stream — both deliveries landed, the second one overwrote
        // the first, and the first round-trip was wasted. Trust the events.
      });
      if (cancelled) { u2(); return; }
      unlisteners.push(u2);

      const u3 = await listen<DebugOutputEvent>("debugger:output", (payload) => {
        if (cancelled) return;
        if (sessionStateRef.current === "idle") return;
        setOutput((o) => appendOutput(o, { stream: payload.stream as OutputLine["stream"], text: payload.text, timestamp: Date.now() }));
      });
      if (cancelled) { u3(); return; }
      unlisteners.push(u3);

      const u4 = await listen<DebugExitedEvent>("debugger:exited", (payload) => {
        if (cancelled) return;
        void clearTempBreakpoints();
        setDebugCapabilities({});
        pushOutput("system", `Process exited with code ${payload.code}${payload.reason ? `: ${payload.reason}` : ""}`);
      });
      if (cancelled) { u4(); return; }
      unlisteners.push(u4);

      const u5 = await listen<{ level: number; vars: DebugVariable[] }>(
        "debugger:variables",
        (payload) => {
          if (cancelled) return;
          if (!acceptsRuntimeDataDebugSession(sessionStateRef.current)) return;
          setVariables(payload.vars);
          setVariablesLevel(payload.level);
        },
      );
      if (cancelled) { u5(); return; }
      unlisteners.push(u5);

      const u6 = await listen<{ frames: DebugStackFramePayload[] }>("debugger:stack", (payload) => {
        if (cancelled) return;
        if (!acceptsRuntimeDataDebugSession(sessionStateRef.current)) return;
        setStack(normalizeDebugStackFrames(payload.frames));
      });
      if (cancelled) { u6(); return; }
      unlisteners.push(u6);

      const u7 = await listen<DebugEvalResult>("debugger:eval_result", (payload) => {
        if (cancelled) return;
        if (!acceptsRuntimeDataDebugSession(sessionStateRef.current)) return;
        setOutput((o) => appendOutput(o, {
          stream: payload.error ? "error" : "eval",
          text: payload.result,
          timestamp: Date.now(),
        }));
      });
      if (cancelled) { u7(); return; }
      unlisteners.push(u7);

      const u8 = await listen<DebugBytecodeChunkEvent>("debugger:bytecode_chunk", (payload) => {
        if (cancelled) return;
        if (!isActiveDebugSession(sessionStateRef.current)) return;
        if (payload.sequence === 1) {
          pushOutput("system", `Streaming bytecode chunks via ${payload.command}`);
        }
        setBytecodeChunks((prev) => {
          const next = [...prev, payload];
          return next.length > 200 ? next.slice(-200) : next;
        });
        if (payload.disassembly && payload.disassembly.trim().length > 0) {
          setDisassemblyText(payload.disassembly);
        }
        if (payload.decompileText && payload.decompileText.trim().length > 0) {
          setLiveDecompileText(payload.decompileText);
        }
      });
      if (cancelled) { u8(); return; }
      unlisteners.push(u8);

      const uLiveLoad = await listen<TcldData>("debugger:live_load", (payload) => {
        if (cancelled) return;
        if (!isActiveDebugSession(sessionStateRef.current)) return;
        ingestLiveLoadSnapshot(setTcldData, payload);
      });
      if (cancelled) { uLiveLoad(); return; }
      unlisteners.push(uLiveLoad);

      const u9 = await listen<DebugCommandErrorEvent>("debugger:command_error", (payload) => {
        if (cancelled) return;
        if (!isActiveDebugSession(sessionStateRef.current)) return;

        const summary = payload.message?.trim() || "command failed";
        pushOutput(
          "error",
          `[hook:${payload.command}] ${summary} (code ${payload.code}, mode=${payload.controlMode}, argc=${payload.argc})`,
        );

        if (payload.errorCode && payload.errorCode.trim().length > 0) {
          pushOutput("error", `[hook:${payload.command}] errorCode: ${payload.errorCode}`);
        }

        if (payload.errorInfo && payload.errorInfo.trim().length > 0) {
          const firstLine = payload.errorInfo.trim().split("\n", 1)[0] ?? payload.errorInfo.trim();
          pushOutput("error", `[hook:${payload.command}] ${firstLine}`);
        }
      });
      if (cancelled) { u9(); return; }
      unlisteners.push(u9);

      const uResumed = await listen<DebugResumedEvent>("debugger:resumed", () => {
        if (cancelled) return;
        if (!isActiveDebugSession(sessionStateRef.current)) return;
        setStoppedAtSynced(null);
      });
      if (cancelled) { uResumed(); return; }
      unlisteners.push(uResumed);

      const uWatchpointHit = await listen<DebugWatchpointHitEvent>("debugger:watchpoint_hit", (payload) => {
        if (cancelled) return;
        if (!isActiveDebugSession(sessionStateRef.current)) return;
        setWatchpointHit(payload);
      });
      if (cancelled) { uWatchpointHit(); return; }
      unlisteners.push(uWatchpointHit);

      // ── Reserved v0.4+ event kinds (tolerant default branch) ──
      // The backend reserves these channels for the four gated runtime
      // features (live disassembly at PC, opcode profiling, epoch
      // invalidation tracking, compilation introspection). No UI consumes
      // them yet, so record each arrival in the output log instead of
      // dropping it silently — and never let it break the store. Event
      // kinds the store has never heard of at all can't break it either:
      // Tauri events are per-channel, so unsubscribed kinds never reach
      // this code by construction.
      for (const kind of RESERVED_DEBUG_EVENT_KINDS) {
        const uReserved = await listen<unknown>(`debugger:${kind}`, () => {
          if (cancelled) return;
          pushOutput("diag", `unhandled event kind: ${kind}`);
        });
        if (cancelled) { uReserved(); return; }
        unlisteners.push(uReserved);
      }
    };

    setup();

    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, [applyBackendState, clearTempBreakpoints, enqueueAction, pushOutput, setDisassemblyText, setStoppedAtSynced, setTcldData]);

  // ── Actions ─────────────────────────────────────────────────

  const resetSession = useCallback(() => {
    cancelResetTimer();
    tempBreakpointsRef.current.clear();
    precompiledStepAdvisoryShownRef.current = false;
    setStoppedAtSynced(null);
    setVariables([]);
    setStack([]);
    setWatchpoints([]);
    setWatchpointHit(null);
    setDebugCapabilities({});
    setDisassemblyText(null);
    setLiveDecompileText(null);
    setBytecodeChunks([]);
    void syncFromBackendState();
  }, [cancelResetTimer, setDisassemblyText, setStoppedAtSynced, syncFromBackendState]);

  const launch = useCallback(
    async (
      path: string,
      args: string[] = [],
      tclshPath?: string,
      launchOptions?: DebugLaunchOptions,
    ): Promise<boolean> =>
      enqueueAction(async () => {
        // Refuse launches outright when the backend lacks the debugger
        // feature — otherwise the IPC call would surface a raw "command
        // not found" error. Read through the module getter (not the React
        // snapshot) so an in-flight handshake result is never stale here.
        if (!isDebuggerAvailable()) {
          pushOutput("error", DEBUGGER_UNAVAILABLE_MESSAGE);
          return false;
        }
        if (!canLaunchDebugSession(sessionStateRef.current)) {
          pushOutput("error", "A debug session is already active.");
          return false;
        }

        cancelResetTimer();
        tempBreakpointsRef.current.clear();
        precompiledStepAdvisoryShownRef.current = false;

        // If backend has a stale session, force-stop it first.
        const backend = await syncFromBackendState();
        if (backend !== null && !canLaunchDebugSession(backend)) {
          try {
            await debugStop();
          } catch (err) {
            pushOutput("error", `Failed to recover stale session: ${err instanceof Error ? err.message : String(err)}`);
            return false;
          }
          await syncFromBackendState();
        }

        setTclVersion(null);
        setDebugCapabilities({});
        setPid(null);
        setStoppedAtSynced(null);
        setVariables([]);
        setStack([]);
        setWatchpoints([]);
        setWatchpointHit(null);
        setDisassemblyText(null);
        setLiveDecompileText(null);
        setBytecodeChunks([]);
        resetTcldData();
        // Pre-load the static workspace so the structured panels (Source
        // View, Disassembly) populate even when the target pauses before any
        // bytecode hook (tbcload::bceval and friends) has streamed a live
        // chunk — e.g. stopped on a breakpoint at the first line. Without
        // this, store.data stays null and the Source View hangs on "waiting
        // for bytecode chunks…". Fire-and-forget AFTER resetTcldData() so the
        // eventual setData isn't wiped by the reset, and unawaited so it runs
        // in parallel with debugLaunch below — by the time the debugger pauses
        // on its first breakpoint the panels are already filled. If live
        // chunks do stream, debugger:live_load upserts the more-accurate live
        // snapshot on top. A non-decompilable target just no-ops via .catch.
        void loadStaticFile(path).catch(() => null);
        setOutput([
          { stream: "system", text: `Launching ${path}...`, timestamp: Date.now() },
        ]);
        lastLaunchRef.current = { path, args, tclshPath, launchOptions };

        // Set state to "launching" immediately so that debugger:ready and
        // debugger:stopped events arriving before debugLaunch() resolves
        // are not dropped by the isActiveDebugSession / acceptsReadyDebugSession guards.
        setSession("launching");
        setScriptPath(path);

        try {
          await debugLaunch(path, args, tclshPath, launchOptions);
          await syncFromBackendState();
          // Replay any breakpoints the user set before launching so they
          // take effect in the new session. Errors per-breakpoint are
          // logged but don't abort the launch — the user still has a
          // working session, just without that particular breakpoint.
          const pendingBps = breakpointsRef.current.filter(isSourceBreakpoint);
          for (const bp of pendingBps) {
            try {
              await debugSetBreakpoint(bp.file, bp.line, bp.condition, bp.offset, bp.len);
              if (!bp.enabled) {
                await debugSetBreakpointEnabled(bp.file, bp.line, false, bp.offset);
              }
            } catch (err) {
              pushOutput(
                "error",
                `Failed to apply breakpoint ${bp.file}:${bp.line}: ${err instanceof Error ? err.message : String(err)}`,
              );
            }
          }
          return true;
        } catch (err) {
          const message = err instanceof Error ? err.message : String(err);
          pushOutput("error", `Launch failed: ${message}`);
          await syncFromBackendState();
          return false;
        }
      }),
    [
      cancelResetTimer,
      enqueueAction,
      loadStaticFile,
      pushOutput,
      resetTcldData,
      setDisassemblyText,
      setScriptPath,
      setSession,
      setStoppedAtSynced,
      syncFromBackendState,
    ],
  );

  const stop = useCallback(
    async () =>
      enqueueAction(async () => {
        if (!canStopDebugSession(sessionStateRef.current)) return;

        cancelResetTimer();
        await clearTempBreakpoints();

        try {
          await debugStop();
          await syncFromBackendState();
          pushOutput("system", "Debug session stopped.");
        } catch (err) {
          pushOutput("error", `Stop failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [cancelResetTimer, clearTempBreakpoints, enqueueAction, pushOutput, syncFromBackendState],
  );

  const restart = useCallback(async () => {
    const last = lastLaunchRef.current;
    if (!last) return;
    if (canStopDebugSession(sessionStateRef.current)) {
      await stop();
    }
    await launch(last.path, last.args, last.tclshPath, last.launchOptions);
  }, [stop, launch]);

  const stepInto = useCallback(
    async () =>
      enqueueAction(async () => {
        if (stoppedAtRef.current?.reason === "exception") {
          pushOutput("system", "Cannot step from an uncaught exception. Restart to run again.");
          return;
        }
        if (sessionStateRef.current !== "paused") {
          pushOutput("system", `Step into ignored: debugger is ${sessionStateRef.current}, expected paused.`);
          return;
        }

        try {
          await debugStepInto();
        } catch (err) {
          await syncFromBackendState();
          pushOutput("error", `Step into failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput, syncFromBackendState],
  );

  const stepInstruction = useCallback(
    async () =>
      enqueueAction(async () => {
        if (stoppedAtRef.current?.reason === "exception") {
          pushOutput("system", "Cannot step from an uncaught exception. Restart to run again.");
          return;
        }
        if (sessionStateRef.current !== "paused") {
          pushOutput(
            "system",
            `Step instruction ignored: debugger is ${sessionStateRef.current}, expected paused.`,
          );
          return;
        }
        if (!canStepInstruction) {
          pushOutput("system", "Instruction stepping is not available for this runtime.");
          return;
        }
        try {
          await debugStepInstruction();
        } catch (err) {
          await syncFromBackendState();
          pushOutput(
            "error",
            `Step instruction failed: ${err instanceof Error ? err.message : String(err)}`,
          );
        }
      }),
    [canStepInstruction, enqueueAction, pushOutput, syncFromBackendState],
  );

  const stepOverInstruction = useCallback(
    async () =>
      enqueueAction(async () => {
        if (stoppedAtRef.current?.reason === "exception") {
          pushOutput("system", "Cannot step from an uncaught exception. Restart to run again.");
          return;
        }
        if (sessionStateRef.current !== "paused") {
          pushOutput("system", `Step over instruction ignored: debugger is ${sessionStateRef.current}, expected paused.`);
          return;
        }
        if (!canStepInstruction) {
          pushOutput("system", "Instruction stepping is not available for this runtime.");
          return;
        }
        try {
          await debugStepOverInstruction();
        } catch (err) {
          await syncFromBackendState();
          pushOutput("error", `Step over instruction failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [canStepInstruction, enqueueAction, pushOutput, syncFromBackendState],
  );

  const stepOutInstruction = useCallback(
    async () =>
      enqueueAction(async () => {
        if (stoppedAtRef.current?.reason === "exception") {
          pushOutput("system", "Cannot step from an uncaught exception. Restart to run again.");
          return;
        }
        if (sessionStateRef.current !== "paused") {
          pushOutput("system", `Step out instruction ignored: debugger is ${sessionStateRef.current}, expected paused.`);
          return;
        }
        if (!canStepInstruction) {
          pushOutput("system", "Instruction stepping is not available for this runtime.");
          return;
        }
        try {
          await debugStepOutInstruction();
        } catch (err) {
          await syncFromBackendState();
          pushOutput("error", `Step out instruction failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [canStepInstruction, enqueueAction, pushOutput, syncFromBackendState],
  );

  const runToPc = useCallback(
    async (pc: number) =>
      enqueueAction(async () => {
        if (stoppedAtRef.current?.reason === "exception") {
          pushOutput("system", "Cannot run from an uncaught exception. Restart to run again.");
          return;
        }
        if (sessionStateRef.current !== "paused") {
          pushOutput("system", `Run to PC ignored: debugger is ${sessionStateRef.current}, expected paused.`);
          return;
        }
        if (!canStepInstruction) {
          pushOutput("system", "Run to PC is not available for this runtime.");
          return;
        }
        try {
          await debugRunToPc(pc);
        } catch (err) {
          await syncFromBackendState();
          pushOutput("error", `Run to PC failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [canStepInstruction, enqueueAction, pushOutput, syncFromBackendState],
  );

  const stepOver = useCallback(
    async () =>
      enqueueAction(async () => {
        if (stoppedAtRef.current?.reason === "exception") {
          pushOutput("system", "Cannot step from an uncaught exception. Restart to run again.");
          return;
        }
        if (sessionStateRef.current !== "paused") {
          pushOutput("system", `Step over ignored: debugger is ${sessionStateRef.current}, expected paused.`);
          return;
        }
        try {
          await debugStepOver();
        } catch (err) {
          await syncFromBackendState();
          pushOutput("error", `Step over failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput, syncFromBackendState],
  );

  const stepOut = useCallback(
    async () =>
      enqueueAction(async () => {
        if (stoppedAtRef.current?.reason === "exception") {
          pushOutput("system", "Cannot step from an uncaught exception. Restart to run again.");
          return;
        }
        if (sessionStateRef.current !== "paused") {
          pushOutput("system", `Step out ignored: debugger is ${sessionStateRef.current}, expected paused.`);
          return;
        }
        try {
          await debugStepOut();
        } catch (err) {
          await syncFromBackendState();
          pushOutput("error", `Step out failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput, syncFromBackendState],
  );

  const stepToNextGroup = useCallback(
    async () =>
      enqueueAction(async () => {
        if (stoppedAtRef.current?.reason === "exception") {
          pushOutput("system", "Cannot step from an uncaught exception. Restart to run again.");
          return;
        }
        if (sessionStateRef.current !== "paused") {
          pushOutput("system", `Step to next group ignored: debugger is ${sessionStateRef.current}, expected paused.`);
          return;
        }

        const data = tcldDataRef.current;
        const stopped = stoppedAtRef.current;
        const currentScriptPath = scriptPathRef.current;
        if (!data || !stopped) {
          pushOutput("system", "No loaded bytecode context for group stepping.");
          return;
        }

        const startIdx = resolveStoppedInstructionIndex(stopped, currentScriptPath, data);
        if (startIdx == null || startIdx < 0 || startIdx >= data.instructions.length) {
          pushOutput("system", "Unable to resolve current instruction for group stepping.");
          return;
        }

        const startGroup = data.instructions[startIdx]?.group ?? null;
        const maxSteps = Math.max(1, Math.min(2000, data.instructions.length * 2));

        for (let i = 0; i < maxSteps; i += 1) {
          try {
            await debugStepOver();
          } catch (err) {
            await syncFromBackendState();
            pushOutput("error", `Step to next group failed: ${err instanceof Error ? err.message : String(err)}`);
            return;
          }

          const nextData = tcldDataRef.current ?? data;
          const nextStopped = stoppedAtRef.current;
          if (!nextStopped) continue;

          const nextIdx = resolveStoppedInstructionIndex(nextStopped, scriptPathRef.current, nextData);
          if (nextIdx == null || nextIdx < 0 || nextIdx >= nextData.instructions.length) continue;

          const nextGroup = nextData.instructions[nextIdx]?.group ?? null;
          if (nextGroup !== startGroup) {
            pushOutput(
              "system",
              `Stepped to group ${nextGroup != null ? `c${String(nextGroup).padStart(3, "0")}` : "none"}.`,
            );
            return;
          }
        }

        pushOutput("system", "Step-to-next-group limit reached before group changed.");
      }),
    [enqueueAction, pushOutput, syncFromBackendState],
  );

  const stepToPrevGroup = useCallback(
    async () =>
      enqueueAction(async () => {
        if (stoppedAtRef.current?.reason === "exception") {
          pushOutput("system", "Cannot step from an uncaught exception. Restart to run again.");
          return;
        }
        if (sessionStateRef.current !== "paused") {
          pushOutput("system", `Step to previous group ignored: debugger is ${sessionStateRef.current}, expected paused.`);
          return;
        }
        pushOutput("system", "Step-to-previous-group is not supported without reverse execution.");
      }),
    [enqueueAction, pushOutput],
  );

  const resume = useCallback(
    async () =>
      enqueueAction(async () => {
        if (stoppedAtRef.current?.reason === "exception") {
          pushOutput("system", "Cannot continue after an uncaught exception. Restart to run again.");
          return;
        }
        if (sessionStateRef.current !== "paused") {
          pushOutput("system", `Continue ignored: debugger is ${sessionStateRef.current}, expected paused.`);
          return;
        }
        try {
          await debugContinue();
        } catch (err) {
          await syncFromBackendState();
          pushOutput("error", `Continue failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput, syncFromBackendState],
  );

  const runToCursor = useCallback(
    async (file: string, line: number, command?: { offset: number; len: number }) =>
      enqueueAction(async () => {
        if (stoppedAtRef.current?.reason === "exception") {
          pushOutput("system", "Cannot continue after an uncaught exception. Restart to run again.");
          return;
        }
        if (sessionStateRef.current !== "paused") {
          pushOutput("system", `Run to cursor ignored: debugger is ${sessionStateRef.current}, expected paused.`);
          return;
        }
        if (!file || !Number.isFinite(line) || line <= 0) {
          pushOutput("system", "Run to cursor requires a valid file and line.");
          return;
        }

        const existing = breakpointsRef.current.find(
          (bp) =>
            isSourceBreakpoint(bp)
            && bp.file === file
            && bp.line === line
            && bp.offset === command?.offset,
        );
        const key = tempBreakpointKey(file, line, command?.offset);
        const createdTemp = !existing;

        if (createdTemp) {
          try {
            await debugSetBreakpoint(
              file,
              line,
              undefined,
              command?.offset,
              command?.len,
            );
            tempBreakpointsRef.current.set(key, {
              file,
              line,
              offset: command?.offset,
              len: command?.len,
            });
            setBreakpoints((bps) => {
              const withoutSame = bps.filter(
                (bp) =>
                  !isSourceBreakpoint(bp)
                  || bp.file !== file
                  || bp.line !== line
                  || bp.offset !== command?.offset,
              );
              return [
                ...withoutSame,
                {
                  file,
                  line,
                  offset: command?.offset,
                  len: command?.len,
                  enabled: true,
                },
              ];
            });
          } catch (err) {
            pushOutput("error", `Run to cursor failed to set breakpoint: ${err instanceof Error ? err.message : String(err)}`);
            return;
          }
        }

        try {
          await debugContinue();
          pushOutput("system", `Running to ${file.split("/").pop()}:${line}${createdTemp ? " (temporary breakpoint)" : ""}`);
        } catch (err) {
          if (createdTemp) {
            tempBreakpointsRef.current.delete(key);
            setBreakpoints((bps) =>
              bps.filter(
                (bp) =>
                  !isSourceBreakpoint(bp)
                  || bp.file !== file
                  || bp.line !== line
                  || bp.offset !== command?.offset,
              ));
            try {
              await debugClearBreakpoint(file, line, command?.offset);
            } catch {
              // Ignore cleanup errors on failed continue.
            }
          }
          await syncFromBackendState();
          pushOutput("error", `Run to cursor failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput, syncFromBackendState, tempBreakpointKey],
  );

  const pauseAction = useCallback(
    async () =>
      enqueueAction(async () => {
        if (sessionStateRef.current !== "running") {
          pushOutput("system", `Pause ignored: debugger is ${sessionStateRef.current}, expected running.`);
          return;
        }
        try {
          await debugPause();
          await syncFromBackendState();
        } catch (err) {
          await syncFromBackendState();
          pushOutput("error", `Pause failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput, syncFromBackendState],
  );

  const toggleBreakpoint = useCallback(
    async (file: string, line: number, command?: { offset: number; len: number }) =>
      enqueueAction(async () => {
        const existing = breakpointsRef.current.find(
          (bp) =>
            isSourceBreakpoint(bp)
            && bp.file === file
            && bp.line === line
            && bp.offset === command?.offset,
        );
        // When no debug session is active, just update local state.
        // Stored breakpoints are replayed to the backend on the next
        // successful `launch`. This lets the user click in the gutter to
        // set breakpoints BEFORE running the program — the typical IDE
        // workflow — without the click silently no-op'ing.
        const sessionActive = isActiveDebugSession(sessionStateRef.current);

        try {
          if (existing) {
            const next = { ...existing, enabled: !existing.enabled };
            if (sessionActive) {
              await debugSetBreakpointEnabled(file, line, next.enabled, command?.offset);
            }
            setBreakpoints((bps) =>
              bps.map((bp) => (
                isSourceBreakpoint(bp)
                && bp.file === file
                && bp.line === line
                && bp.offset === command?.offset
                  ? next
                  : bp
              )),
            );
          } else {
            if (sessionActive) {
              await debugSetBreakpoint(file, line, undefined, command?.offset, command?.len);
            }
            setBreakpoints((bps) => [
              ...bps,
              { file, line, offset: command?.offset, len: command?.len, enabled: true },
            ]);
          }
        } catch (err) {
          pushOutput("error", `Breakpoint toggle failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput],
  );

  const setBreakpointAction = useCallback(
    async (
      file: string,
      line: number,
      condition?: string,
      command?: { offset: number; len: number },
    ) =>
      enqueueAction(async () => {
        try {
          await debugSetBreakpoint(file, line, condition, command?.offset, command?.len);
          setBreakpoints((bps) => {
            const filtered = bps.filter(
              (bp) =>
                !isSourceBreakpoint(bp)
                || bp.file !== file
                || bp.line !== line
                || bp.offset !== command?.offset,
            );
            return [
              ...filtered,
              {
                file,
                line,
                offset: command?.offset,
                len: command?.len,
                condition,
                enabled: true,
              },
            ];
          });
        } catch (err) {
          pushOutput("error", `Set breakpoint failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput],
  );

  const clearBreakpointAction = useCallback(
    async (file: string, line: number, command?: { offset: number; len: number }) =>
      enqueueAction(async () => {
        // Mirror toggleBreakpoint's session handling: when no debug session
        // is active, just update local state so the dot disappears
        // immediately. The next launch won't replay this breakpoint since
        // it's no longer in the local set.
        const sessionActive = isActiveDebugSession(sessionStateRef.current);
        try {
          if (sessionActive) {
            await debugClearBreakpoint(file, line, command?.offset);
          }
          setBreakpoints((bps) =>
            bps.filter((bp) =>
              !isSourceBreakpoint(bp)
              || bp.file !== file
              || bp.line !== line
              || bp.offset !== command?.offset),
          );
        } catch (err) {
          pushOutput("error", `Clear breakpoint failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput],
  );

  const clearAllBreakpoints = useCallback(
    async () =>
      enqueueAction(async () => {
        const current = breakpointsRef.current.filter(isSourceBreakpoint);
        for (const bp of current) {
          try {
            await debugClearBreakpoint(bp.file, bp.line, bp.offset);
          } catch { /* ignore individual failures */ }
        }
        setBreakpoints([]);
      }),
    [enqueueAction],
  );

  const refreshBreakpoints = useCallback(
    async () =>
      enqueueAction(async () => {
        try {
          setBreakpoints(await debugListBreakpoints());
        } catch { /* ignore refresh failures */ }
      }),
    [enqueueAction],
  );

  const setWatchpointAction = useCallback(
    async (name: string, scope?: string, level?: number, condition?: string) =>
      enqueueAction(async () => {
        try {
          await debugSetWatchpoint(name, scope, level, condition);
          setWatchpoints(await debugListWatchpoints());
        } catch (err) {
          pushOutput("error", `Set watchpoint failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput],
  );

  const clearWatchpointAction = useCallback(
    async (name: string) =>
      enqueueAction(async () => {
        try {
          await debugClearWatchpoint(name);
          setWatchpoints(await debugListWatchpoints());
        } catch (err) {
          pushOutput("error", `Clear watchpoint failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput],
  );

  const clearAllWatchpointsAction = useCallback(
    async () =>
      enqueueAction(async () => {
        try {
          await debugClearAllWatchpoints();
          setWatchpoints([]);
          setWatchpointHit(null);
        } catch (err) {
          pushOutput("error", `Clear watchpoints failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput],
  );

  const refreshWatchpoints = useCallback(
    async () =>
      enqueueAction(async () => {
        try {
          setWatchpoints(await debugListWatchpoints());
        } catch { /* ignore refresh failures */ }
      }),
    [enqueueAction],
  );

  const evalScript = useCallback(
    async (script: string, level: number = 1) =>
      enqueueAction(async () => {
        setEvalHistory((h) => [...h, script]);
        setOutput((o) => appendOutput(o, { stream: "eval", text: `> ${script}`, timestamp: Date.now() }));
        try {
          await debugEval(script, level);
        } catch (err) {
          pushOutput("error", `Eval failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput],
  );

  const fetchVariables = useCallback(
    async (level: number = 1) =>
      enqueueAction(async () => {
        try { await debugGetVariables(level); } catch { /* ignore */ }
      }),
    [enqueueAction],
  );

  const fetchStack = useCallback(
    async () =>
      enqueueAction(async () => {
        try { await debugGetStack(); } catch { /* ignore */ }
      }),
    [enqueueAction],
  );

  const setVariableAction = useCallback(
    async (name: string, value: string, level: number = 1) =>
      enqueueAction(async () => {
        try {
          await debugSetVariable(name, value, level);
          await debugGetVariables(level);
        } catch (err) {
          pushOutput("error", `Set variable failed: ${err instanceof Error ? err.message : String(err)}`);
        }
      }),
    [enqueueAction, pushOutput],
  );

  const clearOutput = useCallback(() => {
    setOutput([]);
  }, []);

  const value = useMemo(
    (): DebugStore => ({
      sessionState,
      isBusy,
      debuggerAvailable,
      scriptPath,
      tclVersion,
      debugCapabilities,
      canStepInstruction,
      pid,
      stoppedAt,
      variables,
      variablesLevel,
      stack,
      output,
      breakpoints,
      watchpoints,
      watchpointHit,
      evalHistory,
      disassemblyText,
      disassemblyHeader,
      liveDecompileText,
      bytecodeChunks,
      launch,
      stop,
      stepInto,
      stepInstruction,
      stepOverInstruction,
      stepOutInstruction,
      runToPc,
      stepOver,
      stepOut,
      stepToNextGroup,
      stepToPrevGroup,
      resume,
      runToCursor,
      pause: pauseAction,
      toggleBreakpoint,
      setBreakpoint: setBreakpointAction,
      clearBreakpoint: clearBreakpointAction,
      clearAllBreakpoints,
      refreshBreakpoints,
      setWatchpoint: setWatchpointAction,
      clearWatchpoint: clearWatchpointAction,
      clearAllWatchpoints: clearAllWatchpointsAction,
      refreshWatchpoints,
      evalScript,
      fetchVariables,
      fetchStack,
      setVariable: setVariableAction,
      clearOutput,
      resetSession,
      restart,
    }),
    [
      sessionState,
      isBusy,
      debuggerAvailable,
      scriptPath,
      tclVersion,
      debugCapabilities,
      canStepInstruction,
      pid,
      stoppedAt,
      variables,
      variablesLevel,
      stack,
      output,
      breakpoints,
      watchpoints,
      watchpointHit,
      evalHistory,
      disassemblyText,
      disassemblyHeader,
      liveDecompileText,
      bytecodeChunks,
      launch,
      stop,
      stepInto,
      stepInstruction,
      stepOverInstruction,
      stepOutInstruction,
      runToPc,
      stepOver,
      stepOut,
      stepToNextGroup,
      stepToPrevGroup,
      resume,
      runToCursor,
      pauseAction,
      toggleBreakpoint,
      setBreakpointAction,
      clearBreakpointAction,
      clearAllBreakpoints,
      refreshBreakpoints,
      setWatchpointAction,
      clearWatchpointAction,
      clearAllWatchpointsAction,
      refreshWatchpoints,
      evalScript,
      fetchVariables,
      fetchStack,
      setVariableAction,
      clearOutput,
      resetSession,
      restart,
    ],
  );

  return (
    <DebugContext.Provider value={value}>{children}</DebugContext.Provider>
  );
}

export function useDebugStore(): DebugStore {
  const ctx = useContext(DebugContext);
  if (!ctx) throw new Error("useDebugStore must be used within DebugStoreProvider");
  return ctx;
}
