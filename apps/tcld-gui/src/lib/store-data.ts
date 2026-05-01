import type {
  LoadProvenance,
  TcldData,
  InstructionDetail,
} from "@/types";
import type { LoadResult } from "@/lib/tauri-api";

export type { TcldData } from "@/types";

export type SessionDocumentRole = "open" | "recent" | "debug" | "live";

export interface DocumentDebuggerState {
  sessionState: string;
  scriptPath: string | null;
  stoppedFile: string | null;
  stoppedLine: number | null;
}

export interface DocumentWorkspaceState {
  selectedInst: number;
  activeLine: number | null;
  debuggerState: DocumentDebuggerState;
}

export interface SessionDocument {
  id: string;
  role: SessionDocumentRole;
  label: string;
  readOnly: boolean;
  data: TcldData;
  provenance: LoadProvenance;
  status: "ready" | "error";
  error?: string;
  workspaceState: DocumentWorkspaceState;
  /**
   * Absolute filesystem path the document was loaded from. Set when the
   * document originated from `loadFile(path)` (open or recent), undefined
   * for inline-text documents (`loadText`) and live debugger chunks.
   * Used by the LaunchControl Run button to launch debug for the current
   * target without re-prompting the user for a file.
   */
  sourcePath?: string;
}

export interface SessionState {
  documents: SessionDocument[];
  activeDocumentId: string | null;
  recentDocumentIds: string[];
}

export function tcldDataFromLoadResult(result: LoadResult): TcldData {
  return result;
}

export function createSessionState(): SessionState {
  return {
    documents: [],
    activeDocumentId: null,
    recentDocumentIds: [],
  };
}

export function buildSessionDocument(
  data: TcldData,
  options: {
    role: SessionDocumentRole;
    label?: string;
    status?: "ready" | "error";
    error?: string;
    workspaceState?: Partial<DocumentWorkspaceState>;
    sourcePath?: string;
  },
): SessionDocument {
  const label = options.label ?? data.metadata.filename;
  const provenance = data.provenance ?? {
    kind: options.role === "live" ? "DebuggerChunk" : "Path",
    label,
    originalSourceAvailable: false,
  };
  return {
    id: `${options.role}:${provenance.kind}:${provenance.label}:${data.metadata.instructionCount}:${data.metadata.codeBytes}`,
    role: options.role,
    label,
    readOnly: true,
    data,
    provenance,
    status: options.status ?? "ready",
    error: options.error,
    workspaceState: normalizeDocumentWorkspaceState(data, options.workspaceState),
    sourcePath: options.sourcePath,
  };
}

export function upsertSessionDocument(session: SessionState, document: SessionDocument): SessionState {
  const existing = session.documents.find((doc) => doc.id === document.id);
  const documents = session.documents.filter((doc) => doc.id !== document.id);
  documents.push(existing ? { ...document, workspaceState: existing.workspaceState } : document);
  const recentDocumentIds = [document.id, ...session.recentDocumentIds.filter((id) => id !== document.id)].slice(0, 20);
  return {
    documents,
    activeDocumentId: document.id,
    recentDocumentIds,
  };
}

export function closeSessionDocument(session: SessionState, documentId: string): SessionState {
  const documents = session.documents.filter((doc) => doc.id !== documentId);
  const recentDocumentIds = session.recentDocumentIds.filter((id) => id !== documentId);
  const activeDocumentId = session.activeDocumentId === documentId
    ? (recentDocumentIds.find((id) => documents.some((doc) => doc.id === id)) ?? documents[documents.length - 1]?.id ?? null)
    : (documents.some((doc) => doc.id === session.activeDocumentId) ? session.activeDocumentId : null);
  return {
    documents,
    activeDocumentId,
    recentDocumentIds,
  };
}

