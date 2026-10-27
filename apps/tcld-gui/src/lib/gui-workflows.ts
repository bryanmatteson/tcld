import type { DebugStoppedEvent, FoldingRange, FormatEdit, TcldData, ViewId } from "@/types";
import type { OutputLine } from "@/lib/debug-store";
import type { DebugLaunchOptions } from "@/lib/tauri-api";
import type { SessionState } from "@/lib/store-data";
import type { CommandContext } from "@/lib/command-registry";
import { formatOutputForCopy } from "@/lib/output-search";
import { buildComparisonModel, buildFullReport, buildProvenanceRows } from "@/lib/workflow-contracts";

export interface DebugLaunchRequest {
  path: string;
  args: string[];
  tclshPath?: string;
  launchOptions: DebugLaunchOptions;
  channelRouting?: LaunchChannelRouting;
}

export type LaunchChannelRoute = "console" | "discard";
export type LaunchExitBehavior = "keep-open" | "close" | "restart";

export interface LaunchChannelRouting {
  stdout: LaunchChannelRoute;
  stderr: LaunchChannelRoute;
  eval: LaunchChannelRoute;
  system: LaunchChannelRoute;
  forwardSignals: boolean;
  onExit: LaunchExitBehavior;
}

export interface DroppedFileResult {
  supported: boolean;
  path: string | null;
  filename: string;
  reason: "ready" | "unsupported" | "path-unavailable";
}

export interface ExportRequest {
  name: string;
  contents: string;
}

export function applyFormattedTextToData(
  data: TcldData,
  formatted: string,
  decompileFoldingRanges: FoldingRange[] = data.decompileFoldingRanges ?? [],
): TcldData {
  const lines = formatted.split("\n");
  return {
    ...data,
    decompileText: formatted,
    decompileFoldingRanges,
    decompileLines: lines.map((text, lineIndex) => {
      const existing = data.decompileLines[lineIndex];
      return existing
        ? {
          ...existing,
          text,
          lineIndex,
        }
        : {
          text,
          lineIndex,
          group: null,
        };
    }),
  };
}

