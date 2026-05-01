"use client";

import React, {
  useState,
  useCallback,
  useMemo,
  useRef,
  useEffect,
  useSyncExternalStore,
} from "react";
import clsx from "clsx";
import { useTcldStore } from "@/lib/store";
import { useExplorer } from "@/hooks/use-explorer";
import { getLeafIds, getOpenViews, LAYOUT_PRESETS } from "@/lib/default-layouts";
import { useCommands } from "@/hooks/use-commands";
import { commandRegistry, type ChordHint, type CommandContext, type CommandDef } from "@/lib/command-registry";
import {
  loadSettings,
  loadSettingsWithTauriFallback,
  mergeAppearanceSettings,
  saveSettings,
  saveSettingsWithTauriFallback,
  applySettings,
  changeFontSize,
  resetFontSize,
  setDebugHookMode,
  setDebugHookCommandPreset,
  describeDebugHookSettings,
  serializeSettingsForTauri,
  settingsConfigStorageContract,
  setDensity,
} from "@/lib/settings";
import type { Settings, DensityMode, SerializedAppearanceSettings } from "@/lib/settings";
import type { ThemeId, FontId } from "@/lib/themes";
import { THEMES, FONTS } from "@/lib/themes";

// Chrome
import { FileHeader } from "@/components/chrome/FileHeader";
import { NavHistoryControls } from "@/components/chrome/NavHistoryControls";
import { StatusBar } from "@/components/chrome/StatusBar";
import { CommandLine } from "@/components/chrome/CommandLine";

// Workspace
import { MosaicWorkspace } from "@/components/MosaicWorkspace";
import type { InstructionPresentationMode } from "@/components/panels/InstructionPanel";
import { WelcomeScreen } from "@/components/WelcomeScreen";

// Overlays
import { CommandPalette } from "@/components/overlays/CommandPalette";
import { HelpOverlay } from "@/components/overlays/HelpOverlay";

// Debug
import { DebugToolbar } from "@/components/chrome/DebugToolbar";
import { MainToolbar } from "@/components/chrome/MainToolbar";
import { LaunchControl } from "@/components/chrome/LaunchControl";
import { SectionEyebrow } from "@/components/primitives";
import { useDebugStore } from "@/lib/debug-store";
import { DEBUGGER_UNAVAILABLE_MESSAGE } from "@/lib/capabilities";
import {
  foldTclSource,
  formatTclSource,
  isTauri,
  listen,
  openRunConfigurationWindow,
  openPreferencesWindow,
  openScriptDialog,
  planTclFormatEdits,
  saveFileDialog,
  saveTextFile,
  emit as emitTauriEvent,
} from "@/lib/tauri-api";
import { addRecentFile, loadRecentFiles, type RecentFile } from "@/lib/recent-files";
import { useLocationStore } from "@/lib/location-store";
import { computeScopeId } from "@/lib/location-index";
import { buildFullReport, resolveBreakpointTarget } from "@/lib/workflow-contracts";
import { formatOutputForCopy } from "@/lib/output-search";
import {
  applyFormattedTextToData,
  buildNativeMenuAvailabilityPayload,
  buildExportRequests,
  executeNativeMenuAction,
  resolveDroppedFile,
  summarizeFormatPreview,
} from "@/lib/gui-workflows";
import {
  buildRecentRunConfigurations,
  buildRecentRunConfigurationMenuItems,
  buildRunConfigurationLaunchRequest,
  createRunConfigurationStore,
  importRunConfigurationsJson,
  newRunConfigurationProfile,
  type RecentRunConfiguration,
  type RunConfigurationProfile,
  type RunConfigurationState,
} from "@/lib/run-configuration-store";
import { createKeybindingStore } from "@/lib/keybindings";
import { dispatchPanelAction } from "@/lib/panel-action-bus";
import { defaultNativeMenuCoverageContract } from "@/lib/action-inventory";
import { clearSavedLayout } from "@/lib/layout-storage";
import { DESIGN_PREFERENCE_SECTION_MAPPINGS } from "@/lib/design-surface-contract";
import type { ViewId } from "@/types";
import {
  AlertTriangle,
  Bug,
  Command,
  Cpu,
  FileCode2,
  Layers,
  Play,
  Plus,
  Settings2,
  type LucideIcon,
} from "lucide-react";

function browserStorage() {
  const memory = new Map<string, string>();
  return {
    getItem: (key: string) => {
      if (typeof window === "undefined") return memory.get(key) ?? null;
      return window.localStorage.getItem(key);
    },
    setItem: (key: string, value: string) => {
      if (typeof window === "undefined") {
        memory.set(key, value);
        return;
      }
      window.localStorage.setItem(key, value);
    },
    removeItem: (key: string) => {
      if (typeof window === "undefined") {
        memory.delete(key);
        return;
      }
      window.localStorage.removeItem(key);
    },
  };
}

type WindowRouteState = {
  windowMode: string;
  visualParityState: string | null;
};

const defaultWindowRouteState: WindowRouteState = {
  windowMode: "main",
  visualParityState: null,
};

function readWindowRouteState(search: string): WindowRouteState {
  const params = new URLSearchParams(search);
  return {
    windowMode: params.get("window") ?? defaultWindowRouteState.windowMode,
    visualParityState: params.get("parity"),
  };
}

function subscribeWindowRoute(callback: () => void): () => void {
  window.addEventListener("popstate", callback);
  return () => window.removeEventListener("popstate", callback);
}

function currentWindowSearch(): string {
  return window.location.search;
}

function serverWindowSearch(): string {
  return "";
}

function describeAppearanceSettings(settings: Settings): string {
  return `${THEMES[settings.theme].label} · ${FONTS[settings.fontFamily].label} · ${settings.fontSize}px · ${settings.density}`;
}

export default function TcldExplorer() {
  const search = useSyncExternalStore(
    subscribeWindowRoute,
    currentWindowSearch,
    serverWindowSearch,
  );
  const routeState = useMemo(() => readWindowRouteState(search), [search]);
  const { windowMode, visualParityState } = routeState;
  if (windowMode === "preferences") return <PreferencesWindowApp />;
  if (windowMode === "run-configuration") return <RunConfigurationWindowApp />;
  return <ExplorerApp visualParityState={visualParityState} />;
}