export function selectActiveDocument(session: SessionState, documentId: string): SessionState {
  if (!session.documents.some((doc) => doc.id === documentId)) {
    return session;
  }
  return {
    ...session,
    activeDocumentId: documentId,
    recentDocumentIds: [documentId, ...session.recentDocumentIds.filter((id) => id !== documentId)].slice(0, 20),
  };
}

export function updateSessionDocumentWorkspaceState(
  session: SessionState,
  documentId: string,
  patch: Partial<DocumentWorkspaceState>,
): SessionState {
  let changed = false;
  const documents = session.documents.map((doc) => {
    if (doc.id !== documentId) return doc;
    const workspaceState = normalizeDocumentWorkspaceState(doc.data, {
      ...doc.workspaceState,
      ...patch,
      debuggerState: {
        ...doc.workspaceState.debuggerState,
        ...patch.debuggerState,
      },
    });
    if (sameDocumentWorkspaceState(doc.workspaceState, workspaceState)) return doc;
    changed = true;
    return { ...doc, workspaceState };
  });
  return changed ? { ...session, documents } : session;
}

export function buildInstructionDetailFromData(data: TcldData, index: number): InstructionDetail | null {
  const inst = data.instructions[index];
  if (!inst) return null;

  const trace = data.traces[index];
  const extras: string[] = [];
  if (trace?.popped.length) {
    extras.push(`popped: [${trace.popped.join(", ")}]`);
  }
  if (trace?.pushed.length) {
    extras.push(`pushed: [${trace.pushed.join(", ")}]`);
  }
  if (trace) {
    extras.push(`stack: ${trace.beforeDepth} -> ${trace.afterDepth} (Delta ${trace.stackDelta})`);
  }
  const baseValue = inst.comment && !inst.isJump ? inst.comment : null;
  const value = [baseValue, ...extras].filter(Boolean).join("\n") || null;

  return {
    instructionIndex: inst.idx,
    opcode: inst.op,
    pc: inst.pc,
    size: inst.size,
    argc: inst.argc,
    operands: inst.operands,
    commandGroup: inst.group != null ? `c${String(inst.group).padStart(3, "0")}` : "n/a",
    commandPcRange: inst.commandPcRange ? `${inst.commandPcRange[0]}..${inst.commandPcRange[1]}` : "n/a",
    jumps: {
      conditional: inst.isJumpConditional,
      unconditional: inst.isJumpUnconditional,
    },
    jumpTarget: inst.jumpTarget != null ? String(inst.jumpTarget) : "n/a",
    fallthrough: inst.fallthrough != null ? String(inst.fallthrough) : "n/a",
    value,
  };
}

function normalizeDocumentWorkspaceState(
  data: TcldData,
  state: Partial<DocumentWorkspaceState> | undefined,
): DocumentWorkspaceState {
  const maxInstructionIndex = Math.max(0, data.instructions.length - 1);
  const selectedInst = Number.isFinite(state?.selectedInst)
    ? Math.max(0, Math.min(Math.floor(state!.selectedInst!), maxInstructionIndex))
    : 0;
  return {
    selectedInst,
    activeLine: typeof state?.activeLine === "number" ? state.activeLine : (data.decompileLines[0]?.sourceLineStart ?? null),
    debuggerState: {
      sessionState: state?.debuggerState?.sessionState ?? "idle",
      scriptPath: state?.debuggerState?.scriptPath ?? null,
      stoppedFile: state?.debuggerState?.stoppedFile ?? null,
      stoppedLine: state?.debuggerState?.stoppedLine ?? null,
    },
  };
}

function sameDocumentWorkspaceState(a: DocumentWorkspaceState, b: DocumentWorkspaceState): boolean {
  return a.selectedInst === b.selectedInst
    && a.activeLine === b.activeLine
    && a.debuggerState.sessionState === b.debuggerState.sessionState
    && a.debuggerState.scriptPath === b.debuggerState.scriptPath
    && a.debuggerState.stoppedFile === b.debuggerState.stoppedFile
    && a.debuggerState.stoppedLine === b.debuggerState.stoppedLine;
}
