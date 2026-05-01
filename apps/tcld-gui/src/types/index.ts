// ─── tcld Data Model Types ────────────────────────────────────────
// Mirrors the Rust Tauri command DTOs for frontend consumption.

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
  }
}

export type ViewId =
  | "instructions"
  | "literals"
  | "exceptions"
  | "graph"
  | "decompile"
  | "source"
  | "comparison"
  | "diagnostics"
  | "provenance"
  | "project"
  | "summary"
  | "detail"
  | "variables"
  | "stack"
  | "console"
  | "breakpoints"
  | "watchpoints"
  | "commandResolver";

export interface ViewDef {
  id: ViewId;
  label: string;
  icon: string;
  iconRole: string;
  key: string;
}

export type WorkspaceLeafId = string;

export interface ViewStackLeaf {
  leafId: WorkspaceLeafId;
  views: ViewId[];
  activeView: ViewId;
}

export type ViewStacks = Record<WorkspaceLeafId, ViewStackLeaf>;

// ─── Mosaic Layout Types ─────────────────────────────────────────

export type { MosaicNode, MosaicParent, MosaicPath, MosaicBranch, MosaicDirection } from "react-mosaic-component";
export type WorkspaceTree = import("react-mosaic-component").MosaicNode<WorkspaceLeafId>;
export type ViewLayoutTree = import("react-mosaic-component").MosaicNode<ViewId>;

export interface WorkspaceLayoutSnapshot {
  tree: WorkspaceTree | null;
  stacks: ViewStacks;
}

export interface LayoutPreset {
  name: string;
  label: string;
  tree: ViewLayoutTree;
  /** Pre-built workspace with multi-view tab stacks (takes priority over tree). */
  workspace?: WorkspaceLayoutSnapshot;
}

// ─── Bytecode Data Types ─────────────────────────────────────────

export interface Instruction {
  idx: number;
  pc: number;
  op: string;
  args: string;
  operands: number[];
  argc: number;
  group: number | null;
  commandPcRange: [number, number] | null;
  groupStart: boolean;
  comment: string;
  size: number;
  isJump: boolean;
  isJumpConditional: boolean;
  isJumpUnconditional: boolean;
  jumpTarget: number | null;
  fallthrough: number | null;
  primaryNextLocation: number | null;
  secondaryNextLocation: number | null;
  nextLocations: number[];
}

export interface DecompileLine {
  text: string;
  group: number | null;
  lineIndex: number;
  sourceLineStart?: number;
  sourceLineEnd?: number;
  sourceMapIndex?: number;
}

export type SourceAvailability = "available" | "unavailable";

export interface SourceMapEntry {
  bytecodeRange: [number, number];
  sourceLines: [number, number];
  sourceChars: [number, number];
  text: string;
  commandGroup: number | null;
  commandPath: number[];
  parentCommandGroup: number | null;
  originalStatus: SourceAvailability;
  originalSpan: [number, number] | null;
}

export type SourceCommandAnchorKind = "root" | "nested";

export interface SourceCommandAnchor {
  kind: SourceCommandAnchorKind;
  sourceMapIndex: number;
  commandPath: number[];
  displayPath: string;
  parentRootGroup: number | null;
  commandGroup?: number | null;
  sourceLines: [number, number];
  originalSpan?: [number, number] | null;
  snippet: string;
}

export type ApproximationKind =
  | "NestedProcedure"
  | "UnresolvedInstruction";

export interface DiagnosticApproximation {
  kind: ApproximationKind;
  bytecodeRange: [number, number] | null;
  reason: string;
}

export interface PassDecision {
  name: string;
  enabled: boolean;
  iteration: number;
  changed: boolean;
}

export type ReconstructionStatus = "Strict" | "Approximation" | "Failed";

export interface DiagnosticsReport {
  reconstruction: ReconstructionStatus;
  recognizedConstructs: string[];
  unreducedOpcodes: string[];
  approximations: DiagnosticApproximation[];
  suppressedFastPathRanges: [number, number][];
  passDecisions: PassDecision[];
  unsupportedInstructions: string[];
  suggestedNextWork: string[];
}

export interface LoadProvenance {
  kind: string;
  label: string;
  originalSourceAvailable: boolean;
}

export interface LiteralEntry {
  idx: number;
  kind: string;
  value: string;
}

export type SymbolKind = "ns" | "proc" | "var";

export interface SymbolEntry {
  kind: SymbolKind;
  name: string;
  location?: string | null;
  address?: number | string | null;
  size?: number | null;
  refs?: number | null;
  sourceMapIndex?: number | null;
  bytecodeRange?: [number, number] | null;
}

export interface ExceptionEntry {
  idx: number;
  kind: string;
  nestingLevel: number;
  codeRange: [number, number];
  breakOffset: number;
  continueOffset: number;
  catchOffset: number;
}

