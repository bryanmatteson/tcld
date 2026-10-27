// ─── Tauri IPC API Layer ─────────────────────────────────────────
// Bridges the Next.js frontend to the Rust backend via Tauri commands.

import type {
  FileMetadata,
  Instruction,
  LiteralEntry,
  ExceptionEntry,
  BasicBlock,
  TraceFrame,
  InstructionDetail,
  DecompileLine,
  SyntaxToken,
  FoldingRange,
  FormatRequest,
  FormatEdit,
  DebugBreakpoint,
  CommandResolution,
  WatchpointInfo,
  SourceMapEntry,
  SourceCommandAnchor,
  DiagnosticsReport,
  LoadProvenance,
  ReconstructionStatus,
} from "@/types";

// ─── Tauri invoke wrapper ────────────────────────────────────────

type InvokeArgs = Record<string, unknown>;
type InvokeOverride = <T>(cmd: string, args?: InvokeArgs) => Promise<T>;
type EventUnlisten = () => void;

interface BrowserTauriTestHarness {
  invoke?: InvokeOverride;
  openFileDialog?: () => Promise<string | null>;
  openScriptDialog?: () => Promise<string | null>;
  saveFileDialog?: (defaultPath?: string) => Promise<string | null>;
  emit?: <T>(event: string, payload: T) => Promise<void>;
  listen?: <T>(event: string, handler: (payload: T) => void) => Promise<EventUnlisten>;
}

declare global {
  interface Window {
    __TCLD_TEST_TAURI__?: BrowserTauriTestHarness;
  }
}

let invokeOverride: InvokeOverride | null = null;

export function __setInvokeForTests(override: InvokeOverride | null): void {
  invokeOverride = override;
}

async function invoke<T>(cmd: string, args?: InvokeArgs): Promise<T> {
  if (invokeOverride) {
    return invokeOverride<T>(cmd, args);
  }
  if (typeof window !== "undefined" && window.__TCLD_TEST_TAURI__?.invoke) {
    return window.__TCLD_TEST_TAURI__.invoke<T>(cmd, args);
  }
  if (typeof window !== "undefined" && window.__TAURI_INTERNALS__) {
    const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
    return tauriInvoke<T>(cmd, args);
  }
  throw new Error("This action requires the desktop application.");
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && (!!window.__TAURI_INTERNALS__ || !!window.__TCLD_TEST_TAURI__);
}

/**
 * Best-effort OS detection — used to gate macOS-only chrome (overlay
 * title-bar traffic-light gutter, etc.). Falls back to navigator.platform /
 * userAgentData on the web preview, and to `"macos"` only when we can be
 * confident; otherwise returns `"other"` so consumers don't accidentally
 * reserve mac-only space on Linux/Windows.
 */
export type Platform = "macos" | "windows" | "linux" | "other";

export function detectPlatform(): Platform {
  if (typeof navigator === "undefined") return "other";
  // Modern API (Chromium-based). Tauri webviews surface this for macOS as "macOS".
  const uaData = (navigator as unknown as {
    userAgentData?: { platform?: string };
  }).userAgentData;
  const platformHint = (uaData?.platform ?? navigator.platform ?? "").toLowerCase();
  if (platformHint.includes("mac")) return "macos";
  if (platformHint.includes("win")) return "windows";
  if (platformHint.includes("linux") || platformHint.includes("x11")) return "linux";
  return "other";
}

export function isMacOS(): boolean {
  return detectPlatform() === "macos";
}

// ─── File dialog ─────────────────────────────────────────────────

export async function openFileDialog(): Promise<string | null> {
  if (typeof window !== "undefined" && window.__TCLD_TEST_TAURI__?.openFileDialog) {
    return window.__TCLD_TEST_TAURI__.openFileDialog();
  }
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const result = await open({
      multiple: false,
      filters: [
        { name: "Tcl Bytecode and Source", extensions: ["tbc", "tcl", "tk", "itcl", "tm"] },
        { name: "All Files", extensions: ["*"] },
      ],
    });
    if (typeof result === "string") return result;
    if (result && typeof result === "object" && "path" in result) return (result as { path: string }).path;
    return null;
  } catch {
    return null;
  }
}

export async function openScriptDialog(): Promise<string | null> {
  if (typeof window !== "undefined" && window.__TCLD_TEST_TAURI__?.openScriptDialog) {
    return window.__TCLD_TEST_TAURI__.openScriptDialog();
  }
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const result = await open({
      multiple: false,
      filters: [
        { name: "Debug Targets", extensions: ["tbc", "tcl", "tk", "itcl", "tm"] },
        { name: "All Files", extensions: ["*"] },
      ],
    });
    if (typeof result === "string") return result;
    if (result && typeof result === "object" && "path" in result) {
      return (result as { path: string }).path;
    }
    return null;
  } catch {
    return null;
  }
}

