import type { BasicBlock, Instruction, SourceCommandAnchor, SourceMapEntry, TcldData } from "@/types";
import type { Location } from "@/lib/location-resolve";

export type GraphEdgeType = "fallthrough" | "unconditional" | "conditional-true" | "conditional-false";
export type GraphMode = "basic-block" | "source-snippet";
export type GraphBlockRole = "entry" | "branch" | "loop" | "exit" | "linear";
export type GraphVisualRole = "selected" | "execution" | "predecessor" | "successor" | "normal";

export interface GraphViewModelOptions {
  mode: GraphMode;
  cursor: Partial<Location> | null;
  execution: { blockId: number | null } | null;
}

export interface GraphNodeViewModel {
  id: string;
  kind: GraphMode;
  flowRole: "flow-node" | "source-anchor";
  blockRole: GraphBlockRole;
  label: string;
  detail: string;
  visualRoles: GraphVisualRole[];
  blockId: number | null;
  pcRange: [number, number] | null;
  commandGroup: string | null;
  instructionCount: number | null;
  terminalOpcode: string | null;
  exitStatus: string | null;
  commandPath: string | null;
  sourceLineRange: [number, number] | null;
  snippet: string | null;
  cursorTarget: Partial<Location>;
}

export interface GraphEdgeViewModel {
  id: string;
  sourceId: string;
  targetId: string;
  sourceBlockId: number | null;
  targetBlockId: number | null;
  role: GraphEdgeType;
  label: string;
}

export interface GraphViewModel {
  mode: GraphMode;
  nodes: GraphNodeViewModel[];
  edges: GraphEdgeViewModel[];
  actions: string[];
  metrics: GraphMetrics;
}

export interface GraphMetrics {
  blockCount: number;
  edgeCount: number;
  loopCount: number;
  branchCount: number;
  cyclomatic: number;
  depth: number;
}

function blockTerminalInstruction(block: BasicBlock, instructions: Instruction[]): Instruction | null {
  const terminalPc = block.instructions[block.instructions.length - 1]?.pc;
  if (terminalPc == null) return null;
  return instructions.find((inst) => inst.pc === terminalPc) ?? null;
}

export function classifyGraphEdge(
  block: BasicBlock,
  targetId: number,
  instructions: Instruction[],
  blocks: BasicBlock[],
): GraphEdgeType {
  const inst = blockTerminalInstruction(block, instructions);
  if (!inst?.isJump) return "fallthrough";
  if (inst.isJumpUnconditional) return "unconditional";
  if (!inst.isJumpConditional) return "fallthrough";

  const targetBlock = blocks.find((candidate) => candidate.id === targetId);
  const isJumpTarget = inst.jumpTarget != null && targetBlock != null && targetBlock.startPc === inst.jumpTarget;
  return isJumpTarget ? "conditional-true" : "conditional-false";
}