export interface BasicBlock {
  id: number;
  start: number;
  end: number;
  startPc: number;
  endPc: number;
  group: string;
  edges: number[];
  instructions: { pc: number; name: string; operands: number[] }[];
}

export interface InstructionDetail {
  instructionIndex: number;
  opcode: string;
  pc: number;
  size: number;
  argc: number;
  operands: number[];
  commandGroup: string;
  commandPcRange: string;
  jumps: { conditional: boolean; unconditional: boolean };
  jumpTarget: string;
  fallthrough: string;
  value: string | null;
}

export interface FileMetadata {
  filename: string;
  path: string;
  tclVersion: string;
  instructionCount: number;
  literalCount: number;
  commandCount: number;
  exceptionCount: number;
  auxCount: number;
  codeBytes: number;
  maxStackDepth: number;
  syntaxEngine: string;
  format: number | null;
  build: number | null;
}

export type FocusedPane = "list" | "decompile" | "detail";

export interface PaletteItem {
  label: string;
  hint: string;
  cmd: string;
  category?: string;
}

export interface TraceFrame {
  instructionIndex: number;
  pc: number;
  opcode: string;
  argc: number;
  stackDelta: number;
  beforeDepth: number;
  afterDepth: number;
  popped: string[];
  pushed: string[];
}

export interface SyntaxToken {
  capture: string;
  startByte: number;
  endByte: number;
  startRow: number;
  startColumn: number;
  endRow: number;
  endColumn: number;
}

export type FoldingRangeKind = "region" | "comment";

export interface FoldingRange {
  kind: FoldingRangeKind;
  startByte: number;
  endByte: number;
  startRow: number;
  startColumn: number;
  endRow: number;
  endColumn: number;
}

export interface FormatRequest {
  source: string;
  trimTrailingWhitespace?: boolean;
  collapseBlankLines?: boolean;
  ensureTrailingNewline?: boolean;
}

export interface FormatEdit {
  startByte: number;
  endByte: number;
  replacement: string;
}

export interface TcldData {
  metadata: FileMetadata;
  instructions: Instruction[];
  literals: LiteralEntry[];
  exceptions: ExceptionEntry[];
  blocks: BasicBlock[];
  traces: TraceFrame[];
  symbols?: SymbolEntry[];
  decompileLines: DecompileLine[];
  decompileTokens: SyntaxToken[];
  decompileFoldingRanges?: FoldingRange[];
  decompileText: string;
  dumpText: string;
  summaryText: string;
  sourceMap?: SourceMapEntry[];
  sourceCommandAnchors?: SourceCommandAnchor[];
  diagnostics?: DiagnosticsReport;
  reconstruction?: ReconstructionStatus;
  provenance?: LoadProvenance;
  originalSourceText?: string | null;
}

// ─── Debug Types ────────────────────────────────────────────────

export type DebugSessionState =
  | "idle"
  | "launching"
  | "connected"
  | "running"
  | "paused"
  | "pausing"
  | "stopping"
  | "terminated";

export interface DebugStoppedEvent {
  reason: string;
  file: string;
  line: number;
  frameCount: number;
  pc: number | null;
  instructionIndex: number | null;
  blockId: number | null;
  error?: string;
  errorInfo?: string;
  watchpointName?: string;
  watchpointElement?: string;
  watchpointOldValue?: string;
  watchpointNewValue?: string;
  watchpointOperation?: string;
}

export interface DebugOutputEvent {
  stream: "stdout" | "stderr";
  text: string;
}

export interface DebugExitedEvent {
  code: number;
  reason?: string;
}

export interface DebugReadyEvent {
  tclVersion: string;
  pid: number;
  debugExtensionLoaded?: boolean | null;
  debugExtensionInstalled?: boolean | null;
  debugExtensionCapabilities?: string | null;
  debugExtensionReason?: string | null;
}

export interface DebugBytecodeChunkEvent {
  sequence: number;
  command: string;
  procName?: string | null;
  payloadSize: number;
  disassembly?: string | null;
  decompileText?: string | null;
  error?: string | null;
}

export interface DebugCommandErrorEvent {
  command: string;
  controlMode: string;
  code: number;
  message: string;
  errorInfo?: string;
  errorCode?: string;
  argc: number;
}

export interface DebugVariable {
  name: string;
  value: string;
  type: string;
  scope: string;
  internalRep?: string;
  internalRepDetail?: string;
  hasStringRep?: boolean;
  refCount?: number;
  changed?: boolean;
  children?: DebugVariable[];
}

export interface DebugStackFrame {
  level: number;
  type: string;
  proc: string | null;
  file: string | null;
  line: number | null;
  cmd: string | null;
}