export async function saveFileDialog(defaultPath?: string): Promise<string | null> {
  if (typeof window !== "undefined" && window.__TCLD_TEST_TAURI__?.saveFileDialog) {
    return window.__TCLD_TEST_TAURI__.saveFileDialog(defaultPath);
  }
  try {
    const { save } = await import("@tauri-apps/plugin-dialog");
    const result = await save({ defaultPath });
    if (typeof result === "string") return result;
    if (result && typeof result === "object" && "path" in result) {
      return (result as { path: string }).path;
    }
    return null;
  } catch {
    return null;
  }
}

// ─── Raw backend result shape ────────────────────────────────────

interface LoadResultRaw {
  metadata: FileMetadata;
  instructions: Instruction[];
  literals: LiteralEntry[];
  exceptions: ExceptionEntry[];
  blocks: BasicBlock[];
  traces: TraceFrame[];
  decompileLines: DecompileLine[];
  decompileTokens: SyntaxToken[];
  decompileFoldingRanges?: FoldingRange[];
  decompileText: string;
  dumpText: string;
  summaryText: string;
  sourceMap: SourceMapEntry[];
  sourceCommandAnchors?: SourceCommandAnchor[];
  diagnostics: DiagnosticsReport;
  reconstruction: ReconstructionStatus;
  provenance: LoadProvenance;
  originalSourceText?: string | null;
}

export type LoadResult = LoadResultRaw;

export function consumeLoadResult(raw: LoadResultRaw): LoadResult {
  const sourceCommandAnchors = raw.sourceCommandAnchors ?? buildSourceCommandAnchors(raw.sourceMap);
  return {
    ...raw,
    sourceCommandAnchors,
    decompileLines: raw.decompileLines.map((line) => {
      if (line.sourceMapIndex != null) return line;
      const sourceMapIndex = raw.sourceMap.findIndex(
        (entry) => entry.commandGroup === line.group && entry.sourceLines[0] <= line.lineIndex && line.lineIndex < entry.sourceLines[1],
      );
      return {
        ...line,
        sourceMapIndex: sourceMapIndex >= 0 ? sourceMapIndex : undefined,
      };
    }),
  };
}

function buildSourceCommandAnchors(sourceMap: SourceMapEntry[]): SourceCommandAnchor[] {
  const anchors: SourceCommandAnchor[] = [];
  sourceMap.forEach((entry, sourceMapIndex) => {
      if (entry.commandPath.length === 0) return;
      const root = entry.parentCommandGroup ?? entry.commandPath[0] ?? entry.commandGroup ?? null;
      anchors.push({
        kind: entry.parentCommandGroup == null ? "root" as const : "nested" as const,
        sourceMapIndex,
        commandPath: entry.commandPath,
        displayPath: displayCommandPath(entry.commandPath),
        parentRootGroup: root,
        commandGroup: entry.commandGroup,
        sourceLines: entry.sourceLines,
        originalSpan: entry.originalSpan,
        snippet: firstSourceLine(entry.text),
      });
    });
  return anchors;
}

function displayCommandPath(commandPath: number[]): string {
  const [root, ...nested] = commandPath;
  if (root == null) return "";
  return [`c${String(root).padStart(3, "0")}`, ...nested.map((part) => `n${String(part).padStart(3, "0")}`)].join("/");
}

function firstSourceLine(text: string): string {
  return text.split("\n").map((line) => line.trim()).find(Boolean) ?? "";
}

// ─── Commands ────────────────────────────────────────────────────

/**
 * Backend capability handshake. `debuggerAvailable` gates debug UI so a
 * non-debugger build degrades explicitly instead of surfacing raw IPC
 * errors. New capability fields are additive; treat absent as unavailable.
 */
export interface AppCapabilities {
  debugger_available: boolean;
}

export async function getAppCapabilities(): Promise<AppCapabilities> {
  try {
    const capabilities = await invoke<AppCapabilities | null | undefined>("get_app_capabilities");
    // Defensive normalization: only an explicit `false` degrades debug UI.
    // A null/malformed payload keeps the historical default so a handshake
    // bug can never brick working debug controls.
    return { debugger_available: capabilities?.debugger_available !== false };
  } catch {
    // Older backend without the handshake: assume the historical default
    // (debugger feature compiled in) rather than hiding working UI.
    return { debugger_available: true };
  }
}

/**
 * File handed over by the launcher (`tcld ui <file>` / `tcld-tauri <file>`).
 * The backend yields it once; outside the desktop host, or against an older
 * backend without the command, there is nothing to open.
 */
export async function takeLaunchOpenPath(): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    return (await invoke<string | null | undefined>("take_launch_open_path")) ?? null;
  } catch {
    return null;
  }
}