export function buildGraphViewModel(data: TcldData, options: GraphViewModelOptions): GraphViewModel {
  const metrics = computeGraphMetrics(data.blocks, data.instructions);
  if (options.mode === "source-snippet") {
    return {
      mode: "source-snippet",
      nodes: buildSourceSnippetNodes(data),
      edges: [],
      actions: getGraphActionIds(),
      metrics,
    };
  }

  const predecessorMap = buildPredecessorMap(data.blocks);
  const selectedBlockId = options.cursor?.blockId ?? blockIdForInstruction(data, options.cursor?.instIdx ?? null);
  const executionBlockId = options.execution?.blockId ?? null;
  const selectedPredecessors = new Set(selectedBlockId != null ? predecessorMap.get(selectedBlockId) ?? [] : []);
  const selectedSuccessors = new Set(selectedBlockId != null ? data.blocks.find((block) => block.id === selectedBlockId)?.edges ?? [] : []);

  return {
    mode: "basic-block",
    nodes: data.blocks.map((block) => {
      const terminal = terminalInstructionSummary(block);
      const roles: GraphVisualRole[] = [];
      if (block.id === selectedBlockId) roles.push("selected");
      if (block.id === executionBlockId) roles.push("execution");
      if (selectedPredecessors.has(block.id)) roles.push("predecessor");
      if (selectedSuccessors.has(block.id)) roles.push("successor");
      if (roles.length === 0) roles.push("normal");
      return {
        id: `block:${block.id}`,
        kind: "basic-block",
        flowRole: "flow-node",
        blockRole: deriveGraphBlockRole(block, data.blocks, data.instructions),
        label: `b${String(block.id).padStart(3, "0")}`,
        detail: `${block.startPc}..${block.endPc} ${block.group}`,
        visualRoles: roles,
        blockId: block.id,
        pcRange: [block.startPc, block.endPc],
        commandGroup: block.group,
        instructionCount: block.instructions.length,
        terminalOpcode: terminal,
        exitStatus: block.edges.length === 0 ? "exit" : terminal ?? "fallthrough",
        commandPath: null,
        sourceLineRange: null,
        snippet: null,
        cursorTarget: {
          blockId: block.id,
          instIdx: block.start,
          pc: block.startPc,
          group: parseCommandGroup(block.group),
        },
      } satisfies GraphNodeViewModel;
    }),
    edges: data.blocks.flatMap((block) =>
      block.edges.map((targetId) => ({
        id: `edge:${block.id}:${targetId}`,
        sourceId: `block:${block.id}`,
        targetId: `block:${targetId}`,
        sourceBlockId: block.id,
        targetBlockId: targetId,
        role: classifyGraphEdgeWithFallback(block, targetId, data.instructions, data.blocks),
        label: edgeLabel(classifyGraphEdgeWithFallback(block, targetId, data.instructions, data.blocks)),
      })),
    ),
    actions: getGraphActionIds(),
    metrics,
  };
}

export function graphNodeRenderAttrs(node: GraphNodeViewModel): Record<string, string> {
  const attrs: Record<string, string> = {
    "data-graph-mode": node.kind,
    "data-node-kind": node.kind,
    "data-flow-role": node.flowRole,
    "data-flow-block-role": node.blockRole,
    "data-node-id": node.id,
    className: [
      "graph-node",
      `graph-node-${node.kind}`,
      `graph-block-role-${node.blockRole}`,
      ...node.visualRoles.map((role) => `graph-role-${role}`),
    ].join(" "),
  };
  for (const role of node.visualRoles) {
    attrs[`data-visual-${role}`] = "true";
  }
  if (node.blockId != null) attrs["data-block-id"] = String(node.blockId);
  if (node.commandPath) attrs["data-command-path"] = node.commandPath;
  return attrs;
}

export function graphEdgeRenderAttrs(edge: GraphEdgeViewModel): Record<string, string> {
  return {
    "data-edge-id": edge.id,
    "data-edge-role": edge.role,
    className: `graph-edge graph-edge-${edge.role}`,
  };
}

export function getGraphActionIds(): string[] {
  return [
    "graph:mode-basic-block",
    "graph:mode-source-snippet",
    "graph:fit",
    "graph:center-selection",
    "graph:expand-node",
    "graph:copy-details",
    "graph:detail-compact",
    "graph:detail-medium",
    "graph:detail-full",
    "graph:zoom-in",
    "graph:zoom-out",
  ];
}

export function computeGraphMetrics(blocks: BasicBlock[], instructions: Instruction[]): GraphMetrics {
  const edgeCount = blocks.reduce((sum, block) => sum + block.edges.length, 0);
  const loopCount = blocks.reduce((sum, block) => sum + block.edges.filter((targetId) => isBackEdge(block, targetId, blocks)).length, 0);
  const branchCount = blocks.filter((block) => isBranchBlock(block, instructions, blocks)).length;
  return {
    blockCount: blocks.length,
    edgeCount,
    loopCount,
    branchCount,
    cyclomatic: blocks.length > 0 ? Math.max(1, edgeCount - blocks.length + 2) : 0,
    depth: computeGraphDepth(blocks),
  };
}