export interface DebugResumedEvent {
  controlMode: string;
}

export interface DebugEvalResult {
  requestId?: number;
  result: string;
  error: boolean;
  errorInfo?: string;
  errorCode?: string;
}

export type DebugBreakpointKind = "source" | "instruction";

export interface DebugBreakpoint {
  kind?: DebugBreakpointKind;
  file?: string;
  line?: number;
  offset?: number;
  len?: number;
  pc?: number;
  condition?: string;
  enabled: boolean;
  hitCount?: number;
}

export interface CommandResolution {
  originalName: string;
  resolvedName: string;
  fullyQualifiedName: string;
  namespace: string;
  resolvedInNamespace: string;
  isImported: boolean;
  importSource?: string;
  importChain: string[];
  isEnsemble: boolean;
  ensembleNamespace?: string;
  isAlias: boolean;
  aliasTarget?: string;
  commandType: string;
  exists: boolean;
  resolutionPath: ResolutionStep[];
}

export interface ResolutionStep {
  step: string;
  namespace?: string;
  name?: string;
  source?: string;
  target?: string;
  result: string;
}

// ─── Reserved debug event kinds (v0.4+ gated runtimes) ──────────
// Forward-visibility vocabulary for the four gated runtime features.
// Nothing emits these in v1 — the backend reserves matching AgentEvent
// variants and `debugger:*` channels (apps/tcld-tauri/src/debugger/
// protocol.rs), and the debug store records any arrival as an unhandled
// event kind instead of breaking. Every field is optional so the shapes
// remain forward-compatible while the runtimes are gated.

/** Reserved for gated:live-disassembly-runtime. */
export interface DebugDisassemblyContextEvent {
  procName?: string | null;
  sourceFile?: string | null;
  disassembly?: string;
  /** [pcOffset, sourceLine] pairs mapping the disassembly to source. */
  pcToLine?: [number, number][];
}

/** Reserved for gated:opcode-profiling-runtime. */
export interface DebugLineProfileEntry {
  file?: string;
  line?: number;
  count?: number;
  totalNs?: number | null;
  minNs?: number | null;
  maxNs?: number | null;
}

/** Reserved for gated:opcode-profiling-runtime. */
export interface DebugOpcodeProfileEntry {
  pc?: number;
  count?: number;
}

/** Reserved for gated:opcode-profiling-runtime. */
export interface DebugProfileDataEvent {
  requestId?: number;
  /** "line_counts" | "opcode_counts" | "line_timing" */
  mode?: string;
  lineData?: DebugLineProfileEntry[] | null;
  opcodeData?: DebugOpcodeProfileEntry[] | null;
}

/** Reserved for gated:epoch-tracking-runtime. */
export interface DebugEpochChangeEvent {
  command?: string;
  oldEpoch?: number;
  newEpoch?: number;
  /** "proc_redefine" | "rename" | "delete" | "namespace_delete" | "import" */
  trigger?: string;
  compileEpoch?: number;
  file?: string | null;
  line?: number | null;
}

/** Reserved for gated:compilation-introspection-runtime. */
export interface DebugCompilationEvent {
  /** "first_call" | "recompile" | "eval" | "source" | "namespace_eval" */
  trigger?: string;
  command?: string;
  file?: string | null;
  line?: number | null;
  bytecodeSize?: number;
  instructionCount?: number | null;
  literalCount?: number | null;
  disassembly?: string | null;
  oldEpoch?: number | null;
  newEpoch?: number | null;
}

/**
 * Event kinds reserved ahead of their runtimes. Each maps to the Tauri
 * channel `debugger:<kind>`. The debug store subscribes to all of them
 * with a tolerant default branch (log to output, never throw) so a newer
 * backend emitting them degrades visibly instead of breaking the store.
 */
export const RESERVED_DEBUG_EVENT_KINDS = [
  "disassembly_context",
  "profile_data",
  "epoch_change",
  "compilation_event",
] as const;

export type ReservedDebugEventKind = (typeof RESERVED_DEBUG_EVENT_KINDS)[number];

export interface ReservedDebugEventPayloads {
  disassembly_context: DebugDisassemblyContextEvent;
  profile_data: DebugProfileDataEvent;
  epoch_change: DebugEpochChangeEvent;
  compilation_event: DebugCompilationEvent;
}

// ─── Watchpoint Types ───────────────────────────────────────────

export interface WatchpointInfo {
  id: number;
  name: string;
  scope: string;
  level?: number;
  condition?: string;
  active: boolean;
}

export interface DebugWatchpointHitEvent {
  watchId: number;
  name: string;
  element?: string;
  operation: string;
  oldValue: string;
  newValue: string;
  file: string;
  line: number;
  frameCount: number;
}