export async function loadFile(path: string): Promise<LoadResult> {
  return consumeLoadResult(await invoke<LoadResultRaw>("load_file", { path }));
}

/**
 * Load a Tcl source snippet directly from text (no path required).
 * Uses the source compiler admission rules without executing the script.
 */
export async function loadText(source: string, label: string): Promise<LoadResult> {
  return consumeLoadResult(await invoke<LoadResultRaw>("load_text", { source, label }));
}

export async function loadExample(id: string): Promise<LoadResult> {
  return consumeLoadResult(await invoke<LoadResultRaw>("load_example", { id }));
}

export async function getInstructionDetail(index: number): Promise<InstructionDetail> {
  return invoke<InstructionDetail>("get_instruction_detail", { index });
}

export async function getDecompileFull(): Promise<string> {
  return invoke<string>("get_decompile_full");
}

export async function disassembleFile(path: string): Promise<string> {
  return invoke<string>("disassemble_file", { path });
}

export async function decompileFile(path: string): Promise<string> {
  return invoke<string>("decompile_file", { path });
}

export async function readTextFile(path: string): Promise<string> {
  return invoke<string>("read_text_file", { path });
}

export async function saveTextFile(path: string, contents: string): Promise<void> {
  return invoke<void>("save_text_file", { path, contents });
}

export async function highlightTclSource(source: string): Promise<SyntaxToken[]> {
  return invoke<SyntaxToken[]>("highlight_tcl_source", { source });
}

export async function foldTclSource(source: string): Promise<FoldingRange[]> {
  return invoke<FoldingRange[]>("fold_tcl_source", { source });
}

export async function formatTclSource(request: FormatRequest): Promise<string> {
  return invoke<string>("format_tcl_source", { request });
}

export async function planTclFormatEdits(request: FormatRequest): Promise<FormatEdit[]> {
  return invoke<FormatEdit[]>("plan_tcl_format_edits", { request });
}

export async function openLaunchProfileWindow(): Promise<void> {
  return invoke<void>("open_launch_profile_window");
}

export async function openRunConfigurationWindow(): Promise<void> {
  return invoke<void>("open_run_configuration_window");
}

export async function openPreferencesWindow(): Promise<void> {
  return invoke<void>("open_preferences_window");
}

export interface AppearanceSettingsPayload {
  theme: string;
  typeSystem: string;
  density: string;
  showSparklines: boolean;
  groupRails: boolean;
  fontSize: number;
}

export async function loadAppearanceSettings(): Promise<AppearanceSettingsPayload | null> {
  return invoke<AppearanceSettingsPayload | null>("load_appearance_settings");
}

export async function saveAppearanceSettings(settings: AppearanceSettingsPayload): Promise<void> {
  return invoke<void>("save_appearance_settings", { settings });
}

// ─── Debug Commands ─────────────────────────────────────────────

export type BytecodeHookMode = "passthrough" | "intercept";

/** Which host-debugger transport a desktop launch uses. This is deliberately
 * not the CLI Auto/Host/Hybrid/VmStrict execution policy: the native desktop
 * debugger runs Tcl source in a host interpreter. */
export type DebugBackendKind = "embedded" | "external";

export interface DebugLaunchOptions {
  bytecodeHookMode?: BytecodeHookMode;
  bytecodeHookCommands?: string[];
  env?: Record<string, string>;
  cwd?: string | null;
  backend?: DebugBackendKind;
  /** External-backend override for the tclsh binary. */
  tclshPath?: string;
  /** External-backend override for the libtcld_dbg nub dylib. */
  dylibPath?: string;
}

export async function debugLaunch(
  path: string,
  args: string[] = [],
  tclshPath?: string,
  launchOptions?: DebugLaunchOptions,
): Promise<void> {
  return invoke<void>("debug_launch", { path, args, tclshPath, launchOptions });
}

export async function debugStop(): Promise<void> {
  return invoke<void>("debug_stop");
}

export async function debugStepInto(): Promise<void> {
  return invoke<void>("debug_step_into");
}

export async function debugStepInstruction(): Promise<void> {
  return invoke<void>("debug_step_instruction");
}

export async function debugStepOverInstruction(): Promise<void> {
  return invoke<void>("debug_step_over_instruction");
}

export async function debugStepOutInstruction(): Promise<void> {
  return invoke<void>("debug_step_out_instruction");
}

export async function debugRunToPc(pc: number): Promise<void> {
  return invoke<void>("debug_run_to_pc", { pc });
}

export async function debugStepOver(): Promise<void> {
  return invoke<void>("debug_step_over");
}

export async function debugStepOut(): Promise<void> {
  return invoke<void>("debug_step_out");
}

export async function debugContinue(): Promise<void> {
  return invoke<void>("debug_continue");
}

export async function debugPause(): Promise<void> {
  return invoke<void>("debug_pause");
}