export function deriveGraphBlockRole(block: BasicBlock, blocks: BasicBlock[], instructions: Instruction[]): GraphBlockRole {
  if (hasNoPredecessors(block, blocks)) return "entry";
  if (block.edges.length === 0) return "exit";
  if (block.edges.some((targetId) => isBackEdge(block, targetId, blocks))) return "loop";
  if (isBranchBlock(block, instructions, blocks)) return "branch";
  return "linear";
}

function buildSourceSnippetNodes(data: TcldData): GraphNodeViewModel[] {
  const anchors = data.sourceCommandAnchors ?? [];
  if (anchors.length > 0) {
    return anchors.map((anchor, index) => sourceAnchorNode(data, anchor, index));
  }
  return (data.sourceMap ?? []).map((entry, index) => sourceMapNode(data, entry, index));
}

function sourceAnchorNode(data: TcldData, anchor: SourceCommandAnchor, index: number): GraphNodeViewModel {
  const entry = data.sourceMap?.[anchor.sourceMapIndex];
  const blockId = blockIdForGroup(data, anchor.parentRootGroup ?? anchor.commandGroup ?? entry?.commandGroup ?? null);
  return {
    id: `source:${index}`,
    kind: "source-snippet",
    flowRole: "source-anchor",
    blockRole: blockRoleForId(data, blockId),
    label: anchor.displayPath,
    detail: `lines ${anchor.sourceLines[0]}..${anchor.sourceLines[1]}`,
    visualRoles: ["normal"],
    blockId,
    pcRange: entry?.bytecodeRange ?? null,
    commandGroup: anchor.commandGroup != null ? `c${String(anchor.commandGroup).padStart(3, "0")}` : null,
    instructionCount: null,
    terminalOpcode: null,
    exitStatus: null,
    commandPath: anchor.displayPath,
    sourceLineRange: anchor.sourceLines,
    snippet: anchor.snippet,
    cursorTarget: sourceTargetForEntry(data, anchor.sourceMapIndex, anchor.sourceLines[0]),
  };
}

function sourceMapNode(data: TcldData, entry: SourceMapEntry, index: number): GraphNodeViewModel {
  const commandPath = displayCommandPath(entry.commandPath);
  const blockId = blockIdForGroup(data, entry.commandGroup);
  return {
    id: `source:${index}`,
    kind: "source-snippet",
    flowRole: "source-anchor",
    blockRole: blockRoleForId(data, blockId),
    label: commandPath || `source:${index}`,
    detail: `lines ${entry.sourceLines[0]}..${entry.sourceLines[1]}`,
    visualRoles: ["normal"],
    blockId,
    pcRange: entry.bytecodeRange,
    commandGroup: entry.commandGroup != null ? `c${String(entry.commandGroup).padStart(3, "0")}` : null,
    instructionCount: null,
    terminalOpcode: null,
    exitStatus: null,
    commandPath,
    sourceLineRange: entry.sourceLines,
    snippet: entry.text.split("\n").map((line) => line.trim()).find(Boolean) ?? "",
    cursorTarget: sourceTargetForEntry(data, index, entry.sourceLines[0]),
  };
}

function sourceTargetForEntry(data: TcldData, sourceMapIndex: number, sourceLine: number): Partial<Location> {
  const entry = data.sourceMap?.[sourceMapIndex];
  const instruction = entry
    ? data.instructions.find((inst) => inst.group === entry.commandGroup && inst.groupStart)
      ?? data.instructions.find((inst) => entry.bytecodeRange[0] <= inst.pc && inst.pc < entry.bytecodeRange[1])
    : null;
  return {
    sourceLine,
    decompileLine: data.decompileLines.findIndex((line) => line.sourceMapIndex === sourceMapIndex),
    group: entry?.commandGroup ?? instruction?.group ?? null,
    instIdx: instruction?.idx ?? null,
    pc: instruction?.pc ?? entry?.bytecodeRange[0] ?? null,
    blockId: instruction ? blockIdForInstruction(data, instruction.idx) : null,
  };
}

function blockIdForInstruction(data: TcldData, instIdx: number | null): number | null {
  if (instIdx == null) return null;
  return data.blocks.find((block) => block.start <= instIdx && instIdx <= block.end)?.id ?? null;
}