export function parseLaunchArgs(argsText: string): string[] {
  return argsText.match(/(?:[^\s"]+|"[^"]*")+/g)?.map((arg) => arg.replace(/^"|"$/g, "")) ?? [];
}

export function parseLaunchEnv(envText: string): Record<string, string> {
  return Object.fromEntries(
    envText
      .split(/\s+/)
      .map((entry) => entry.trim())
      .filter(Boolean)
      .map((entry) => {
        const idx = entry.indexOf("=");
        return idx > 0 ? [entry.slice(0, idx), entry.slice(idx + 1)] : [entry, ""];
      }),
  );
}

export function buildDebugLaunchRequest(
  path: string,
  launchConfig: {
    args: string;
    env: string;
    cwd: string;
    tclshPath: string;
    backend?: "embedded" | "external";
    channelRouting?: LaunchChannelRouting;
  },
  baseOptions: DebugLaunchOptions = {},
): DebugLaunchRequest {
  return {
    path,
    args: parseLaunchArgs(launchConfig.args),
    tclshPath: launchConfig.tclshPath || undefined,
    channelRouting: launchConfig.channelRouting ? { ...launchConfig.channelRouting } : undefined,
    launchOptions: {
      ...baseOptions,
      cwd: launchConfig.cwd || null,
      env: parseLaunchEnv(launchConfig.env),
      // Only the non-default host transport is sent so embedded launches keep
      // their existing wire shape (the Rust side defaults to embedded).
      ...(launchConfig.backend === "external" ? { backend: "external" as const } : {}),
    },
  };
}

export function resolveDroppedFile(file: { name: string; path?: string }): DroppedFileResult {
  const supported = /\.(tbc|tcl|tk|itcl|tm)$/i.test(file.name);
  if (!supported) {
    return { supported: false, path: null, filename: file.name, reason: "unsupported" };
  }
  if (!file.path) {
    return { supported: true, path: null, filename: file.name, reason: "path-unavailable" };
  }
  return { supported: true, path: file.path, filename: file.name, reason: "ready" };
}

export function buildExportRequests(args: {
  data: TcldData;
  session: SessionState;
  output: OutputLine[];
  stoppedAt: DebugStoppedEvent | null;
}): Record<string, ExportRequest> {
  const { data, session, output, stoppedAt } = args;
  const base = data.metadata.filename;
  const comparison = buildComparisonModel(data, data.originalSourceText ?? null);
  const comparisonText = [
    `# ${comparison.original.label}`,
    comparison.original.text ?? "Original source unavailable.",
    "",
    `# ${comparison.reconstructed.label}`,
    comparison.reconstructed.text,
    "",
    "# Links",
    ...comparison.links.map((link) => `#${link.sourceMapIndex} bc ${link.bytecodeRange[0]}..${link.bytecodeRange[1]} original=${link.originalSpan ? `${link.originalSpan[0]}..${link.originalSpan[1]}` : "unavailable"} reconstructed=${link.reconstructedLines[0]}..${link.reconstructedLines[1]}`),
  ].join("\n");

  return {
    decompile: { name: `${base}.decompiled.tcl`, contents: data.decompileText },
    disassembly: { name: `${base}.disassembly.txt`, contents: data.dumpText },
    diagnostics: { name: `${base}.diagnostics.json`, contents: JSON.stringify(data.diagnostics ?? {}, null, 2) },
    provenance: { name: `${base}.provenance.json`, contents: JSON.stringify(buildProvenanceRows(data), null, 2) },
    comparison: { name: `${base}.comparison.txt`, contents: comparisonText },
    console: { name: "tcld-console.txt", contents: formatOutputForCopy(output) },
    fullReport: {
      name: `${base}.full-report.md`,
      contents: buildFullReport({ data, session, output, stoppedAt }),
    },
  };
}

export interface NativeMenuHandlers {
  openFile: () => void;
  closeWindow: () => void;
  resume: () => void;
  stop: () => void;
  stepInto: () => void;
  stepInstruction: () => void;
  stepOver: () => void;
  stepOut: () => void;
  toggleBreakpoint: () => void;
  restart: () => void;
  clearAllBreakpoints: () => void;
  launch: () => void;
  openPalette: () => void;
  addView: (viewId: ViewId) => void;
}

export interface NativeMenuState {
  isBusy: boolean;
  isExceptionStop: boolean;
  canStepInstruction: boolean;
}

export function dispatchNativeMenuAction(
  action: string,
  handlers: NativeMenuHandlers,
  state: NativeMenuState,
): boolean {
  const allowDebugAction = !state.isBusy;
  const allowResumeOrStep = allowDebugAction && !state.isExceptionStop;
  switch (action) {
    case "file_open":
      handlers.openFile();
      return true;
    case "file_close":
      handlers.closeWindow();
      return true;
    case "debug_continue":
      if (allowResumeOrStep) handlers.resume();
      return true;
    case "debug_stop":
      handlers.stop();
      return true;
    case "debug_step_into":
      if (allowResumeOrStep) handlers.stepInto();
      return true;
    case "debug_step_instruction":
      if (allowResumeOrStep && state.canStepInstruction) handlers.stepInstruction();
      return true;
    case "debug_step_over":
      if (allowResumeOrStep) handlers.stepOver();
      return true;
    case "debug_step_out":
      if (allowResumeOrStep) handlers.stepOut();
      return true;
    case "debug_toggle_breakpoint":
      if (allowDebugAction) handlers.toggleBreakpoint();
      return true;
    case "debug_restart":
      if (allowDebugAction) handlers.restart();
      return true;
    case "debug_clear_all_breakpoints":
      if (allowDebugAction) handlers.clearAllBreakpoints();
      return true;
    case "debug_launch":
      handlers.launch();
      return true;
    default:
      if (action.startsWith("view_")) {
        const viewId = action.replace("view_", "");
        if (viewId === "palette") {
          handlers.openPalette();
        } else {
          handlers.addView(viewId as ViewId);
        }
        return true;
      }
      return false;
  }
}

export function nativeMenuActionId(action: string): string | null {
  const direct = nativeMenuActionMap[action];
  if (direct) return direct;
  const recent = recentConfigurationNativeMenuActionId(action);
  if (recent) return recent;
  if (action.startsWith("view_")) {
    const viewId = action.replace("view_", "");
    return viewId === "commandResolver" ? "panel:command_resolver" : `panel:${viewId}`;
  }
  return null;
}

export function recentConfigurationNativeMenuActionId(action: string): string | null {
  return action.startsWith("run_recent_configuration:")
    ? action.replace("run_recent_configuration:", "run-configuration:recent:")
    : null;
}

export function nativeMenuAvailability(
  action: string,
  ctx: CommandContext,
  registry: { getForContext(ctx: CommandContext): { id: string }[] },
): boolean {
  const actionId = nativeMenuActionId(action);
  if (!actionId) return false;
  return registry.getForContext(ctx).some((command) => command.id === actionId);
}

export function buildNativeMenuAvailabilityPayload(
  itemIds: string[],
  ctx: CommandContext,
  registry: { getForContext(ctx: CommandContext): { id: string }[] },
): Record<string, boolean> {
  // Resolve the context once rather than once per menu item.
  const available = new Set(registry.getForContext(ctx).map((command) => command.id));
  return Object.fromEntries(itemIds.map((itemId) => {
    const actionId = nativeMenuActionId(itemId);
    return [itemId, actionId != null && available.has(actionId)];
  }));
}

const nativeMenuActionMap: Record<string, string> = {
  file_open: "open",
  file_copy_panel: "copy:panel",
  file_copy_diagnostics: "copy:diagnostics",
  file_copy_full_report: "copy:full_report",
  file_export_decompile: "export:decompile",
  file_export_disassembly: "export:disassembly",
  file_export_diagnostics: "export:diagnostics",
  file_export_provenance: "export:provenance",
  file_export_comparison: "export:comparison",
  file_export_console: "export:console",
  file_export_full_report: "export:full_report",
  file_close: "app:close-window",
  nav_next: "nav:next",
  nav_prev: "nav:prev",
  nav_top: "nav:top",
  nav_bottom: "nav:bottom",
  nav_next_group: "nav:next_group",
  nav_prev_group: "nav:prev_group",
  nav_first_bookmark: "nav:first_bookmark",
  nav_back: "nav:back",
  nav_forward: "nav:forward",
  nav_cycle_focus: "nav:cycle_focus",
  nav_cycle_focus_back: "nav:cycle_focus_back",
  layout_default: "layout:default",
  layout_analysis: "layout:analysis",
  layout_wide: "layout:wide",
  layout_decompile: "layout:decompile",
  layout_debug: "layout:debug",
  layout_overview: "layout:overview",
  debug_launch: "debug:launch",
  debug_launch_profile: "debug:launch-profile",
  run_configuration: "run-configuration:open-window",
  debug_continue: "debug:continue",
  debug_pause: "debug:pause",
  debug_stop: "debug:stop",
  debug_restart: "debug:restart",
  debug_step_into: "debug:step_into",
  debug_step_instruction: "debug:step_instruction",
  debug_step_next_group: "debug:step_next_group",
  debug_step_over: "debug:step_over",
  debug_step_out: "debug:step_out",
  debug_run_to_cursor: "debug:run_to_cursor",
  debug_toggle_breakpoint: "debug:toggle_breakpoint",
  debug_clear_all_breakpoints: "debug:clear_breakpoints",
  source_fold_all: "source:fold-all",
  source_unfold_all: "source:unfold-all",
  instructions_fold_all: "instructions:fold-all",
  instructions_unfold_all: "instructions:unfold-all",
  jumparrows: "jumparrows",
  graph_fit: "graph:fit",
  graph_center_selection: "graph:center-selection",
  graph_copy_details: "graph:copy-details",
  graph_expand_node: "graph:expand-node",
  graph_zoom_in: "graph:zoom-in",
  graph_zoom_out: "graph:zoom-out",
  graph_mode_basic_block: "graph:mode-basic-block",
  graph_mode_source_snippet: "graph:mode-source-snippet",
  format_preview: "format:preview",
  format_apply: "format:apply",
  appearance_open: "appearance:open",
  fontsize_up: "fontsize:up",
  fontsize_down: "fontsize:down",
  fontsize_reset: "fontsize:reset",
  keybindings_import: "keybindings:import",
  keybindings_export: "keybindings:export",
  keybindings_reset: "keybindings:reset",
  view_palette: "palette:open",
  help: "help",
};

export function legacyNativeMenuActionId(action: string): string | null {
  switch (action) {
    case "file_open":
      return "native-menu:file-open";
    case "file_close":
      return "native-menu:file-close-window";
    case "debug_launch":
      return "native-menu:debug-launch";
    case "debug_continue":
      return "native-menu:debug-continue";
    case "debug_stop":
      return "native-menu:debug-stop";
    case "debug_step_into":
      return "native-menu:debug-step-into";
    case "debug_step_instruction":
      return "native-menu:debug-step-instruction";
    case "debug_step_over":
      return "native-menu:debug-step-over";
    case "debug_step_out":
      return "native-menu:debug-step-out";
    case "debug_toggle_breakpoint":
      return "native-menu:debug-toggle-breakpoint";
    case "debug_restart":
      return "native-menu:debug-restart";
    case "debug_clear_all_breakpoints":
      return "native-menu:debug-clear-breakpoints";
    case "view_palette":
      return "native-menu:view-palette";
    default:
      if (!action.startsWith("view_")) return null;
      const viewId = action.replace("view_", "");
      return viewId === "commandResolver" ? "panel:command_resolver" : `panel:${viewId}`;
  }
}

export function executeNativeMenuAction(
  action: string,
  executeAction: (actionId: string) => boolean,
): boolean {
  const actionId = nativeMenuActionId(action);
  return actionId != null ? executeAction(actionId) : false;
}

export function summarizeFormatPreview(edits: FormatEdit[]): string {
  return `${edits.length} formatting edit${edits.length === 1 ? "" : "s"}`;
}