export async function debugSetBreakpoint(
  file: string,
  line: number,
  condition?: string,
  offset?: number,
  len?: number,
): Promise<void> {
  return invoke<void>("debug_set_breakpoint", {
    file,
    line,
    condition,
    ...(offset == null ? {} : { offset }),
    ...(len == null ? {} : { len }),
  });
}

export async function debugClearBreakpoint(file: string, line: number, offset?: number): Promise<void> {
  return invoke<void>("debug_clear_breakpoint", {
    file,
    line,
    ...(offset == null ? {} : { offset }),
  });
}

export async function debugSetBreakpointEnabled(
  file: string,
  line: number,
  enabled: boolean,
  offset?: number,
): Promise<void> {
  return invoke<void>("debug_set_breakpoint_enabled", {
    file,
    line,
    enabled,
    ...(offset == null ? {} : { offset }),
  });
}

export async function debugListBreakpoints(): Promise<DebugBreakpoint[]> {
  return invoke<DebugBreakpoint[]>("debug_list_breakpoints");
}

export async function debugClearAllBreakpoints(): Promise<void> {
  return invoke<void>("debug_clear_all_breakpoints");
}

export async function debugEval(script: string, level: number = 1): Promise<void> {
  return invoke<void>("debug_eval", { script, level });
}

export async function debugGetVariables(level: number = 1): Promise<void> {
  return invoke<void>("debug_get_variables", { level });
}

export async function debugGetStack(): Promise<void> {
  return invoke<void>("debug_get_stack");
}

export async function debugSetVariable(
  name: string,
  value: string,
  level: number = 1,
): Promise<void> {
  return invoke<void>("debug_set_variable", { name, value, level });
}

export interface DebugStateInfo {
  epoch?: number;
  state: string;
  scriptPath: string;
  tclVersion: string | null;
  pid: number | null;
  breakpointCount: number;
  chunkCount?: number;
}

export async function debugGetState(): Promise<DebugStateInfo> {
  return invoke<DebugStateInfo>("debug_get_state");
}

export interface DebugBytecodeChunkInfo {
  sequence: number;
  command: string;
  procName?: string | null;
  payloadSize: number;
  disassembly?: string | null;
  decompileText?: string | null;
  error?: string | null;
}

export async function debugGetBytecodeChunks(): Promise<DebugBytecodeChunkInfo[]> {
  return invoke<DebugBytecodeChunkInfo[]>("debug_get_bytecode_chunks");
}

export interface DebugEvalSyncResult {
  result: string;
  error: boolean;
  errorInfo?: string;
  errorCode?: string;
}

export async function debugEvalSync(
  script: string,
  level: number = 0,
): Promise<DebugEvalSyncResult> {
  return invoke<DebugEvalSyncResult>("debug_eval_sync", { script, level });
}

export async function debugDisassemble(path: string): Promise<string> {
  return invoke<string>("debug_disassemble", { path });
}

export async function debugResolveCommand(
  name: string,
  namespace?: string,
): Promise<CommandResolution> {
  return invoke<CommandResolution>("debug_resolve_command", { name, namespace });
}

export async function debugSetWatchpoint(
  name: string,
  scope?: string,
  level?: number,
  condition?: string,
): Promise<void> {
  return invoke<void>("debug_set_watchpoint", { name, scope, level, condition });
}

export async function debugClearWatchpoint(name: string): Promise<void> {
  return invoke<void>("debug_clear_watchpoint", { name });
}

export async function debugListWatchpoints(): Promise<WatchpointInfo[]> {
  return invoke<WatchpointInfo[]>("debug_list_watchpoints");
}

export async function debugClearAllWatchpoints(): Promise<void> {
  return invoke<void>("debug_clear_all_watchpoints");
}

// ─── Tauri Event Emitter ────────────────────────────────────────

export async function emit<T>(event: string, payload: T): Promise<void> {
  if (typeof window !== "undefined" && window.__TCLD_TEST_TAURI__?.emit) {
    await window.__TCLD_TEST_TAURI__.emit(event, payload);
    return;
  }
  if (typeof window !== "undefined" && window.__TAURI_INTERNALS__) {
    const { emit: tauriEmit } = await import("@tauri-apps/api/event");
    await tauriEmit(event, payload);
  }
}

// ─── Tauri Event Listener ───────────────────────────────────────

export async function listen<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  if (typeof window !== "undefined" && window.__TCLD_TEST_TAURI__?.listen) {
    return window.__TCLD_TEST_TAURI__.listen(event, handler);
  }
  if (typeof window !== "undefined" && window.__TAURI_INTERNALS__) {
    const { listen: tauriListen } = await import("@tauri-apps/api/event");
    const unlisten = await tauriListen<T>(event, (e) => handler(e.payload));
    return unlisten;
  }
  return () => {};
}