function blockIdForGroup(data: TcldData, group: number | null): number | null {
  if (group == null) return null;
  const instruction = data.instructions.find((inst) => inst.group === group);
  return instruction ? blockIdForInstruction(data, instruction.idx) : null;
}

function buildPredecessorMap(blocks: BasicBlock[]): Map<number, number[]> {
  const map = new Map<number, number[]>();
  for (const block of blocks) {
    for (const target of block.edges) {
      const predecessors = map.get(target) ?? [];
      predecessors.push(block.id);
      map.set(target, predecessors);
    }
  }
  return map;
}

function blockRoleForId(data: TcldData, blockId: number | null): GraphBlockRole {
  const block = blockId == null ? null : data.blocks.find((candidate) => candidate.id === blockId);
  return block ? deriveGraphBlockRole(block, data.blocks, data.instructions) : "linear";
}

function hasNoPredecessors(block: BasicBlock, blocks: BasicBlock[]): boolean {
  return !blocks.some((candidate) => candidate.edges.includes(block.id));
}

function isBackEdge(block: BasicBlock, targetId: number, blocks: BasicBlock[]): boolean {
  const target = blocks.find((candidate) => candidate.id === targetId);
  return target ? target.startPc <= block.startPc : targetId <= block.id;
}

function isBranchBlock(block: BasicBlock, instructions: Instruction[], blocks: BasicBlock[]): boolean {
  if (block.edges.length > 1) return true;
  return block.edges.some((targetId) => {
    const role = classifyGraphEdgeWithFallback(block, targetId, instructions, blocks);
    return role === "conditional-true" || role === "conditional-false";
  });
}

function computeGraphDepth(blocks: BasicBlock[]): number {
  if (blocks.length === 0) return 0;
  const byId = new Map(blocks.map((block) => [block.id, block]));
  const memo = new Map<number, number>();
  const visit = (block: BasicBlock, seen: Set<number>): number => {
    const cached = memo.get(block.id);
    if (cached != null) return cached;
    if (seen.has(block.id)) return 0;
    const nextSeen = new Set(seen);
    nextSeen.add(block.id);
    const childDepth = block.edges
      .map((targetId) => byId.get(targetId))
      .filter((target): target is BasicBlock => target != null)
      .filter((target) => target.startPc > block.startPc)
      .reduce((max, target) => Math.max(max, visit(target, nextSeen)), 0);
    const depth = 1 + childDepth;
    memo.set(block.id, depth);
    return depth;
  };
  return blocks.reduce((max, block) => Math.max(max, visit(block, new Set())), 0);
}

function terminalInstructionSummary(block: BasicBlock): string | null {
  return block.instructions[block.instructions.length - 1]?.name ?? null;
}

function classifyGraphEdgeWithFallback(
  block: BasicBlock,
  targetId: number,
  instructions: Instruction[],
  blocks: BasicBlock[],
): GraphEdgeType {
  const edge = classifyGraphEdge(block, targetId, instructions, blocks);
  const terminal = blockTerminalInstruction(block, instructions);
  if (edge !== "conditional-false" || !terminal?.isJumpConditional) return edge;
  const target = blocks.find((candidate) => candidate.id === targetId);
  if (terminal.fallthrough != null && target?.startPc === terminal.fallthrough) return "conditional-false";
  if (terminal.jumpTarget != null && target?.startPc === terminal.jumpTarget) return "conditional-true";
  return "conditional-true";
}

function edgeLabel(edge: GraphEdgeType): string {
  switch (edge) {
    case "conditional-true":
      return "true";
    case "conditional-false":
      return "false";
    case "unconditional":
      return "jump";
    case "fallthrough":
      return "fallthrough";
  }
}

function parseCommandGroup(group: string): number | null {
  const match = /^c(\d+)$/.exec(group);
  return match ? Number(match[1]) : null;
}

function displayCommandPath(commandPath: number[]): string {
  const [root, ...nested] = commandPath;
  if (root == null) return "";
  return [`c${String(root).padStart(3, "0")}`, ...nested.map((part) => `n${String(part).padStart(3, "0")}`)].join("/");
}
