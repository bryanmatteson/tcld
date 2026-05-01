import type { OutputLine } from "@/lib/debug-store";
import {
  LAYOUT_PRESETS,
  cloneViewStacks,
  findLeafForView,
  getDefaultWorkspace,
  getOpenViews,
  removeFromTree,
  workspaceFromViewTree,
} from "@/lib/default-layouts";
import { resolveDroppedFile } from "@/lib/gui-workflows";
import type { RecentFile } from "@/lib/recent-files";
import type { RunConfigurationState } from "@/lib/run-configuration-store";
import type { ViewId, WorkspaceLayoutSnapshot } from "@/types";

export const DESIGN_MIN_PANEL_WIDTH = 220;

export interface DesignOpenFileInput {
  path: string;
  filename: string;
  instructionCount: number;
  tclVersion: string;
}

export function designOpenFileTransition(input: DesignOpenFileInput) {
  const workspace = getDefaultWorkspace();
  return {
    routeState: "workspace" as const,
    workspace,
    recent: { path: input.path, filename: input.filename, mode: "open" as const },
    tab: {
      filename: input.filename,
      tclVersion: input.tclVersion,
      instructionCount: input.instructionCount,
      dirty: false,
    },
    actionBar: {
      state: "inspect" as const,
      enabledActions: ["open", "debug:launch-profile", "run-configuration:open-window", "layout:default", "layout:save"],
    },
    status: `loaded ${input.filename} - ${input.instructionCount} instructions`,
  };
}

export function designDropToOpenRequest(file: { name: string; path?: string }) {
  return resolveDroppedFile(file);
}

export function designRecentFileTransition(entry: RecentFile) {
  return {
    loadPath: entry.path,
    nextRecent: { ...entry, mode: entry.mode },
    routeState: entry.mode === "debug" ? "debug" : "workspace",
  };
}

export function applyDesignWorkspacePreset(name: string) {
  const preset = LAYOUT_PRESETS.find((candidate) => candidate.name === name);
  if (!preset) {
    throw new Error(`Unknown design workspace preset: ${name}`);
  }
  const workspace = preset.workspace ?? workspaceFromViewTree(preset.tree);
  return {
    workspace: cloneWorkspace(workspace),
    status: `layout: ${preset.label}`,
  };
}

export function designSwapWorkspaceView(
  workspace: WorkspaceLayoutSnapshot,
  oldView: ViewId,
  newView: ViewId,
): WorkspaceLayoutSnapshot {
  const stacks = cloneViewStacks(workspace.stacks);
  const oldLeaf = findLeafForView(stacks, oldView);
  if (!oldLeaf) return cloneWorkspace(workspace);

  const existingLeaf = findLeafForView(stacks, newView);
  if (existingLeaf && existingLeaf !== oldLeaf) {
    const existing = stacks[existingLeaf];
    existing.views = existing.views.filter((view) => view !== newView);
    if (existing.activeView === newView) existing.activeView = existing.views[0] ?? oldView;
    if (existing.views.length === 0) delete stacks[existingLeaf];
  }

  const target = stacks[oldLeaf];
  target.views = target.views.map((view) => view === oldView ? newView : view);
  target.activeView = target.activeView === oldView ? newView : target.activeView;
  return { tree: workspace.tree, stacks };
}

export function closeDesignWorkspaceView(
  workspace: WorkspaceLayoutSnapshot,
  viewId: ViewId,
): WorkspaceLayoutSnapshot {
  const openViews = getOpenViews(workspace.tree, workspace.stacks);
  if (openViews.length <= 1) return cloneWorkspace(workspace);

  const stacks = cloneViewStacks(workspace.stacks);
  const leafId = findLeafForView(stacks, viewId);
  if (!leafId) return cloneWorkspace(workspace);

  const stack = stacks[leafId];
  stack.views = stack.views.filter((view) => view !== viewId);
  let tree = workspace.tree;
  if (stack.views.length === 0) {
    delete stacks[leafId];
    tree = removeFromTree(tree, leafId);
  } else if (stack.activeView === viewId) {
    stack.activeView = stack.views[0];
  }
  return { tree, stacks };
}