function ExplorerApp({ visualParityState }: { visualParityState: string | null }) {
  const store = useTcldStore();
  const {
    setMark,
    goToMark,
    clearMark,
    togglePinA,
    togglePinB,
    setPinA,
    setPinB,
    pinA,
    pinB,
    marks,
    index,
    execution,
    historyBack,
    historyForward,
    goBackTo,
    cursor,
  } = useLocationStore();
  const {
    state,
    setState,
    addView,
    removeView,
    setMosaicNode,
    stackLeafIntoTabs,
    moveLeafToSplit,
    moveLeafToWindowEdge,
    stackViewIntoLeaf,
    reorderTabInLeaf,
    addViewToLeaf,
    swapViewInLeaf,
    splitViewIntoLeaf,
    splitViewToWindowEdge,
    setFocusedView,
    setSelected,
    loadPreset,
    saveLayout,
    toggleExpand,
    togglePanel,
    toggleTrace,
    cycleFocus,
    cycleTabInLeaf,
    closeFocusedTab,
    closeFocusedLeaf,
    splitFocusedToEdge,
    cycleLayoutPreset,
    extractLeafToFlyout,
    restoreFlyoutLeaf,
    setFlyoutOpen,
    goBack,
    goForward,
    navigateGroup,
    closePalette,
    openPalette,
    openHelp,
    closeHelp,
    openCommand,
    closeCommand,
    handleOpenFile,
    executeCommand,
    omniboxResults,
    executeOmniboxResult,
    handleToggleBookmark,
    handleSetComment,
    toggleJumpArrows,
  } = useExplorer();

  const debug = useDebugStore();
  const lastStopKeyRef = useRef<string | null>(null);

  useEffect(() => {
    const stopped = debug.stoppedAt;
    if (!stopped) return;
    const key = `${stopped.reason}:${stopped.file}:${stopped.line}:${debug.sessionState}`;
    if (lastStopKeyRef.current === key) return;
    lastStopKeyRef.current = key;

    if (stopped.reason === "exception") {
      addView("console");
    } else if (stopped.reason === "breakpoint") {
      addView("breakpoints");
    } else if (stopped.reason === "watchpoint") {
      addView("variables");
    }
  }, [addView, debug.sessionState, debug.stoppedAt]);

  // ─── Debug-mode condensed chrome ─────────────────────────────
  useEffect(() => {
    const active = debug.sessionState !== "idle" && debug.sessionState !== "terminated";
    document.documentElement.toggleAttribute("data-debug", active);
  }, [debug.sessionState]);

  // ─── Settings ────────────────────────────────────────────────

  const [settings, setSettingsValue] = useState<Settings>(() => loadSettings());
  const [selectedInstructionPresentationMode, setInstructionPresentationMode] =
    useState<InstructionPresentationMode>("ir");
  const instructionPresentationMode =
    visualParityState === "instructions"
      ? "instructions"
      : selectedInstructionPresentationMode;
  const [runConfigurationStore] = useState(() => createRunConfigurationStore({ storage: browserStorage(), settings }));
  const [runConfigurationSnapshot, setRunConfigurationSnapshot] = useState<RunConfigurationState>(() => runConfigurationStore.snapshot());
  const [keybindingStore] = useState(() => createKeybindingStore({ storage: browserStorage(), actions: [] }));
  const [keybindingRevision, setKeybindingRevision] = useState(0);

  const refreshRunConfigurationSnapshot = useCallback(() => {
    setRunConfigurationSnapshot(runConfigurationStore.snapshot());
  }, [runConfigurationStore]);

  const applyKeybindingOverrides = useCallback(() => {
    commandRegistry.setShortcutOverrides(keybindingStore.shortcutOverrides());
  }, [keybindingStore]);

  useEffect(() => {
    applySettings(settings);
  }, [settings]);

  useEffect(() => {
    let cancelled = false;
    loadSettingsWithTauriFallback().then((next) => {
      if (cancelled) return;
      setSettingsValue(next);
      applySettings(next);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  const updateSettings = useCallback(
    (next: Settings, statusMessage?: string) => {
      setSettingsValue(next);
      saveSettings(next);
      void saveSettingsWithTauriFallback(next)
        .then(() => emitTauriEvent(settingsConfigStorageContract.tauriEvents.changed, serializeSettingsForTauri(next)))
        .catch(() => {});
      applySettings(next);
      const defaultMessage = describeAppearanceSettings(next);
      setState((s) => ({ ...s, statusMessage: statusMessage ?? defaultMessage }));
    },
    [setState],
  );

  const launchDebugPath = useCallback(async (path: string) => {
    // Debugger-less backend: refuse before any layout flip / file dialog so
    // native-menu or recent-file dispatches degrade to a status message
    // (the debug store repeats the refusal if reached directly).
    if (!debug.debuggerAvailable) {
      setState((s) => ({ ...s, statusMessage: DEBUGGER_UNAVAILABLE_MESSAGE }));
      return false;
    }
    loadPreset("debug");
    const request = buildRunConfigurationLaunchRequest(runConfigurationSnapshot, settings, {
      id: `script:${path}`,
      label: path.split("/").pop() ?? path,
      actionId: `run-configuration:recent:script:${path}`,
      itemId: `run_recent_configuration:script:${path}`,
      kind: "script",
      path,
    });
    if (!request) return false;
    // debug.launch() hydrates the static workspace itself (in parallel with
    // the live session) so the Source View / Disassembly panels populate even
    // when the target pauses before any bytecode chunk streams. See the
    // loadStaticFile call in DebugStoreProvider.launch.
    const ok = await debug.launch(request.path, request.args, request.tclshPath, request.launchOptions);
    if (!ok) {
      loadPreset("default");
      return false;
    }

    addRecentFile({ path, filename: path.split("/").pop() ?? path, mode: "debug" });
    return true;
  }, [debug, loadPreset, runConfigurationSnapshot, settings, setState]);

  const handleLaunchDebug = useCallback(async () => {
    if (!debug.debuggerAvailable) {
      setState((s) => ({ ...s, statusMessage: DEBUGGER_UNAVAILABLE_MESSAGE }));
      return;
    }
    const path = await openScriptDialog();
    if (!path) return;
    await launchDebugPath(path);
  }, [debug.debuggerAvailable, launchDebugPath, setState]);

  const launchSelectedProfile = useCallback(async () => {
    if (!debug.debuggerAvailable) {
      setState((s) => ({ ...s, statusMessage: DEBUGGER_UNAVAILABLE_MESSAGE }));
      return false;
    }
    const request = buildRunConfigurationLaunchRequest(runConfigurationSnapshot, settings);
    if (!request) {
      setState((s) => ({ ...s, statusMessage: "Select a run configuration before launching." }));
      return false;
    }
    loadPreset("debug");
    const ok = await debug.launch(request.path, request.args, request.tclshPath, request.launchOptions);
    if (!ok) {
      loadPreset("default");
      return false;
    }
    addRecentFile({ path: request.path, filename: request.path.split("/").pop() ?? request.path, mode: "debug" });
    return true;
  }, [debug, loadPreset, runConfigurationSnapshot, settings, setState]);

  const launchRecentConfiguration = useCallback(async (recent: RecentRunConfiguration) => {
    if (!debug.debuggerAvailable) {
      setState((s) => ({ ...s, statusMessage: DEBUGGER_UNAVAILABLE_MESSAGE }));
      return false;
    }
    const request = buildRunConfigurationLaunchRequest(runConfigurationSnapshot, settings, recent);
    if (!request) {
      setState((s) => ({ ...s, statusMessage: "That recent run configuration is no longer available." }));
      return false;
    }
    loadPreset("debug");
    const ok = await debug.launch(request.path, request.args, request.tclshPath, request.launchOptions);
    if (!ok) {
      loadPreset("default");
      return false;
    }
    addRecentFile({ path: request.path, filename: request.path.split("/").pop() ?? request.path, mode: "debug" });
    return true;
  }, [debug, loadPreset, runConfigurationSnapshot, settings, setState]);

  const openRunConfigurationEditor = useCallback(async () => {
    try {
      await openRunConfigurationWindow();
      const selected = runConfigurationSnapshot.profiles.find((profile) => profile.id === runConfigurationSnapshot.selectedProfileId) ?? null;
      await emitTauriEvent("run-configuration:edit", selected);
    } catch {
      setState((s) => ({ ...s, statusMessage: "Run configurations open in the desktop application." }));
    }
  }, [runConfigurationSnapshot.profiles, runConfigurationSnapshot.selectedProfileId, setState]);

  const recentRunConfigurations = useMemo(
    () => buildRecentRunConfigurations(runConfigurationSnapshot, loadRecentFiles()),
    [runConfigurationSnapshot],
  );

  const recentRunConfigurationMenuItems = useMemo(
    () => buildRecentRunConfigurationMenuItems(recentRunConfigurations),
    [recentRunConfigurations],
  );

  const recentRunConfigurationActionByMenuItem = useMemo(
    () => new Map(
      recentRunConfigurationMenuItems
        .filter((item) => item.enabled && item.actionId)
        .map((item) => [item.itemId, item.actionId!]),
    ),
    [recentRunConfigurationMenuItems],
  );

  const selectedSourceLine = useMemo(() => {
    const data = store.data;
    if (!data) return null;
    const inst = data.instructions[cursor.instIdx ?? state.selectedIndex];
    if (!inst || inst.group == null) return null;

    const lineIdx = data.decompileLines.findIndex((line) => line.group === inst.group);
    if (lineIdx < 0) return null;
    const line = data.decompileLines[lineIdx];
    return line.sourceLineStart ?? (lineIdx + 1);
  }, [cursor.instIdx, state.selectedIndex, store.data]);

  const selectedCommandRange = useMemo(() => {
    const data = store.data;
    if (!data) return undefined;
    const inst = data.instructions[cursor.instIdx ?? state.selectedIndex];
    if (inst?.group == null) return undefined;
    const span = data.sourceMap?.find(
      (entry) => entry.commandGroup === inst.group && entry.originalSpan != null,
    )?.originalSpan;
    if (!span) return undefined;
    return { offset: span[0], len: span[1] - span[0] };
  }, [cursor.instIdx, state.selectedIndex, store.data]);

  const activeDocumentScopeId = useMemo(
    () => store.data ? computeScopeId(store.data.metadata) : null,
    [store.data],
  );

  useEffect(() => {
    if (
      activeDocumentScopeId
      && activeDocumentScopeId !== cursor.scopeId
    ) {
      return;
    }
    store.updateActiveDocumentWorkspaceState({
      selectedInst: cursor.instIdx ?? state.selectedIndex,
      activeLine: selectedSourceLine,
      debuggerState: {
        sessionState: debug.sessionState,
        scriptPath: debug.scriptPath,
        stoppedFile: debug.stoppedAt?.file ?? null,
        stoppedLine: debug.stoppedAt?.line ?? null,
      },
    });
  }, [
    activeDocumentScopeId,
    cursor.instIdx,
    cursor.scopeId,
    debug.scriptPath,
    debug.sessionState,
    debug.stoppedAt?.file,
    debug.stoppedAt?.line,
    selectedSourceLine,
    state.selectedIndex,
    store,
  ]);

  const runToSelectedCursor = useCallback(() => {
    if (!debug.scriptPath) {
      setState((s) => ({ ...s, statusMessage: "Run to cursor requires an active debug script." }));
      return;
    }
    if (selectedSourceLine == null) {
      setState((s) => ({ ...s, statusMessage: "The current cursor has no source line." }));
      return;
    }
    void debug.runToCursor(
      debug.scriptPath,
      selectedSourceLine,
      selectedCommandRange,
    );
  }, [debug, selectedCommandRange, selectedSourceLine, setState]);

  const runToSelectedPc = useCallback(() => {
    if (cursor.pc == null) {
      setState((s) => ({ ...s, statusMessage: "The current instruction has no program counter." }));
      return;
    }
    void debug.runToPc(cursor.pc);
  }, [cursor.pc, debug, setState]);

  const toggleBreakpointAtCursor = useCallback(() => {
    const target = resolveBreakpointTarget({
      data: store.data,
      selectedInstructionIndex: cursor.instIdx ?? state.selectedIndex,
      stoppedAt: debug.stoppedAt,
      scriptPath: debug.scriptPath,
    });
    if (target) {
      void debug.toggleBreakpoint(target.file, target.line);
    } else {
      setState((s) => ({ ...s, statusMessage: "The current location cannot hold a source breakpoint." }));
    }
  }, [cursor.instIdx, debug, state.selectedIndex, store.data, setState]);

  const writeNamedOutput = useCallback(async (name: string, contents: string) => {
    const path = await saveFileDialog(name);
    if (!path) return;
    await saveTextFile(path, contents);
    setState((s) => ({ ...s, statusMessage: `saved ${path}` }));
  }, [setState]);

  const fullReportText = useCallback(() => {
    if (!store.data) return "";
    return buildFullReport({
      data: store.data,
      session: store.session,
      output: debug.output,
      stoppedAt: debug.stoppedAt,
    });
  }, [debug.output, debug.stoppedAt, store.data, store.session]);

  const historyItems = useMemo(() => {
    const instructions = store.data?.instructions ?? [];
    return historyBack
      .slice(-10)
      .map((loc, idx, arr) => {
        const offset = arr.length - idx;
        const inst = loc.instIdx != null ? instructions[loc.instIdx] : undefined;
        const group = loc.group != null
          ? `c${String(loc.group).padStart(3, "0")}`
          : "c---";
        const instLabel = loc.instIdx != null
          ? `i${String(loc.instIdx).padStart(3, "0")}`
          : "i---";
        const pcLabel = loc.pc != null
          ? `0x${loc.pc.toString(16).padStart(4, "0")}`
          : "0x----";
        const op = inst ? `${inst.op} ${inst.args}`.trim() : "unknown instruction";
        return {
          offset,
          label: `${group} ${instLabel} ${pcLabel}`,
          detail: op,
        };
      })
      .reverse();
  }, [historyBack, store.data?.instructions]);

  const handleGoBackTo = useCallback((offset: number) => {
    const moved = goBackTo(offset);
    if (!moved) {
      setState((s) => ({ ...s, statusMessage: "That history location is no longer available." }));
    }
  }, [goBackTo, setState]);

  const executionScopeMismatch = Boolean(
    execution
    && index
    && execution.scopeId
    && execution.scopeId !== index.scopeId
  );

  const pinADisplay = useMemo(() => {
    if (pinA?.instIdx == null) return null;
    const inst = store.data?.instructions?.[pinA.instIdx];
    const group = pinA.group != null ? `c${String(pinA.group).padStart(3, "0")}` : "c---";
    const instLabel = `i${String(pinA.instIdx).padStart(3, "0")}`;
    const pcLabel = pinA.pc != null ? `0x${pinA.pc.toString(16).padStart(4, "0")}` : "0x----";
    return {
      label: `${group} ${instLabel} ${pcLabel}`,
      detail: inst ? `${inst.op} ${inst.args}`.trim() : undefined,
    };
  }, [pinA, store.data?.instructions]);

  const pinBDisplay = useMemo(() => {
    if (pinB?.instIdx == null) return null;
    const inst = store.data?.instructions?.[pinB.instIdx];
    const group = pinB.group != null ? `c${String(pinB.group).padStart(3, "0")}` : "c---";
    const instLabel = `i${String(pinB.instIdx).padStart(3, "0")}`;
    const pcLabel = pinB.pc != null ? `0x${pinB.pc.toString(16).padStart(4, "0")}` : "0x----";
    return {
      label: `${group} ${instLabel} ${pcLabel}`,
      detail: inst ? `${inst.op} ${inst.args}`.trim() : undefined,
    };
  }, [pinB, store.data?.instructions]);

  const markItems = useMemo(() => {
    const instructions = store.data?.instructions ?? [];
    return Object.entries(marks)
      .filter(([, loc]) => loc.instIdx != null)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([name, loc]) => {
        const inst = loc.instIdx != null ? instructions[loc.instIdx] : undefined;
        const group = loc.group != null ? `c${String(loc.group).padStart(3, "0")}` : "c---";
        const instLabel = loc.instIdx != null ? `i${String(loc.instIdx).padStart(3, "0")}` : "i---";
        const pcLabel = loc.pc != null ? `0x${loc.pc.toString(16).padStart(4, "0")}` : "0x----";
        return {
          name,
          label: `${group} ${instLabel} ${pcLabel}`,
          detail: inst ? `${inst.op} ${inst.args}`.trim() : undefined,
        };
      });
  }, [marks, store.data?.instructions]);

  const handleJumpPinA = useCallback(() => {
    if (pinA?.instIdx == null) {
      setState((s) => ({ ...s, statusMessage: "Pin A is not set." }));
      return;
    }
    setSelected(pinA.instIdx);
  }, [pinA, setSelected, setState]);

  const handleJumpPinB = useCallback(() => {
    if (pinB?.instIdx == null) {
      setState((s) => ({ ...s, statusMessage: "Pin B is not set." }));
      return;
    }
    setSelected(pinB.instIdx);
  }, [pinB, setSelected, setState]);

  const handleJumpMark = useCallback((name: string) => {
    const ok = goToMark(name);
    if (!ok) {
      setState((s) => ({ ...s, statusMessage: `mark ${name} not set` }));
    }
  }, [goToMark, setState]);

  const commands = useMemo<CommandDef[]>(() => {
    const panelToggleCommands: CommandDef[] = [
      { id: "panel:instructions", title: "Disassembly", category: "Panels", shortcut: ["i"], when: "!overlayOpen,hasFile", run: () => togglePanel("instructions") },
      { id: "panel:decompile", title: "Decompile", category: "Panels", shortcut: ["d"], when: "!overlayOpen,hasFile", run: () => togglePanel("decompile") },
      { id: "panel:source", title: "Source", category: "Panels", when: "hasFile", run: () => togglePanel("source") },
      { id: "panel:comparison", title: "Compare", category: "Panels", when: "hasFile", run: () => togglePanel("comparison") },
      { id: "panel:diagnostics", title: "Diagnostics", category: "Panels", when: "hasFile", run: () => togglePanel("diagnostics") },
      { id: "panel:provenance", title: "Provenance", category: "Panels", when: "hasFile", run: () => togglePanel("provenance") },
      { id: "panel:project", title: "Project", category: "Panels", when: "hasFile", run: () => togglePanel("project") },
      { id: "panel:detail", title: "Detail", category: "Panels", when: "hasFile", run: () => togglePanel("detail") },
      { id: "panel:literals", title: "Literals", category: "Panels", shortcut: ["l"], when: "!overlayOpen,hasFile", run: () => togglePanel("literals") },
      { id: "panel:exceptions", title: "Exceptions", category: "Panels", shortcut: ["e"], when: "!overlayOpen,hasFile", run: () => togglePanel("exceptions") },
      { id: "panel:graph", title: "Graph", category: "Panels", shortcut: ["x"], when: "!overlayOpen,hasFile", run: () => togglePanel("graph") },
      { id: "panel:summary", title: "Summary", category: "Panels", shortcut: ["s"], when: "!overlayOpen,hasFile", run: () => togglePanel("summary") },
      { id: "panel:variables", title: "Variables", category: "Debug Panels", run: () => togglePanel("variables") },
      { id: "panel:stack", title: "Call Stack", category: "Debug Panels", run: () => togglePanel("stack") },
      { id: "panel:console", title: "Console", category: "Debug Panels", run: () => togglePanel("console") },
      { id: "panel:breakpoints", title: "Breakpoints", category: "Debug Panels", run: () => togglePanel("breakpoints") },
      { id: "panel:watchpoints", title: "Watchpoints", category: "Debug Panels", run: () => togglePanel("watchpoints") },
      { id: "panel:command_resolver", title: "Command Resolver", category: "Debug Panels", run: () => togglePanel("commandResolver") },
    ];

    const layoutCommands: CommandDef[] = LAYOUT_PRESETS.map((preset) => ({
      id: `layout:${preset.name}`,
      title: `Layout: ${preset.label}`,
      category: "Layout",
      run: () => loadPreset(preset.name),
    }));

    const themeCommands: CommandDef[] = (Object.keys(THEMES) as ThemeId[]).map((themeId) => ({
      id: `theme:${themeId}`,
      title: `Theme: ${THEMES[themeId].label}`,
      category: "Settings",
      run: () => updateSettings({ ...settings, theme: themeId }),
    }));

    const fontCommands: CommandDef[] = (Object.keys(FONTS) as FontId[]).map((fontId) => ({
      id: `font:${fontId}`,
      title: `Font: ${FONTS[fontId].label}`,
      category: "Settings",
      run: () => updateSettings({ ...settings, fontFamily: fontId, typeSystem: fontId }),
    }));

    const densityCommands: CommandDef[] = (["regular", "compact", "comfy"] as DensityMode[]).map((density) => ({
      id: `density:${density}`,
      title: `Density: ${density[0].toUpperCase()}${density.slice(1)}`,
      category: "Settings",
      run: () => updateSettings(setDensity(settings, density)),
    }));

    const markCommands: CommandDef[] = "abcdefghijklmnopqrstuvwxyz".split("").flatMap((letter) => ([
      {
        id: `mark:set:${letter}`,
        title: `Mark: Set ${letter.toUpperCase()}`,
        category: "Navigation",
        shortcut: [`m ${letter}`],
        when: "!overlayOpen",
        run: () => {
          const ok = setMark(letter);
          setState((prev) => ({ ...prev, statusMessage: ok ? `mark ${letter} set` : `mark ${letter} unavailable` }));
        },
      },
      {
        id: `mark:goto:${letter}`,
        title: `Mark: Go To ${letter.toUpperCase()}`,
        category: "Navigation",
        shortcut: [`' ${letter}`],
        when: "!overlayOpen",
        run: () => {
          const ok = goToMark(letter);
          setState((prev) => ({ ...prev, statusMessage: ok ? `jumped to mark ${letter}` : `mark ${letter} not set` }));
        },
      },
      {
        id: `mark:clear:${letter}`,
        title: `Mark: Clear ${letter.toUpperCase()}`,
        category: "Navigation",
        run: () => {
          clearMark(letter);
          setState((prev) => ({ ...prev, statusMessage: `mark ${letter} cleared` }));
        },
      },
    ]));

    const pinCommands: CommandDef[] = [
      {
        id: "pin:toggle_a",
        title: "Pins: Toggle A",
        category: "Navigation",
        shortcut: ["p a"],
        when: "!overlayOpen",
        run: () => {
          const ok = togglePinA();
          setState((prev) => ({
            ...prev,
            statusMessage: ok
              ? (pinA?.instIdx === state.selectedIndex ? "pin A cleared" : "pin A set")
              : "pin A unavailable",
          }));
        },
      },
      {
        id: "pin:goto_a",
        title: "Pins: Go To A",
        category: "Navigation",
        run: () => {
          if (pinA?.instIdx != null) {
            setSelected(pinA.instIdx);
          } else {
            setState((prev) => ({ ...prev, statusMessage: "Pin A is not set." }));
          }
        },
      },
      {
        id: "pin:toggle_b",
        title: "Pins: Toggle B",
        category: "Navigation",
        shortcut: ["p b"],
        when: "!overlayOpen",
        run: () => {
          const ok = togglePinB();
          setState((prev) => ({
            ...prev,
            statusMessage: ok
              ? (pinB?.instIdx === state.selectedIndex ? "pin B cleared" : "pin B set")
              : "pin B unavailable",
          }));
        },
      },
      {
        id: "pin:goto_b",
        title: "Pins: Go To B",
        category: "Navigation",
        run: () => {
          if (pinB?.instIdx != null) {
            setSelected(pinB.instIdx);
          } else {
            setState((prev) => ({ ...prev, statusMessage: "Pin B is not set." }));
          }
        },
      },
      {
        id: "pin:clear_a",
        title: "Pins: Clear A",
        category: "Navigation",
        run: () => {
          setPinA(null);
          setState((prev) => ({ ...prev, statusMessage: "Pin A cleared." }));
        },
      },
      {
        id: "pin:clear_b",
        title: "Pins: Clear B",
        category: "Navigation",
        run: () => {
          setPinB(null);
          setState((prev) => ({ ...prev, statusMessage: "Pin B cleared." }));
        },
      },
    ];

    const debugHookCommands: CommandDef[] = [
      {
        id: "debug:hooks_mode_default",
        title: "Debug Hooks: Mode Default",
        category: "Debug Settings",
        run: () => {
          const next = setDebugHookMode(settings, "backend-default");
          updateSettings(next, describeDebugHookSettings(next));
        },
      },
      {
        id: "debug:hooks_mode_passthrough",
        title: "Debug Hooks: Mode Passthrough",
        category: "Debug Settings",
        run: () => {
          const next = setDebugHookMode(settings, "passthrough");
          updateSettings(next, describeDebugHookSettings(next));
        },
      },
      {
        id: "debug:hooks_mode_intercept",
        title: "Debug Hooks: Mode Intercept",
        category: "Debug Settings",
        run: () => {
          const next = setDebugHookMode(settings, "intercept");
          updateSettings(next, describeDebugHookSettings(next));
        },
      },
      {
        id: "debug:hooks_cmd_default",
        title: "Debug Hooks: Commands Default",
        category: "Debug Settings",
        run: () => {
          const next = setDebugHookCommandPreset(settings, "backend-default");
          updateSettings(next, describeDebugHookSettings(next));
        },
      },
      {
        id: "debug:hooks_cmd_tbcload",
        title: "Debug Hooks: Commands Tbcload",
        category: "Debug Settings",
        run: () => {
          const next = setDebugHookCommandPreset(settings, "tbcload");
          updateSettings(next, describeDebugHookSettings(next));
        },
      },
      {
        id: "debug:hooks_cmd_runtime",
        title: "Debug Hooks: Commands Runtime",
        category: "Debug Settings",
        run: () => {
          const next = setDebugHookCommandPreset(settings, "runtime");
          updateSettings(next, describeDebugHookSettings(next));
        },
      },
      {
        id: "debug:hooks_cmd_all",
        title: "Debug Hooks: Commands All",
        category: "Debug Settings",
        run: () => {
          const next = setDebugHookCommandPreset(settings, "all");
          updateSettings(next, describeDebugHookSettings(next));
        },
      },
    ];

    const recentRunConfigurationCommands: CommandDef[] = recentRunConfigurations.map((recent) => ({
      id: recent.actionId,
      title: `Run Recent: ${recent.label}`,
      category: "Run Configuration",
      run: () => {
        void launchRecentConfiguration(recent);
      },
    }));

    const commandList: CommandDef[] = [
      {
        id: "open",
        title: "Open File…",
        category: "File",
        shortcut: ["Mod+o"],
        allowInInput: true,
        run: () => { void handleOpenFile(); },
      },
      {
        id: "toolbar:open-file",
        title: "Toolbar: Open File",
        category: "Toolbar",
        run: () => { void handleOpenFile(); },
      },
      {
        id: "toolbar:debug-launch",
        title: "Toolbar: Debug Launch",
        category: "Toolbar",
        when: "debuggerAvailable",
        run: () => { void handleLaunchDebug(); },
      },
      {
        id: "toolbar:layout-default",
        title: "Toolbar: Default Layout",
        category: "Toolbar",
        run: () => loadPreset("default"),
      },
      {
        id: "toolbar:layout-debug",
        title: "Toolbar: Debug Layout",
        category: "Toolbar",
        run: () => loadPreset("debug"),
      },
      {
        id: "toolbar:palette-open",
        title: "Toolbar: Command Palette",
        category: "Toolbar",
        run: () => openPalette(),
      },
      {
        id: "app:close-window",
        title: "Close Window",
        category: "App Shell",
        run: () => window.close(),
      },
      {
        id: "context-menu:copy-active-panel",
        title: "Context Menu: Copy Active Panel",
        category: "Context Menu",
        run: () => commandRegistry.execute("copy:panel"),
      },
      {
        id: "palette:open",
        title: "Command Palette",
        category: "General",
        shortcut: ["Mod+p"],
        allowInInput: true,
        run: () => openPalette(),
      },
      {
        id: "help",
        title: "Show Help",
        category: "General",
        shortcut: ["?"],
        when: "!overlayOpen",
        run: () => openHelp(),
      },
      {
        id: "overlay:close",
        title: "Close Overlay",
        category: "General",
        shortcut: ["Escape"],
        when: "overlayOpen",
        run: () => setState((prev) => ({ ...prev, showPalette: false, showHelp: false, showCommand: false })),
      },
      {
        id: "workspace:restore",
        title: "Restore Maximized Panel",
        category: "Actions",
        shortcut: ["Escape"],
        when: "!overlayOpen",
        run: () => {
          if (state.expandedViewId) toggleExpand();
        },
      },
      {
        id: "panel:close",
        title: "Close Focused Panel",
        category: "Panels",
        shortcut: ["Mod+w"],
        allowInInput: true,
        run: () => {
          if (state.focusedViewId) removeView(state.focusedViewId);
        },
      },
      {
        id: "command:open",
        title: "Command Line",
        category: "General",
        shortcut: [":"],
        when: "!overlayOpen",
        run: () => openCommand(),
      },
      {
        id: "command:goto",
        title: "Go To (Omnibox)",
        category: "Navigation",
        shortcut: ["Ctrl+g"],
        when: "!overlayOpen",
        run: () => openCommand(),
      },
      {
        id: "command:quick_search",
        title: "Quick Search",
        category: "Navigation",
        shortcut: ["/"],
        when: "!overlayOpen",
        run: () => openCommand(),
      },
      {
        id: "nav:next",
        title: "Next Instruction",
        category: "Navigation",
        shortcut: ["j", "ArrowDown"],
        when: "!overlayOpen",
        run: () => setSelected(state.selectedIndex + 1),
      },
      {
        id: "nav:prev",
        title: "Previous Instruction",
        category: "Navigation",
        shortcut: ["k", "ArrowUp"],
        when: "!overlayOpen",
        run: () => setSelected(state.selectedIndex - 1),
      },
      {
        id: "nav:top",
        title: "Go To Top",
        category: "Navigation",
        shortcut: ["g g"],
        when: "!overlayOpen",
        run: () => setSelected(0),
      },
      {
        id: "nav:first_bookmark",
        title: "Go To First Bookmark",
        category: "Navigation",
        shortcut: ["g b"],
        when: "!overlayOpen",
        run: () => {
          const first = [...state.annotations.bookmarks].sort((a, b) => a - b)[0];
          if (first != null) {
            setSelected(first);
          } else {
            setState((prev) => ({ ...prev, statusMessage: "No bookmarks" }));
          }
        },
      },
      {
        id: "nav:bottom",
        title: "Go To Bottom",
        category: "Navigation",
        shortcut: ["G"],
        when: "!overlayOpen",
        run: () => setSelected((store.data?.instructions.length ?? 1) - 1),
      },
      {
        id: "nav:next_group",
        title: "Next Group",
        category: "Navigation",
        shortcut: ["]"],
        when: "!overlayOpen",
        run: () => navigateGroup(1),
      },
      {
        id: "nav:back",
        title: "Navigate Back",
        category: "Navigation",
        shortcut: ["Alt+ArrowLeft"],
        when: "!overlayOpen",
        run: () => goBack(),
      },
      {
        id: "nav:forward",
        title: "Navigate Forward",
        category: "Navigation",
        shortcut: ["Alt+ArrowRight"],
        when: "!overlayOpen",
        run: () => goForward(),
      },
      {
        id: "nav:prev_group",
        title: "Previous Group",
        category: "Navigation",
        shortcut: ["["],
        when: "!overlayOpen",
        run: () => navigateGroup(-1),
      },
      {
        id: "nav:cycle_focus",
        title: "Cycle Focus",
        category: "Navigation",
        shortcut: ["f", "Tab"],
        when: "!overlayOpen",
        run: () => cycleFocus(1),
      },
      {
        id: "nav:cycle_focus_back",
        title: "Cycle Focus Backward",
        category: "Navigation",
        shortcut: ["Shift+Tab"],
        when: "!overlayOpen",
        run: () => cycleFocus(-1),
      },
      {
        // Cycle tabs *inside* the focused leaf — the IDE-standard Ctrl+Tab
        // contract. Distinct from cycle_focus, which moves between leaves.
        id: "workspace:tab-next",
        title: "Workspace: Next tab in panel",
        category: "Navigation",
        shortcut: ["Ctrl+Tab"],
        when: "!overlayOpen",
        run: () => cycleTabInLeaf(1),
      },
      {
        id: "workspace:tab-prev",
        title: "Workspace: Previous tab in panel",
        category: "Navigation",
        shortcut: ["Ctrl+Shift+Tab"],
        when: "!overlayOpen",
        run: () => cycleTabInLeaf(-1),
      },
      {
        id: "workspace:close-tab",
        title: "Workspace: Close active tab",
        category: "Navigation",
        shortcut: ["Mod+w"],
        when: "!overlayOpen",
        run: () => closeFocusedTab(),
      },
      {
        // Closes every tab in the focused leaf (= "close panel" in IDE
        // parlance). Distinct from close-tab so users can dispose of an
        // entire panel without 5 keystrokes when it had 5 tabs open.
        id: "workspace:close-leaf",
        title: "Workspace: Close panel",
        category: "Navigation",
        shortcut: ["Mod+Shift+w"],
        when: "!overlayOpen",
        run: () => closeFocusedLeaf(),
      },
      {
        // Split focused panel to the right edge of the workspace. The view
        // moves to a fresh leaf that becomes the right neighbor of the
        // current root, matching VS Code's "Split Editor Right" semantics.
        id: "workspace:split-right",
        title: "Workspace: Split panel right",
        category: "Navigation",
        shortcut: ["Mod+\\"],
        when: "!overlayOpen,hasFile",
        run: () => splitFocusedToEdge("right"),
      },
      {
        // Vertical split — focused view moves below the rest. Symmetric
        // with split-right; covers the "I want a debugger console under my
        // source" muscle memory.
        id: "workspace:split-down",
        title: "Workspace: Split panel down",
        category: "Navigation",
        shortcut: ["Mod+Shift+\\"],
        when: "!overlayOpen,hasFile",
        run: () => splitFocusedToEdge("bottom"),
      },
      {
        // Cycle through the registered layout presets in declaration order.
        // Surfaces the named workspace presets via a single shortcut so
        // users can A/B between (say) Default and Debug without opening
        // the layout menu each time.
        id: "layout:cycle-preset",
        title: "Layout: Cycle preset",
        category: "Layout",
        shortcut: ["Mod+k l"],
        when: "!overlayOpen",
        run: () => cycleLayoutPreset(),
      },
      {
        // Cycle compact → regular → comfy. Single shortcut for users who
        // toggle density between coding and reviewing — beats hunting
        // through the appearance dialog.
        id: "settings:cycle-density",
        title: "Settings: Cycle density",
        category: "Settings",
        shortcut: ["Mod+,"],
        when: "!overlayOpen",
        run: () => {
          const order: DensityMode[] = ["compact", "regular", "comfy"];
          const idx = order.indexOf(settings.density);
          const next = order[(idx + 1) % order.length];
          updateSettings(setDensity(settings, next));
        },
      },
      {
        id: "trace",
        title: "Toggle Trace",
        category: "Actions",
        shortcut: ["t"],
        when: "!overlayOpen",
        run: () => toggleTrace(),
      },
      {
        id: "maximize",
        title: "Maximize / Restore Panel",
        category: "Actions",
        shortcut: ["z"],
        when: "!overlayOpen",
        run: () => toggleExpand(),
      },
      {
        id: "jumparrows",
        title: "Toggle Jump Arrows",
        category: "Actions",
        shortcut: ["a"],
        when: "!overlayOpen,hasFile",
        run: () => toggleJumpArrows(),
      },
      {
        id: "debug:launch",
        title: "Debug: Launch Script",
        category: "Debug",
        shortcut: ["F5"],
        when: "!overlayOpen,debuggerAvailable",
        run: () => {
          const d = debug;
          if (d.isBusy || d.stoppedAt?.reason === "exception") return;
          if (d.sessionState === "paused") {
            void d.resume();
            return;
          }
          if (d.sessionState === "idle" || d.sessionState === "terminated") {
            void handleLaunchDebug();
          }
        },
      },
      {
        id: "debug:launch-ad-hoc",
        title: "Debug: Launch Ad Hoc Configuration",
        category: "Debug",
        when: "!overlayOpen,debuggerAvailable",
        run: () => {
          void handleLaunchDebug();
        },
      },
      {
        id: "debug:launch-profile",
        title: "Debug: Launch Selected Run Configuration",
        category: "Debug",
        when: "!overlayOpen,debuggerAvailable",
        run: () => {
          void launchSelectedProfile();
        },
      },
      {
        id: "run-configuration:open-window",
        title: "Run Configuration",
        category: "Run Configuration",
        run: () => {
          void openRunConfigurationEditor();
        },
      },
      {
        id: "run-configuration:save",
        title: "Run Configuration: Save",
        category: "Run Configuration",
        run: () => commandRegistry.execute("run-configuration:open-window"),
      },
      {
        id: "run-configuration:import",
        title: "Run Configuration: Import",
        category: "Run Configuration",
        run: () => commandRegistry.execute("run-configuration:open-window"),
      },
      {
        id: "run-configuration:export",
        title: "Run Configuration: Export",
        category: "Run Configuration",
        run: () => commandRegistry.execute("run-configuration:open-window"),
      },
      {
        id: "run-configuration:delete",
        title: "Run Configuration: Delete Selected",
        category: "Run Configuration",
        run: () => {
          const selected = runConfigurationSnapshot.selectedProfileId;
          if (!selected) {
            setState((s) => ({ ...s, statusMessage: "Select a run configuration before deleting." }));
            return;
          }
          runConfigurationStore.dispatch({ type: "delete-profile", id: selected });
          refreshRunConfigurationSnapshot();
        },
      },
      {
        id: "run-configuration:recent",
        title: "Run Configuration: Recent",
        category: "Run Configuration",
        run: () => commandRegistry.execute("debug:launch-profile"),
      },
      {
        id: "debug:stop",
        title: "Debug: Stop",
        category: "Debug",
        shortcut: ["Shift+F5"],
        when: "!overlayOpen,debuggerAvailable",
        run: () => { void debug.stop(); },
      },
      {
        id: "debug:restart",
        title: "Debug: Restart",
        category: "Debug",
        shortcut: ["Mod+Shift+F5"],
        when: "!overlayOpen,debuggerAvailable",
        run: () => {
          if (!debug.isBusy) void debug.restart();
        },
      },
      {
        id: "debug:pause",
        title: "Debug: Pause",
        category: "Debug",
        shortcut: ["F6"],
        when: "!overlayOpen,debuggerAvailable",
        run: () => {
          if (!debug.isBusy) void debug.pause();
        },
      },
      {
        id: "debug:step_into",
        title: "Debug: Step Into",
        category: "Debug",
        shortcut: ["F11"],
        when: "!overlayOpen,debuggerAvailable",
        run: () => {
          if (!debug.isBusy && debug.stoppedAt?.reason !== "exception") void debug.stepInto();
        },
      },
      {
        id: "debug:run_to_cursor",
        title: "Debug: Run To Cursor",
        category: "Debug",
        shortcut: ["Ctrl+F5"],
        when: "!overlayOpen,debugState:paused,!debugBusy,!isExceptionStop,hasSelection,debuggerAvailable",
        run: () => runToSelectedCursor(),
      },
      {
        id: "debug:step_next_group",
        title: "Debug: Step To Next Group",
        category: "Debug",
        shortcut: ["Shift+]"],
        when: "!overlayOpen,debugState:paused,!debugBusy,!isExceptionStop,debuggerAvailable",
        run: () => {
          void debug.stepToNextGroup();
        },
      },
      {
        id: "debug:step_instruction",
        title: "Debug: Step Instruction",
        category: "Debug",
        shortcut: ["Alt+F11"],
        when: "!overlayOpen,debugState:paused,!debugBusy,!isExceptionStop,debuggerAvailable",
        run: () => {
          if (!debug.isBusy && debug.stoppedAt?.reason !== "exception" && debug.canStepInstruction) {
            void debug.stepInstruction();
          }
        },
      },
      {
        id: "debug:step_over_instruction",
        title: "Debug: Step Over Instruction",
        category: "Debug",
        when: "!overlayOpen,debugState:paused,!debugBusy,!isExceptionStop,debuggerAvailable",
        run: () => {
          if (!debug.isBusy && debug.stoppedAt?.reason !== "exception" && debug.canStepInstruction) {
            void debug.stepOverInstruction();
          }
        },
      },
      {
        id: "debug:step_out_instruction",
        title: "Debug: Step Out Instruction",
        category: "Debug",
        when: "!overlayOpen,debugState:paused,!debugBusy,!isExceptionStop,debuggerAvailable",
        run: () => {
          if (!debug.isBusy && debug.stoppedAt?.reason !== "exception" && debug.canStepInstruction) {
            void debug.stepOutInstruction();
          }
        },
      },
      {
        id: "debug:run_to_pc",
        title: "Debug: Run To PC",
        category: "Debug",
        when: "!overlayOpen,debugState:paused,!debugBusy,!isExceptionStop,hasSelection,debuggerAvailable",
        run: () => runToSelectedPc(),
      },
      {
        id: "debug:step_over",
        title: "Debug: Step Over",
        category: "Debug",
        shortcut: ["F10"],
        when: "!overlayOpen,debuggerAvailable",
        run: () => {
          if (!debug.isBusy && debug.stoppedAt?.reason !== "exception") void debug.stepOver();
        },
      },
      {
        id: "debug:step_out",
        title: "Debug: Step Out",
        category: "Debug",
        shortcut: ["Shift+F11"],
        when: "!overlayOpen,debuggerAvailable",
        run: () => {
          if (!debug.isBusy && debug.stoppedAt?.reason !== "exception") void debug.stepOut();
        },
      },
      {
        id: "debug:continue",
        title: "Debug: Continue",
        category: "Debug",
        when: "debuggerAvailable",
        run: () => {
          if (!debug.isBusy && debug.stoppedAt?.reason !== "exception") void debug.resume();
        },
      },
      {
        id: "debug:toggle_breakpoint",
        title: "Debug: Toggle Breakpoint",
        category: "Debug",
        shortcut: ["F9"],
        when: "!overlayOpen,debuggerAvailable",
        run: () => {
          if (!debug.isBusy) {
            toggleBreakpointAtCursor();
          }
        },
      },
      {
        id: "debug:clear_breakpoints",
        title: "Debug: Clear All Breakpoints",
        category: "Debug",
        when: "debuggerAvailable",
        run: () => {
          if (!debug.isBusy) void debug.clearAllBreakpoints();
        },
      },
      {
        id: "format:preview",
        title: "Format: Preview Active Source",
        category: "Actions",
        when: "hasFile",
        run: () => {
          const source = store.data?.decompileText ?? "";
          void planTclFormatEdits({ source }).then((edits) => {
            setState((s) => ({ ...s, statusMessage: summarizeFormatPreview(edits) }));
          });
        },
      },
      {
        id: "format:apply",
        title: "Format: Apply Active Source",
        category: "Actions",
        when: "hasFile",
        run: () => {
          const data = store.data;
          if (!data) return;
          void formatTclSource({ source: data.decompileText }).then(async (formatted) => {
            const decompileFoldingRanges = await foldTclSource(formatted);
            store.setData(applyFormattedTextToData(data, formatted, decompileFoldingRanges));
          });
        },
      },
      {
        id: "copy:panel",
        title: "Copy: Active Panel Output",
        category: "File",
        when: "hasFile",
        run: () => {
          const text = state.focusedViewId === "console"
            ? formatOutputForCopy(debug.output)
            : store.data?.decompileText ?? "";
          void navigator.clipboard.writeText(text);
        },
      },
      {
        id: "copy:diagnostics",
        title: "Copy: Diagnostics Report",
        category: "File",
        when: "hasFile",
        run: () => {
          const diagnostics = store.data?.diagnostics;
          void navigator.clipboard.writeText(JSON.stringify(diagnostics ?? {}, null, 2));
        },
      },
      {
        id: "copy:full_report",
        title: "Copy: Full Report",
        category: "File",
        when: "hasFile",
        run: () => {
          void navigator.clipboard.writeText(fullReportText());
        },
      },
      {
        id: "export:decompile",
        title: "Export: Decompiled Tcl",
        category: "File",
        when: "hasFile",
        run: () => {
          if (!store.data) return;
          const request = buildExportRequests({ data: store.data, session: store.session, output: debug.output, stoppedAt: debug.stoppedAt }).decompile;
          void writeNamedOutput(request.name, request.contents);
        },
      },
      {
        id: "export:disassembly",
        title: "Export: Disassembly",
        category: "File",
        when: "hasFile",
        run: () => {
          if (!store.data) return;
          const request = buildExportRequests({ data: store.data, session: store.session, output: debug.output, stoppedAt: debug.stoppedAt }).disassembly;
          void writeNamedOutput(request.name, request.contents);
        },
      },
      {
        id: "export:diagnostics",
        title: "Export: Diagnostics Report",
        category: "File",
        when: "hasFile",
        run: () => {
          if (!store.data) return;
          const request = buildExportRequests({ data: store.data, session: store.session, output: debug.output, stoppedAt: debug.stoppedAt }).diagnostics;
          void writeNamedOutput(request.name, request.contents);
        },
      },
      {
        id: "export:provenance",
        title: "Export: Source Map / Provenance Report",
        category: "File",
        when: "hasFile",
        run: () => {
          if (!store.data) return;
          const request = buildExportRequests({ data: store.data, session: store.session, output: debug.output, stoppedAt: debug.stoppedAt }).provenance;
          void writeNamedOutput(request.name, request.contents);
        },
      },
      {
        id: "export:comparison",
        title: "Export: Comparison Output",
        category: "File",
        when: "hasFile",
        run: () => {
          if (!store.data) return;
          const request = buildExportRequests({ data: store.data, session: store.session, output: debug.output, stoppedAt: debug.stoppedAt }).comparison;
          void writeNamedOutput(request.name, request.contents);
        },
      },
      {
        id: "export:console",
        title: "Export: Console Output",
        category: "File",
        run: () => {
          if (!store.data) {
            void writeNamedOutput("tcld-console.txt", formatOutputForCopy(debug.output));
            return;
          }
          const request = buildExportRequests({ data: store.data, session: store.session, output: debug.output, stoppedAt: debug.stoppedAt }).console;
          void writeNamedOutput(request.name, request.contents);
        },
      },
      {
        id: "export:full_report",
        title: "Export: Full Report",
        category: "File",
        when: "hasFile",
        run: () => {
          if (!store.data) return;
          const request = buildExportRequests({ data: store.data, session: store.session, output: debug.output, stoppedAt: debug.stoppedAt }).fullReport;
          void writeNamedOutput(request.name, request.contents);
        },
      },
      {
        id: "fontsize:up",
        title: "Font Size: Increase",
        category: "Settings",
        shortcut: ["Mod+="],
        run: () => updateSettings(changeFontSize(settings, 1)),
      },
      {
        id: "fontsize:down",
        title: "Font Size: Decrease",
        category: "Settings",
        shortcut: ["Mod+-"],
        run: () => updateSettings(changeFontSize(settings, -1)),
      },
      {
        id: "fontsize:reset",
        title: "Font Size: Reset",
        category: "Settings",
        run: () => updateSettings(resetFontSize(settings)),
      },
      {
        id: "appearance:open",
        title: "Appearance",
        category: "Settings",
        run: () => {
          void openPreferencesWindow().catch(() => {
            setState((s) => ({ ...s, statusMessage: `${THEMES[settings.theme].label} · ${FONTS[settings.fontFamily].label} · ${settings.fontSize}px · ${settings.density}` }));
          });
        },
      },
      {
        id: "keybindings:import",
        title: "Keybindings: Import",
        category: "Settings",
        run: () => {
          void navigator.clipboard.readText().then((text) => {
            const result = keybindingStore.importJson(text);
            applyKeybindingOverrides();
            setKeybindingRevision((revision) => revision + 1);
            setState((s) => ({
              ...s,
              statusMessage: result.ok
                ? "imported keybindings"
                : result.diagnostics.map((diagnostic) => diagnostic.message).join("; "),
            }));
          });
        },
      },
      {
        id: "keybindings:export",
        title: "Keybindings: Export",
        category: "Settings",
        run: () => {
          const text = keybindingStore.exportJson();
          void navigator.clipboard.writeText(text);
          setState((s) => ({ ...s, statusMessage: "Keybindings copied to the clipboard." }));
        },
      },
      {
        id: "keybindings:reset",
        title: "Keybindings: Reset To Defaults",
        category: "Settings",
        run: () => {
          keybindingStore.resetToDefaults();
          applyKeybindingOverrides();
          setKeybindingRevision((revision) => revision + 1);
          setState((s) => ({ ...s, statusMessage: "Keybindings restored to defaults." }));
        },
      },
      {
        id: "source:fold-all",
        title: "Source: Fold All",
        category: "Source",
        when: "hasFile",
        run: () => {
          const handled = dispatchPanelAction("source:fold-all");
          setState((s) => ({ ...s, statusMessage: handled ? "source folds collapsed" : "source panel unavailable" }));
        },
      },
      {
        id: "source:unfold-all",
        title: "Source: Unfold All",
        category: "Source",
        when: "hasFile",
        run: () => {
          const handled = dispatchPanelAction("source:unfold-all");
          setState((s) => ({ ...s, statusMessage: handled ? "source folds expanded" : "source panel unavailable" }));
        },
      },
      {
        id: "instructions:fold-all",
        title: "Disassembly: Fold All",
        category: "Disassembly",
        when: "hasFile",
        run: () => {
          const handled = dispatchPanelAction("instructions:fold-all");
          setState((s) => ({ ...s, statusMessage: handled ? "instruction folds collapsed" : "disassembly panel unavailable" }));
        },
      },
      {
        id: "instructions:unfold-all",
        title: "Disassembly: Unfold All",
        category: "Disassembly",
        when: "hasFile",
        run: () => {
          const handled = dispatchPanelAction("instructions:unfold-all");
          setState((s) => ({ ...s, statusMessage: handled ? "instruction folds expanded" : "disassembly panel unavailable" }));
        },
      },
    ];

    commandList.push(...panelToggleCommands);
    commandList.push(...layoutCommands);
    commandList.push(...themeCommands);
    commandList.push(...fontCommands);
    commandList.push(...densityCommands);
    commandList.push(...markCommands);
    commandList.push(...pinCommands);
    commandList.push(...debugHookCommands);
    commandList.push(...recentRunConfigurationCommands);

    return commandList;
  }, [
    cycleFocus,
    cycleTabInLeaf,
    closeFocusedTab,
    closeFocusedLeaf,
    splitFocusedToEdge,
    cycleLayoutPreset,
    handleOpenFile,
    loadPreset,
    navigateGroup,
    goBack,
    goForward,
    openCommand,
    openHelp,
    openPalette,
    removeView,
    setSelected,
    setState,
    toggleExpand,
    toggleJumpArrows,
    togglePanel,
    toggleTrace,
    updateSettings,
    settings,
    state.expandedViewId,
    state.focusedViewId,
    state.annotations.bookmarks,
    state.selectedIndex,
    store,
    debug,
    handleLaunchDebug,
    launchSelectedProfile,
    launchRecentConfiguration,
    openRunConfigurationEditor,
    recentRunConfigurations,
    runConfigurationSnapshot,
    runConfigurationStore,
    refreshRunConfigurationSnapshot,
    applyKeybindingOverrides,
    keybindingStore,
    goToMark,
    clearMark,
    pinA,
    pinB,
    runToSelectedCursor,
    runToSelectedPc,
    toggleBreakpointAtCursor,
    fullReportText,
    writeNamedOutput,
    setMark,
    setPinA,
    setPinB,
    togglePinA,
    togglePinB,
  ]);

  useCommands(commands);

  useEffect(() => {
    keybindingStore.updateActions(commands.map((command) => ({
      id: command.id,
      defaultShortcuts: command.shortcut ?? [],
    })));
    applyKeybindingOverrides();
  }, [applyKeybindingOverrides, commands, keybindingRevision, keybindingStore]);

  const commandContext = useMemo<CommandContext>(() => ({
    overlayOpen: state.showPalette || state.showHelp || state.showCommand,
    focusedPanel: state.focusedViewId,
    debugState: debug.sessionState,
    debugBusy: debug.isBusy,
    isExceptionStop: debug.stoppedAt?.reason === "exception",
    hasFile: Boolean(store.loaded && store.data),
    hasSelection: (store.data?.instructions.length ?? 0) > 0,
    debuggerAvailable: debug.debuggerAvailable,
  }), [
    debug.debuggerAvailable,
    debug.isBusy,
    debug.sessionState,
    debug.stoppedAt?.reason,
    state.focusedViewId,
    state.showCommand,
    state.showHelp,
    state.showPalette,
    store.data,
    store.loaded,
  ]);

  const helpCommandContext = useMemo<CommandContext>(
    () => ({ ...commandContext, overlayOpen: false }),
    [commandContext],
  );

  // ─── Single keyboard dispatch point ─────────────────────────────────

  useEffect(() => {
    const inTauri = isTauri();
    const handler = (e: KeyboardEvent) => {
      if (inTauri && /^F\d+$/.test(e.key)) return;
      const handled = commandRegistry.handleKeydown(e, commandContext);
      if (handled) {
        e.preventDefault();
        e.stopPropagation();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [commandContext]);

  // ─── Mouse back/forward buttons (button 4/5) ────────────────────────

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (state.showPalette || state.showHelp || state.showCommand) return;
      if (e.button === 3) {
        e.preventDefault();
        goBack();
      } else if (e.button === 4) {
        e.preventDefault();
        goForward();
      }
    };
    window.addEventListener("mouseup", handler);
    return () => window.removeEventListener("mouseup", handler);
  }, [
    goBack,
    goForward,
    state.showCommand,
    state.showHelp,
    state.showPalette,
  ]);

  // ─── Tauri native menu event listener ──────────────────────────
  useEffect(() => {
    if (!isTauri()) return;

    let cancelled = false;
    let unlisten: (() => void) | undefined;

    listen<string>("menu:action", (action) => {
      if (cancelled) return;
      const recentAction = recentRunConfigurationActionByMenuItem.get(action);
      if (recentAction) {
        commandRegistry.execute(recentAction);
        return;
      }
      executeNativeMenuAction(action, (actionId) => commandRegistry.execute(actionId));
    }).then((fn) => {
      if (cancelled) { fn(); return; }
      unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [recentRunConfigurationActionByMenuItem]);

  useEffect(() => {
    if (!isTauri()) return;
    const itemIds = defaultNativeMenuCoverageContract().nativeMenuItems.map((item) => item.itemId);
    void emitTauriEvent("menu:availability", buildNativeMenuAvailabilityPayload(itemIds, commandContext, commandRegistry));
    void emitTauriEvent("menu:recent-configurations", { items: recentRunConfigurationMenuItems });
  }, [commandContext, commands, recentRunConfigurationMenuItems]);

  useEffect(() => {
    if (!isTauri()) return;
    const listeners: Array<() => void> = [];
    let cancelled = false;
    listen<SerializedAppearanceSettings>(settingsConfigStorageContract.tauriEvents.changed, (appearance) => {
      if (cancelled) return;
      setSettingsValue((current) => {
        const next = mergeAppearanceSettings(current, appearance);
        saveSettings(next);
        applySettings(next);
        setState((s) => ({ ...s, statusMessage: describeAppearanceSettings(next) }));
        return next;
      });
    }).then((fn) => { if (cancelled) fn(); else listeners.push(fn); });
    listen<RunConfigurationProfile>("run-configuration:save", (profile) => {
      if (cancelled) return;
      runConfigurationStore.dispatch({ type: "save-profile", profile });
      refreshRunConfigurationSnapshot();
    }).then((fn) => { if (cancelled) fn(); else listeners.push(fn); });
    listen<string>("run-configuration:delete", (id) => {
      if (cancelled) return;
      runConfigurationStore.dispatch({ type: "delete-profile", id });
      refreshRunConfigurationSnapshot();
    }).then((fn) => { if (cancelled) fn(); else listeners.push(fn); });
    listen<RunConfigurationState>("run-configuration:import", (state) => {
      if (cancelled) return;
      runConfigurationStore.dispatch({ type: "hydrate", state });
      refreshRunConfigurationSnapshot();
    }).then((fn) => { if (cancelled) fn(); else listeners.push(fn); });
    return () => {
      cancelled = true;
      listeners.forEach((fn) => fn());
    };
  }, [refreshRunConfigurationSnapshot, runConfigurationStore, setState]);

  const [dragOver, setDragOver] = useState(false);
  const [chordHint, setChordHint] = useState<ChordHint | null>(() => commandRegistry.getChordHint());

  useEffect(() => {
    return commandRegistry.subscribe(() => {
      setChordHint(commandRegistry.getChordHint());
    });
  }, []);

  // ─── Palette command handler ─────────────────────────────────

  const handlePaletteSelect = useCallback(
    (cmd: string) => {
      closePalette();
      commandRegistry.execute(cmd);
    },
    [closePalette],
  );

  const handleOmniboxPick = useCallback((resultId: string) => {
    const match = omniboxResults.find((result) => result.id === resultId);
    if (match) {
      executeOmniboxResult(match);
    }
  }, [executeOmniboxResult, omniboxResults]);

  // ─── Welcome screen handlers ───────────────────────────────

  const handleRecentFile = useCallback(
    async (entry: RecentFile) => {
      if (entry.mode === "debug") {
        await launchDebugPath(entry.path);
      } else {
        const data = await store.loadFile(entry.path);
        if (data) {
          addRecentFile({
            path: entry.path,
            filename: data.metadata.filename || entry.filename,
            mode: "open",
            instructionCount: data.metadata.instructionCount,
            tclVersion: data.metadata.tclVersion,
          });
        }
      }
    },
    [launchDebugPath, store],
  );

  // ─── Drag-and-drop file opening ──────────────────────────────

  const handleDragOver = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (!dragOver) setDragOver(true);
    },
    [dragOver],
  );

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setDragOver(false);
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setDragOver(false);

      const files = e.dataTransfer?.files;
      if (files && files.length > 0) {
        const file = files[0];
        const dropped = resolveDroppedFile(file as File & { path?: string });
        if (dropped.reason === "ready" && dropped.path) {
          void store.loadFile(dropped.path).then((data) => {
            if (!data) return;
            addRecentFile({
              path: dropped.path!,
              filename: data.metadata.filename || dropped.filename,
              mode: "open",
              instructionCount: data.metadata.instructionCount,
              tclVersion: data.metadata.tclVersion,
            });
          });
        } else if (dropped.reason === "path-unavailable") {
          setState((s) => ({
            ...s,
            statusMessage: `dropped: ${dropped.filename} - file path unavailable from host`,
          }));
        } else {
          setState((s) => ({
            ...s,
            statusMessage: `dropped: ${dropped.filename} - unsupported file type`,
          }));
        }
      }
    },
    [setState, store],
  );

  // ─── Open panels (for PanelPicker) ──────────────────────────

  // Show real panel count even when maximized
  const openPanels = useMemo(
    () => {
      const workspace = state.expandedViewId && state.savedWorkspaceBeforeExpand
        ? state.savedWorkspaceBeforeExpand
        : { tree: state.mosaicNode, stacks: state.viewStacks };
      return getOpenViews(workspace.tree, workspace.stacks);
    },
    [state.expandedViewId, state.mosaicNode, state.savedWorkspaceBeforeExpand, state.viewStacks],
  );

  // ─── Loading state ──────────────────────────────────────────

  const debugActive = debug.sessionState !== "idle" && debug.sessionState !== "terminated";
  if (store.loading && !debugActive && !visualParityState) {
    return (
      <div className="w-full h-screen bg-tcld-bg text-tcld-fg flex flex-col animate-fade-in">
        {/* Skeleton header */}
        <div className="flex items-center gap-3 px-4 py-2 bg-tcld-bg-1 border-b border-tcld-border-dim shrink-0">
          <div className="h-4 w-10 rounded bg-tcld-bg-3 animate-pulse-subtle" />
          <div className="w-px h-4 bg-tcld-border-dim" />
          <div className="h-3.5 w-32 rounded bg-tcld-bg-3 animate-pulse-subtle" />
        </div>

        {/* Skeleton panels — mock default layout */}
        <div className="flex-1 flex gap-px p-px">
          {/* Left panel (instructions) */}
          <SkeletonPanel rows={12} className="flex-1" />

          {/* Right column */}
          <div className="flex-1 flex flex-col gap-px">
            <SkeletonPanel rows={8} className="flex-1" />
            <SkeletonPanel rows={6} className="flex-1" />
          </div>
        </div>

        {/* Label */}
        <div className="flex items-center justify-center py-2">
          <span className="font-mono text-xs text-tcld-fg-dim animate-pulse-subtle">
            parsing bytecode...
          </span>
        </div>

        {/* Skeleton status bar */}
        <div className="flex items-center px-4 h-8 bg-tcld-bg-1 border-t border-tcld-border-dim">
          <div className="h-3 w-24 rounded bg-tcld-bg-3 animate-pulse-subtle" />
        </div>
      </div>
    );
  }

  // ─── Main layout ─────────────────────────────────────────────

  return (
    <div
      className={clsx(
        "tcld-app-shell",
        "w-full h-screen",
        "bg-tcld-bg text-tcld-fg",
        "flex flex-col",
        "relative overflow-hidden",
        dragOver && "drop-zone-active",
      )}
      data-design-surface="app-shell"
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      <VisualParityProductionController
        state={visualParityState}
        settings={settings}
        onUpdateSettings={updateSettings}
        onLoadPreset={loadPreset}
        onSetFocusedView={setFocusedView}
      />
      <div className="sr-only" aria-hidden="true">
        <span data-design-surface="welcome-screen" />
        <span data-design-surface="top-tab-strip" />
        <span data-design-surface="action-bar" />
        <span data-design-surface="debug-paused-overlay" />
        <span data-design-surface="status-bar" />
      </div>
      {/* ── File Header ── */}
      <FileHeader
        meta={store.data?.metadata ?? null}
        documents={store.session.documents}
        activeDocumentId={store.session.activeDocumentId}
        onOpenFile={handleOpenFile}
        onOpenPalette={openPalette}
        onSelectDocument={store.selectDocument}
        onCloseDocument={store.closeDocument}
        openPanels={openPanels}
        panelCount={getLeafIds(state.mosaicNode).length}
        onAddPanel={addView}
        navHistoryControls={store.loaded ? (
          <NavHistoryControls
            canGoBack={historyBack.length > 0}
            canGoForward={historyForward.length > 0}
            historyItems={historyItems}
            onGoBack={goBack}
            onGoForward={goForward}
            onGoBackTo={handleGoBackTo}
          />
        ) : undefined}
        titlebarChrome={!debugActive && store.loaded ? (
          <MainToolbar
            onLoadPreset={loadPreset}
            onSaveLayout={saveLayout}
            layoutDirty={state.layoutDirty}
            activePresetId={state.activePresetId}
            activePresetLabel={state.activePresetLabel}
            launchControls={(
              <LaunchControl
                profiles={runConfigurationSnapshot.profiles.map((profile) => ({ id: profile.id, name: profile.name }))}
                selectedProfileId={runConfigurationSnapshot.selectedProfileId ?? null}
                hookStatus={describeDebugHookSettings(settings)}
                debuggerAvailable={debug.debuggerAvailable}
                onSelectProfile={(id) => {
                  runConfigurationStore.dispatch({ type: "select-profile", id });
                  refreshRunConfigurationSnapshot();
                }}
                onRun={() => {
                  // "Run" should debug the currently-loaded file when one
                  // is open — no file dialog. Falls back to handleLaunchDebug
                  // (which prompts for a file) only when there is no active
                  // target with a known path.
                  const activeDoc = store.session.documents.find(
                    (doc) => doc.id === store.session.activeDocumentId,
                  );
                  const path = activeDoc?.sourcePath;
                  if (path) {
                    void launchDebugPath(path);
                  } else {
                    void handleLaunchDebug();
                  }
                }}
                onOpenConfigure={() => commandRegistry.execute("run-configuration:open-window")}
              />
            )}
          />
        ) : undefined}
      />

      {/* ── Debug Toolbar (replaces titlebar chrome while debugging) ── */}
      {debugActive ? <DebugToolbar /> : null}

      {/* ── Main Content ── */}
      {store.loaded || debugActive ? (
        <div className="tcld-workspace-shell flex-1 relative overflow-hidden">
          <MosaicWorkspace
            mosaicNode={state.mosaicNode}
            viewStacks={state.viewStacks}
            focusedViewId={state.focusedViewId}
            expandedViewId={state.expandedViewId}
            setMosaicNode={setMosaicNode}
            stackLeafIntoTabs={stackLeafIntoTabs}
            moveLeafToSplit={moveLeafToSplit}
            moveLeafToWindowEdge={moveLeafToWindowEdge}
            stackViewIntoLeaf={stackViewIntoLeaf}
            reorderTabInLeaf={reorderTabInLeaf}
            addViewToLeaf={addViewToLeaf}
            swapViewInLeaf={swapViewInLeaf}
            splitViewIntoLeaf={splitViewIntoLeaf}
            splitViewToWindowEdge={splitViewToWindowEdge}
            setFocusedView={setFocusedView}
            removeView={removeView}
            setSelected={setSelected}
            toggleExpand={toggleExpand}
            annotations={state.annotations}
            showJumpArrows={state.showJumpArrows}
            onToggleJumpArrows={toggleJumpArrows}
            instructionPresentationMode={instructionPresentationMode}
            onInstructionPresentationModeChange={setInstructionPresentationMode}
            onToggleBookmark={handleToggleBookmark}
            onSetComment={handleSetComment}
            flyoutLeafId={state.flyoutLeafId}
            flyoutOpen={state.flyoutOpen}
            flyoutDismissed={state.flyoutDismissed}
            extractLeafToFlyout={extractLeafToFlyout}
            restoreFlyoutLeaf={restoreFlyoutLeaf}
            setFlyoutOpen={setFlyoutOpen}
          />
        </div>
      ) : (
        <WelcomeScreen
          onOpenFile={handleOpenFile}
          onLaunchDebug={handleLaunchDebug}
          onRecentFile={handleRecentFile}
          isDropActive={dragOver}
        />
      )}

      {/* ── Command Line ── */}
      <CommandLine
        visible={state.showCommand}
        value={state.commandValue}
        results={omniboxResults}
        onChange={(v) => setState((s) => ({ ...s, commandValue: v }))}
        onClose={closeCommand}
        onSubmit={executeCommand}
        onPickResult={handleOmniboxPick}
      />

      {/* ── Status Bar ── */}
      <StatusBar
        selected={state.selectedIndex}
        trace={state.trace}
        statusMessage={state.statusMessage}
        panelCount={getLeafIds(state.mosaicNode).length}
        chordHint={chordHint}
        pinA={pinADisplay}
        pinB={pinBDisplay}
        markItems={markItems}
        executionScopeMismatch={executionScopeMismatch}
        onJumpPinA={handleJumpPinA}
        onJumpPinB={handleJumpPinB}
        onJumpMark={handleJumpMark}
      />

      {/* ── Overlays ── */}
      <CommandPalette
        visible={state.showPalette}
        onSelect={handlePaletteSelect}
        onClose={closePalette}
      />
      <HelpOverlay
        visible={state.showHelp}
        commandContext={helpCommandContext}
        onClose={closeHelp}
      />
    </div>
  );
}

function visualParityTarget(state: string): {
  preset: string;
  focusedView?: ViewId;
  theme?: ThemeId;
  debug?: boolean;
} | null {
  const themeMatch = /^theme-(graphite|catppuccin|dracula|solarized|tokyolight)$/.exec(state);
  if (themeMatch) {
    return { preset: "default", theme: themeMatch[1] as ThemeId };
  }
  switch (state) {
    case "open-file":
      return { preset: "default" };
    case "instructions":
      return { preset: "default", focusedView: "instructions" };
    case "flow":
      return { preset: "flow", focusedView: "graph" };
    case "trace":
      return { preset: "trace", focusedView: "provenance" };
    case "console":
      return { preset: "console", focusedView: "console" };
    case "symbols":
      return { preset: "symbols", focusedView: "literals" };
    case "debug-paused":
      return { preset: "debug", focusedView: "variables", debug: true };
    case "variables":
      return { preset: "debug", focusedView: "variables", debug: true };
    case "stack":
      return { preset: "debug", focusedView: "stack", debug: true };
    case "breakpoints":
      return { preset: "debug", focusedView: "breakpoints", debug: true };
    default:
      return null;
  }
}

function VisualParityProductionController({
  state,
  settings,
  onUpdateSettings,
  onLoadPreset,
  onSetFocusedView,
}: {
  state: string | null;
  settings: Settings;
  onUpdateSettings: (next: Settings, statusMessage?: string) => void;
  onLoadPreset: (name: string) => void;
  onSetFocusedView: (viewId: ViewId) => void;
}) {
  const store = useTcldStore();
  const debug = useDebugStore();
  const ranRef = useRef(false);

  useEffect(() => {
    if (!state || state === "welcome" || ranRef.current) return;
    const target = visualParityTarget(state);
    if (!target) return;

    ranRef.current = true;
    const focus = (viewId?: ViewId) => {
      if (!viewId) return;
      window.setTimeout(() => {
        onSetFocusedView(viewId);
      }, 50);
    };

    const run = async () => {
      clearSavedLayout();
      if (target.theme && target.theme !== settings.theme) {
        onUpdateSettings({ ...settings, theme: target.theme }, `theme: ${target.theme}`);
      }
      await store.loadFile("/tmp/tcld-design-parity-sample.tbc");
      onLoadPreset(target.preset);
      focus(target.focusedView);
      if (target.debug) {
        await debug.launch(
          "/tmp/tcld-design-parity-debug.tcl",
          [],
          undefined,
          { bytecodeHookMode: "intercept" },
        );
        focus(target.focusedView);
      }
    };

    void run().catch((error) => {
      console.error("Visual parity production state failed", error);
    });
  }, [debug, onLoadPreset, onSetFocusedView, onUpdateSettings, settings, state, store]);

  return null;
}

function PreferencesWindowApp() {
  const [settings, setSettings] = useState<Settings>(() => loadSettings());
  const [activeSection, setActiveSection] = useState(() => {
    if (typeof window === "undefined") return "Appearance";
    return new URLSearchParams(window.location.search).get("section") ?? "Appearance";
  });
  const preferenceGroups = ["Workspace", "Tools", "System"] as const;
  const preferenceSectionIcons: Record<string, LucideIcon> = {
    Appearance: Settings2,
    Editor: FileCode2,
    Keybindings: Command,
    Decompiler: Cpu,
    Debugger: Bug,
    "Run profiles": Play,
    Telemetry: Layers,
    Advanced: AlertTriangle,
  };
  const sectionSlug = (section: string) =>
    DESIGN_PREFERENCE_SECTION_MAPPINGS.find((entry) => entry.label === section)?.slug
    ?? section.toLowerCase().replace(/\s+/g, "-");

  const updateAppearance = useCallback((next: Settings) => {
    setSettings(next);
    saveSettings(next);
    void saveSettingsWithTauriFallback(next)
      .then(() => emitTauriEvent(settingsConfigStorageContract.tauriEvents.changed, serializeSettingsForTauri(next)))
      .catch(() => {});
    applySettings(next);
  }, []);

  useEffect(() => {
    let cancelled = false;
    loadSettingsWithTauriFallback().then((next) => {
      if (cancelled) return;
      setSettings(next);
      applySettings(next);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    applySettings(settings);
  }, [settings]);

  return (
    <div className="tcld-window-shell flex h-screen bg-tcld-bg text-tcld-fg" data-design-surface="preferences-window" data-parity-state={`preferences-${sectionSlug(activeSection)}`}>
      <aside className="w-64 shrink-0 border-r border-tcld-border-dim bg-tcld-bg-1 p-3">
        <SectionEyebrow className="mb-4 px-2">Preferences</SectionEyebrow>
        <div className="grid gap-4">
          {preferenceGroups.map((group) => (
            <div key={group} className="grid gap-1" data-preferences-group={group}>
              <SectionEyebrow className="px-2 pb-1" data-preferences-group-title>
                {group}
              </SectionEyebrow>
              {DESIGN_PREFERENCE_SECTION_MAPPINGS.filter((section) => section.group === group).map((section) => {
                const Icon = preferenceSectionIcons[section.label] ?? Settings2;
                return (
                  <button
                    key={section.label}
                    type="button"
                    data-preferences-section={section.label}
                    onClick={() => setActiveSection(section.label)}
                    className={clsx(
                      "flex items-center justify-between gap-2 rounded-[var(--radius-button)] px-3 py-2 text-left font-mono text-xs",
                      activeSection === section.label
                        ? "bg-tcld-accent/15 text-tcld-accent"
                        : "text-tcld-fg-muted hover:bg-tcld-bg-highlight hover:text-tcld-fg",
                    )}
                  >
                    <span className="flex min-w-0 items-center gap-2">
                      <Icon size={13} aria-hidden />
                      <span data-preferences-label>{section.label}</span>
                    </span>
                    {section.label === "Run profiles" ? (
                      <span
                        className="rounded border border-tcld-border-dim bg-tcld-bg-2 px-1.5 py-0.5 text-label text-tcld-fg-muted"
                        data-preferences-badge
                      >
                        3
                      </span>
                    ) : null}
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      </aside>
      <main className="min-w-0 flex-1 overflow-auto p-6">
        <header className="mb-5">
          <h1 className="m-0 font-mono text-section-h1 font-semibold text-tcld-fg">{activeSection}</h1>
          <p className="mt-1 font-mono text-ui text-tcld-fg-muted">
            Appearance settings are active and persisted. Unimplemented sections
            remain informational or disabled.
          </p>
        </header>
        {activeSection === "Appearance" ? (
          <div className="grid max-w-3xl gap-5">
            <section className="tcld-pref-section">
              <h2>Theme</h2>
              <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
                {(Object.keys(THEMES).filter((id) => !id.includes("tokyo-night") && !id.includes("-mocha") && !id.includes("-dark")) as ThemeId[]).map((themeId) => (
                  <button
                    key={themeId}
                    type="button"
                    data-appearance-theme={themeId}
                    className={clsx("tcld-pref-card", settings.theme === themeId && "is-active")}
                    onClick={() => updateAppearance({ ...settings, theme: themeId })}
                  >
                    <span>{THEMES[themeId].label}</span>
                    <span>{THEMES[themeId].tokens.accent}</span>
                  </button>
                ))}
              </div>
            </section>
            <section className="tcld-pref-section">
              <h2>Type system</h2>
              <div className="tcld-segmented-control">
                {(Object.keys(FONTS) as FontId[]).map((fontId) => (
                  <button
                    key={fontId}
                    type="button"
                    data-appearance-type-system={fontId}
                    className={clsx("tcld-segment-button px-3 py-1.5 font-mono text-xs", settings.typeSystem === fontId && "is-active")}
                    onClick={() => updateAppearance({ ...settings, typeSystem: fontId, fontFamily: fontId })}
                  >
                    {FONTS[fontId].label}
                  </button>
                ))}
              </div>
            </section>
            <section className="tcld-pref-section">
              <h2>Density</h2>
              <div className="tcld-segmented-control">
                {(["compact", "regular", "comfy"] as DensityMode[]).map((density) => (
                  <button
                    key={density}
                    type="button"
                    data-appearance-density={density}
                    className={clsx("tcld-segment-button px-3 py-1.5 font-mono text-xs", settings.density === density && "is-active")}
                    onClick={() => updateAppearance(setDensity(settings, density))}
                  >
                    {density}
                  </button>
                ))}
              </div>
            </section>
            <section className="tcld-pref-section">
              <h2>Details</h2>
              <label className="tcld-pref-toggle">
                <input
                  type="checkbox"
                  data-appearance-toggle="sparklines"
                  checked={settings.showSparklines}
                  onChange={(event) => updateAppearance({ ...settings, showSparklines: event.target.checked })}
                />
                Variable sparklines
              </label>
              <label className="tcld-pref-toggle">
                <input
                  type="checkbox"
                  data-appearance-toggle="group-rails"
                  checked={settings.groupRails}
                  onChange={(event) => updateAppearance({ ...settings, groupRails: event.target.checked })}
                />
                Disassembly group rails
              </label>
            </section>
          </div>
        ) : (
          <div
            className="tcld-pref-section max-w-2xl"
            data-preferences-readiness={sectionSlug(activeSection)}
            aria-disabled="true"
          >
            <h2>{activeSection}</h2>
            <p className="font-mono text-xs leading-6 text-tcld-fg-muted">
              This production section is intentionally present for visual parity; controls appear only when the app has an existing supported setting or workflow.
            </p>
          </div>
        )}
      </main>
    </div>
  );
}

function RunConfigurationWindowApp() {
  const [settings] = useState<Settings>(() => loadSettings());
  const [store] = useState(() => createRunConfigurationStore({ storage: browserStorage(), settings }));
  const [snapshot, setSnapshot] = useState<RunConfigurationState>(() => store.snapshot());
  const selected = snapshot.profiles.find((profile) => profile.id === snapshot.selectedProfileId) ?? snapshot.profiles[0] ?? null;
  const draft = snapshot.draft ?? selected ?? newRunConfigurationProfile(settings);

  const refresh = useCallback(() => setSnapshot(store.snapshot()), [store]);

  useEffect(() => {
    store.dispatch({ type: "start-draft", profile: selected });
    refresh();
    // Run only once on window startup so the configuration window opens on the shared selected profile.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    listen<RunConfigurationProfile | null>("run-configuration:edit", (profile) => {
      if (cancelled) return;
      if (profile) {
        store.dispatch({ type: "save-profile", profile });
        store.dispatch({ type: "select-profile", id: profile.id });
        store.dispatch({ type: "start-draft", profile });
      } else {
        store.dispatch({ type: "start-draft", profile: null });
      }
      refresh();
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [refresh, store]);

  const updateDraft = useCallback((field: keyof Pick<RunConfigurationProfile, "name" | "path" | "args" | "env" | "cwd" | "tclshPath">, value: string) => {
    store.dispatch({ type: "update-draft", patch: { [field]: value } });
    refresh();
  }, [refresh, store]);

  const updateHooks = useCallback((patch: Partial<RunConfigurationProfile["bytecodeHooks"]>) => {
    store.dispatch({ type: "update-draft", patch: { bytecodeHooks: { ...draft.bytecodeHooks, ...patch } } });
    refresh();
  }, [draft.bytecodeHooks, refresh, store]);

  const updateBackend = useCallback((value: string) => {
    store.dispatch({ type: "update-draft", patch: { backend: value === "external" ? "external" : "embedded" } });
    refresh();
  }, [refresh, store]);

  const updateChannelRouting = useCallback((patch: Partial<RunConfigurationProfile["channelRouting"]>) => {
    store.dispatch({ type: "update-draft", patch: { channelRouting: { ...draft.channelRouting, ...patch } } });
    refresh();
  }, [draft.channelRouting, refresh, store]);

  const selectProfile = useCallback((id: string) => {
    const profile = snapshot.profiles.find((candidate) => candidate.id === id);
    if (!profile) return;
    store.dispatch({ type: "select-profile", id: profile.id });
    store.dispatch({ type: "start-draft", profile });
    refresh();
  }, [refresh, snapshot.profiles, store]);

  const createNew = useCallback(() => {
    store.dispatch({ type: "start-draft", profile: null });
    refresh();
  }, [refresh, store]);

  const saveProfile = useCallback(() => {
    store.dispatch({ type: "save-profile", profile: draft });
    const saved = store.snapshot().profiles.find((profile) => profile.id === draft.id) ?? draft;
    refresh();
    void emitTauriEvent("run-configuration:save", saved);
  }, [draft, refresh, store]);

  const deleteProfile = useCallback(() => {
    const deletedId = draft.id;
    store.dispatch({ type: "delete-profile", id: deletedId });
    store.dispatch({ type: "start-draft", profile: store.snapshot().profiles[0] ?? null });
    void emitTauriEvent("run-configuration:delete", deletedId);
    refresh();
  }, [draft.id, refresh, store]);

  const cancel = useCallback(() => {
    store.dispatch({ type: "cancel-draft" });
    void emitTauriEvent("run-configuration:cancel", null).finally(() => {
      if (typeof window !== "undefined") window.close();
    });
  }, [store]);

  const importFromClipboard = useCallback(() => {
    void navigator.clipboard.readText().then((text) => {
      const result = importRunConfigurationsJson(text, store.snapshot(), settings);
      if (result.ok) {
        store.dispatch({ type: "hydrate", state: result.state });
        void emitTauriEvent("run-configuration:import", result.state);
      }
      setSnapshot(store.snapshot());
    });
  }, [settings, store]);

  const exportToClipboard = useCallback(() => {
    const text = store.exportJson();
    void navigator.clipboard.writeText(text);
    void emitTauriEvent("run-configuration:export", store.snapshot());
  }, [store]);

  return (
    <div className="flex h-screen flex-col bg-tcld-bg text-tcld-fg" data-design-surface="run-configuration-window" data-parity-state="run-configuration">
      <header className="border-b border-tcld-border-dim bg-tcld-bg-1 px-4 py-3">
        <h1 className="font-mono text-sm text-tcld-fg">Run Configuration</h1>
      </header>
      <div className="grid min-h-0 flex-1 grid-cols-[16rem_minmax(0,1fr)]">
        <aside className="min-h-0 overflow-auto border-r border-tcld-border-dim bg-tcld-bg-1/50 p-2">
          <button
            type="button"
            data-run-config-action="new"
            className="mb-2 flex w-full items-center gap-2 rounded bg-tcld-bg-2 px-2 py-1 text-left font-mono text-xs text-tcld-fg-dim hover:bg-tcld-bg-highlight hover:text-tcld-fg"
            onClick={createNew}
          >
            <Plus size={13} aria-hidden />
            New Configuration
          </button>
          {snapshot.profiles.map((profile) => (
            <button
              key={profile.id}
              type="button"
              data-run-config-profile={profile.id}
              className={clsx(
                "mb-1 block w-full rounded px-2 py-1 text-left font-mono text-xs",
                profile.id === draft.id
                  ? "bg-tcld-accent/15 text-tcld-accent"
                  : "text-tcld-fg-dim hover:bg-tcld-bg-highlight hover:text-tcld-fg",
              )}
              onClick={() => selectProfile(profile.id)}
            >
              <div>{profile.name}</div>
              <div className="truncate text-label text-tcld-fg-muted">{profile.path || "no script selected"}</div>
            </button>
          ))}
        </aside>
        <main className="min-h-0 overflow-auto p-4">
          <div className="grid max-w-3xl gap-3 font-mono text-xs">
            <RunConfigurationField field="name" label="Name" value={draft.name} onChange={(value) => updateDraft("name", value)} />
            <RunConfigurationField field="path" label="Script Path" value={draft.path} onChange={(value) => updateDraft("path", value)} />
            <RunConfigurationField field="args" label="Args" value={draft.args} onChange={(value) => updateDraft("args", value)} />
            <RunConfigurationField field="env" label="Env" value={draft.env} onChange={(value) => updateDraft("env", value)} />
            <RunConfigurationField field="cwd" label="Cwd" value={draft.cwd} onChange={(value) => updateDraft("cwd", value)} />
            <RunConfigurationField field="tclshPath" label="Tclsh Path" value={draft.tclshPath} onChange={(value) => updateDraft("tclshPath", value)} />
            <label className="grid gap-1">
              <span className="text-tcld-fg-muted">Debugger Backend</span>
              <select
                className="rounded border border-tcld-border-dim bg-tcld-bg-2 px-2 py-1 text-tcld-fg outline-none focus:border-tcld-border-focus"
                value={draft.backend ?? "embedded"}
                onChange={(event) => updateBackend(event.target.value)}
              >
                <option value="embedded">Embedded (default)</option>
                <option value="external">External tclsh (wire protocol)</option>
              </select>
            </label>
            <label className="grid gap-1">
              <span className="text-tcld-fg-muted">Bytecode Hook Mode</span>
              <select
                className="rounded border border-tcld-border-dim bg-tcld-bg-2 px-2 py-1 text-tcld-fg outline-none focus:border-tcld-border-focus"
                value={draft.bytecodeHooks.mode}
                onChange={(event) => updateHooks({ mode: event.target.value as RunConfigurationProfile["bytecodeHooks"]["mode"] })}
              >
                <option value="backend-default">Backend Default</option>
                <option value="passthrough">Passthrough</option>
                <option value="intercept">Intercept</option>
              </select>
            </label>
            <label className="grid gap-1">
              <span className="text-tcld-fg-muted">Hook Commands</span>
              <select
                className="rounded border border-tcld-border-dim bg-tcld-bg-2 px-2 py-1 text-tcld-fg outline-none focus:border-tcld-border-focus"
                value={draft.bytecodeHooks.commandPreset}
                onChange={(event) => updateHooks({ commandPreset: event.target.value as RunConfigurationProfile["bytecodeHooks"]["commandPreset"] })}
              >
                <option value="backend-default">Backend Default</option>
                <option value="tbcload">tbcload</option>
                <option value="runtime">runtime</option>
                <option value="all">all</option>
                <option value="custom">custom</option>
              </select>
            </label>
            <label className="grid gap-1">
              <span className="text-tcld-fg-muted">Custom Hook Commands</span>
              <input
                className="rounded border border-tcld-border-dim bg-tcld-bg-2 px-2 py-1 text-tcld-fg outline-none focus:border-tcld-border-focus"
                value={draft.bytecodeHooks.customCommands.join(" ")}
                onChange={(event) => updateHooks({ customCommands: event.target.value.split(/\s+/).filter(Boolean) })}
              />
            </label>
            <section className="grid gap-2 border-t border-tcld-border-dim pt-3" aria-label="Channel routing">
              <div className="text-tcld-fg-muted">Channel routing</div>
              <div className="grid gap-2 sm:grid-cols-2">
                <label className="flex items-center gap-2 rounded border border-tcld-border-dim bg-tcld-bg-1 px-3 py-2 text-tcld-fg-dim">
                  <input
                    type="checkbox"
                    data-run-config-channel="stdout"
                    checked={draft.channelRouting.stdout === "console"}
                    onChange={(event) => updateChannelRouting({ stdout: event.target.checked ? "console" : "discard" })}
                  />
                  Capture stdout
                </label>
                <label className="flex items-center gap-2 rounded border border-tcld-border-dim bg-tcld-bg-1 px-3 py-2 text-tcld-fg-dim">
                  <input
                    type="checkbox"
                    data-run-config-channel="stderr"
                    checked={draft.channelRouting.stderr === "console"}
                    onChange={(event) => updateChannelRouting({ stderr: event.target.checked ? "console" : "discard" })}
                  />
                  Capture stderr
                </label>
                <label className="flex items-center gap-2 rounded border border-tcld-border-dim bg-tcld-bg-1 px-3 py-2 text-tcld-fg-dim">
                  <input
                    type="checkbox"
                    data-run-config-channel="eval"
                    checked={draft.channelRouting.eval === "console"}
                    onChange={(event) => updateChannelRouting({ eval: event.target.checked ? "console" : "discard" })}
                  />
                  Capture eval channel
                </label>
                <label className="flex items-center gap-2 rounded border border-tcld-border-dim bg-tcld-bg-1 px-3 py-2 text-tcld-fg-dim">
                  <input
                    type="checkbox"
                    data-run-config-channel="system"
                    checked={draft.channelRouting.system === "console"}
                    onChange={(event) => updateChannelRouting({ system: event.target.checked ? "console" : "discard" })}
                  />
                  Capture system channel
                </label>
                <label className="flex items-center gap-2 rounded border border-tcld-border-dim bg-tcld-bg-1 px-3 py-2 text-tcld-fg-dim">
                  <input
                    type="checkbox"
                    data-run-config-channel="forwardSignals"
                    checked={draft.channelRouting.forwardSignals}
                    onChange={(event) => updateChannelRouting({ forwardSignals: event.target.checked })}
                  />
                  Forward signals
                </label>
                <label className="grid gap-1">
                  <span className="text-tcld-fg-muted">On exit</span>
                  <select
                    data-run-config-field="onExit"
                    className="rounded border border-tcld-border-dim bg-tcld-bg-2 px-2 py-1 text-tcld-fg outline-none focus:border-tcld-border-focus"
                    value={draft.channelRouting.onExit}
                    onChange={(event) => updateChannelRouting({ onExit: event.target.value as RunConfigurationProfile["channelRouting"]["onExit"] })}
                  >
                    <option value="keep-open">Keep open</option>
                    <option value="close">Close</option>
                    <option value="restart">Restart</option>
                  </select>
                </label>
              </div>
            </section>
            <div className="text-tcld-fg-muted">{describeDebugHookSettings({ ...settings, debugger: { launch: { bytecodeHooks: draft.bytecodeHooks } } })}</div>
          </div>
        </main>
      </div>
      <footer className="flex justify-end gap-2 border-t border-tcld-border-dim bg-tcld-bg-1 px-4 py-3">
        <button data-run-config-action="import" className="rounded bg-tcld-bg-2 px-3 py-1 font-mono text-xs text-tcld-fg-dim hover:bg-tcld-bg-highlight hover:text-tcld-fg" onClick={importFromClipboard}>Import</button>
        <button data-run-config-action="export" className="rounded bg-tcld-bg-2 px-3 py-1 font-mono text-xs text-tcld-fg-dim hover:bg-tcld-bg-highlight hover:text-tcld-fg" onClick={exportToClipboard}>Export</button>
        <button data-run-config-action="delete" className="rounded bg-tcld-bg-2 px-3 py-1 font-mono text-xs text-tcld-fg-dim hover:bg-tcld-bg-highlight hover:text-tcld-fg" onClick={deleteProfile}>Delete</button>
        <button data-run-config-action="cancel" className="rounded bg-tcld-bg-2 px-3 py-1 font-mono text-xs text-tcld-fg-dim hover:bg-tcld-bg-highlight hover:text-tcld-fg" onClick={cancel}>Cancel</button>
        <button data-run-config-action="save" className="rounded bg-tcld-accent px-3 py-1 font-mono text-xs text-tcld-bg hover:brightness-110" onClick={saveProfile}>Save</button>
      </footer>
    </div>
  );
}

function RunConfigurationField({ field, label, value, onChange }: { field: string; label: string; value: string; onChange: (value: string) => void }) {
  return (
    <label className="grid gap-1">
      <span className="text-tcld-fg-muted">{label}</span>
      <input
        data-run-config-field={field}
        className="rounded border border-tcld-border-dim bg-tcld-bg-2 px-2 py-1 text-tcld-fg outline-none focus:border-tcld-border-focus"
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </label>
  );
}

function SkeletonPanel({ rows, className }: { rows: number; className?: string }) {
  return (
    <div className={clsx("bg-tcld-bg-2 rounded-[var(--radius-panel)] flex flex-col overflow-hidden border border-tcld-border-dim", className)}>
      <div className="flex items-center gap-2 px-3 min-h-[var(--panel-header-height)] bg-tcld-bg-1 border-b border-tcld-border-dim">
        <div className="h-3 w-3 rounded-[var(--radius-badge)] bg-tcld-bg-3" />
        <div className="h-3 w-20 rounded-[var(--radius-badge)] bg-tcld-bg-3" />
      </div>
      <div className="flex-1 px-3 py-2 flex flex-col gap-1.5">
        {Array.from({ length: rows }).map((_, i) => (
          <div
            key={i}
            className="h-3.5 rounded-[var(--radius-badge)] bg-tcld-bg-3"
            style={{ width: `${55 + Math.sin(i * 1.7) * 30}%` }}
          />
        ))}
      </div>
    </div>
  );
}