export function designWorkspaceDirty(
  saved: WorkspaceLayoutSnapshot,
  current: WorkspaceLayoutSnapshot,
): boolean {
  return JSON.stringify(saved) !== JSON.stringify(current);
}

export function designSaveLayout(name: string, workspace: WorkspaceLayoutSnapshot) {
  return {
    name: `saved-${name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "")}`,
    label: name,
    workspace: cloneWorkspace(workspace),
  };
}

export function buildDesignViewActionContract() {
  return {
    instructions: {
      actions: ["instructions:fold-all", "instructions:filter", "instructions:unfold-all"],
      modelBackedBy: "folding-model",
    },
    source: {
      actions: ["source:fold-all", "source:split", "source:unfold-all", "source:linked-navigation"],
      modelBackedBy: "source-navigation",
    },
    graph: {
      actions: ["graph:detail-full", "graph:detail-medium", "graph:mode-basic-block", "graph:mode-source-snippet"],
      modelBackedBy: "graph-model",
    },
    provenance: {
      actions: ["trace:select-pass"],
      modelBackedBy: "buildProvenanceRows",
    },
    console: {
      actions: ["console:filter-channel", "console:submit-repl"],
      modelBackedBy: "debug-store.output",
    },
    comparison: { actions: ["comparison:select"], modelBackedBy: "buildComparisonModel" },
    detail: { actions: ["detail:select"], modelBackedBy: "TcldData.instructions" },
    diagnostics: { actions: ["diagnostics:select"], modelBackedBy: "DiagnosticsReport" },
  };
}

export function buildFlowControlModel(args: { detail: "compact" | "medium" | "full"; mode: "basic-block" | "source-snippet" }) {
  return {
    detail: args.detail,
    mode: args.mode,
    actionIds: [
      "graph:mode-basic-block",
      "graph:mode-source-snippet",
      "graph:detail-compact",
      "graph:detail-medium",
      "graph:detail-full",
    ],
  };
}

export function buildTraceSelectionModel(steps: string[], selected: string) {
  return {
    selected,
    steps: steps.map((step) => ({ id: step, selected: step === selected })),
  };
}

export function filterConsoleChannels(lines: OutputLine[], active: Set<OutputLine["stream"]>) {
  return lines.filter((line) => active.has(line.stream));
}

export function submitConsoleInput(script: string) {
  return { command: "debug:eval" as const, script };
}

export function designPreferenceSections() {
  return ["Appearance", "Editor", "Keybindings", "Decompiler", "Debugger", "Run profiles", "Telemetry", "Advanced"];
}

export function buildRunConfigurationWindowContract(state: RunConfigurationState) {
  return {
    selectedProfile: state.profiles.find((profile) => profile.id === state.selectedProfileId) ?? state.profiles[0] ?? null,
    eventsFromWindow: [
      "run-configuration:save",
      "run-configuration:cancel",
      "run-configuration:delete",
      "run-configuration:import",
      "run-configuration:export",
    ],
  };
}

export function reduceRunConfigurationWindowEvent(
  state: RunConfigurationState,
  event: { type: "delete"; id: string },
): RunConfigurationState {
  if (event.type === "delete") {
    const profiles = state.profiles.filter((profile) => profile.id !== event.id);
    return {
      ...state,
      profiles,
      selectedProfileId: state.selectedProfileId === event.id ? null : state.selectedProfileId,
      draft: state.draft?.id === event.id ? null : state.draft,
    };
  }
  return state;
}

function cloneWorkspace(workspace: WorkspaceLayoutSnapshot): WorkspaceLayoutSnapshot {
  return {
    tree: workspace.tree == null ? null : JSON.parse(JSON.stringify(workspace.tree)),
    stacks: cloneViewStacks(workspace.stacks),
  };
}
