import type { CommandDef } from "@/lib/command-registry";

export interface ShortcutConflict {
  shortcut: string;
  actionIds: string[];
}

export interface ActionInventory {
  actions: CommandDef[];
  classes: Set<string>;
  paletteVisibleIds: string[];
  contexts: Map<string, string[]>;
  shortcutConflicts: ShortcutConflict[];
  nativeMenuCoverage: Map<string, NativeMenuCoverage>;
  nativeMenuMissingCoverageIds: string[];
  nativeMenuUnknownCommandIds: string[];
}

export type NativeMenuCoverage =
  | { status: "included"; itemId: string; path: string[] }
  | { status: "excluded"; reason: string }
  | { status: "missing" };

export interface NativeMenuCoverageItem {
  itemId: string;
  actionId: string;
  path: string[];
}

export interface NativeMenuCoverageContract {
  nativeMenuItems: NativeMenuCoverageItem[];
  nativeMenuExclusions?: Record<string, string>;
}

export function requiredActionClasses(): string[] {
  return [
    "app-shell",
    "toolbar",
    "native-menu",
    "panel",
    "context-menu",
    "navigation",
    "layout",
    "settings",
    "graph",
    "source",
    "disassembly",
    "decompile",
    "provenance",
    "diagnostics",
    "detail",
    "debugger",
    "breakpoint",
    "watch",
    "variable",
    "stack",
    "console",
    "file",
    "format",
    "mark",
    "pin",
    "launch-profile",
    "keybinding",
    "command-palette",
  ];
}

export function buildActionInventory(
  commands: CommandDef[],
  nativeMenuContract: NativeMenuCoverageContract = defaultNativeMenuCoverageContract(),
): ActionInventory {
  const byId = new Map<string, CommandDef>();
  for (const command of commands) {
    byId.set(command.id, command);
  }
  const actions = [...byId.values()].sort((a, b) => a.id.localeCompare(b.id));
  const shortcutBuckets = new Map<string, string[]>();
  const contexts = new Map<string, string[]>();
  const classes = new Set<string>();
  const paletteVisibleIds: string[] = [];

  for (const action of actions) {
    classes.add(classForAction(action));
    paletteVisibleIds.push(action.id);
    contexts.set(action.id, action.when ? action.when.split(",").map((part) => part.trim()) : []);
    for (const shortcut of action.shortcut ?? []) {
      const bucket = shortcutBuckets.get(shortcut) ?? [];
      bucket.push(action.id);
      shortcutBuckets.set(shortcut, bucket);
    }
  }

  const nativeMenuCoverage = new Map<string, NativeMenuCoverage>();
  const nativeMenuActionIds = new Set(nativeMenuContract.nativeMenuItems.map((item) => item.actionId));
  const nativeMenuUnknownCommandIds = nativeMenuContract.nativeMenuItems
    .map((item) => item.actionId)
    .filter((actionId) => !byId.has(actionId))
    .sort((a, b) => a.localeCompare(b));

  for (const action of actions) {
    const included = nativeMenuContract.nativeMenuItems.find((item) => item.actionId === action.id);
    if (included) {
      nativeMenuCoverage.set(action.id, {
        status: "included",
        itemId: included.itemId,
        path: [...included.path],
      });
      continue;
    }
    const reason = nativeMenuContract.nativeMenuExclusions?.[action.id] ?? defaultNativeMenuExclusionReason(action.id);
    if (reason) {
      nativeMenuCoverage.set(action.id, { status: "excluded", reason });
      continue;
    }
    nativeMenuCoverage.set(action.id, { status: "missing" });
  }

  const nativeMenuMissingCoverageIds = actions
    .filter((action) => !nativeMenuActionIds.has(action.id) && !nativeMenuContract.nativeMenuExclusions?.[action.id] && !defaultNativeMenuExclusionReason(action.id))
    .map((action) => action.id)
    .sort((a, b) => a.localeCompare(b));

  return {
    actions,
    classes,
    paletteVisibleIds,
    contexts,
    shortcutConflicts: [...shortcutBuckets.entries()]
      .filter(([, ids]) => ids.length > 1)
      .map(([shortcut, actionIds]) => ({ shortcut, actionIds })),
    nativeMenuCoverage,
    nativeMenuMissingCoverageIds,
    nativeMenuUnknownCommandIds,
  };
}

export function defaultNativeMenuCoverageContract(): NativeMenuCoverageContract {
  return {
    nativeMenuItems: [
      item("file_open", "open", ["File", "Open"]),
      item("file_close", "app:close-window", ["File", "Close Window"]),
      item("file_copy_panel", "copy:panel", ["File", "Copy Active Panel"]),
      item("file_copy_diagnostics", "copy:diagnostics", ["File", "Copy Diagnostics"]),
      item("file_copy_full_report", "copy:full_report", ["File", "Copy Full Report"]),
      item("file_export_decompile", "export:decompile", ["File", "Export Decompiled Tcl"]),
      item("file_export_disassembly", "export:disassembly", ["File", "Export Disassembly"]),
      item("file_export_diagnostics", "export:diagnostics", ["File", "Export Diagnostics"]),
      item("file_export_provenance", "export:provenance", ["File", "Export Provenance"]),
      item("file_export_comparison", "export:comparison", ["File", "Export Comparison"]),
      item("file_export_console", "export:console", ["File", "Export Console"]),
      item("file_export_full_report", "export:full_report", ["File", "Export Full Report"]),
      item("view_instructions", "panel:instructions", ["View", "Disassembly"]),
      item("view_decompile", "panel:decompile", ["View", "Decompile"]),
      item("view_source", "panel:source", ["View", "Source"]),
      item("view_comparison", "panel:comparison", ["View", "Compare"]),
      item("view_diagnostics", "panel:diagnostics", ["View", "Diagnostics"]),
      item("view_provenance", "panel:provenance", ["View", "Provenance"]),
      item("view_project", "panel:project", ["View", "Project"]),
      item("view_detail", "panel:detail", ["View", "Detail"]),
      item("view_literals", "panel:literals", ["View", "Literals"]),
      item("view_exceptions", "panel:exceptions", ["View", "Exceptions"]),
      item("view_summary", "panel:summary", ["View", "Summary"]),
      item("view_variables", "panel:variables", ["View", "Variables"]),
      item("view_stack", "panel:stack", ["View", "Call Stack"]),
      item("view_console", "panel:console", ["View", "Console"]),
      item("view_breakpoints", "panel:breakpoints", ["View", "Breakpoints"]),
      item("view_watchpoints", "panel:watchpoints", ["View", "Watchpoints"]),
      item("view_commandResolver", "panel:command_resolver", ["View", "Command Resolver"]),
      item("layout_default", "layout:default", ["View", "Default Layout"]),
      item("layout_analysis", "layout:analysis", ["View", "Analysis Layout"]),
      item("layout_wide", "layout:wide", ["View", "Wide Layout"]),
      item("layout_decompile", "layout:decompile", ["View", "Decompile Focus Layout"]),
      item("layout_debug", "layout:debug", ["View", "Debug Layout"]),
      item("layout_overview", "layout:overview", ["View", "Overview Layout"]),
      item("appearance_open", "appearance:open", ["View", "Preferences"]),
      item("fontsize_up", "fontsize:up", ["View", "Increase Font Size"]),
      item("fontsize_down", "fontsize:down", ["View", "Decrease Font Size"]),
      item("fontsize_reset", "fontsize:reset", ["View", "Reset Font Size"]),
      item("nav_next", "nav:next", ["Navigate", "Next Instruction"]),
      item("nav_prev", "nav:prev", ["Navigate", "Previous Instruction"]),
      item("nav_top", "nav:top", ["Navigate", "Go To Top"]),
      item("nav_bottom", "nav:bottom", ["Navigate", "Go To Bottom"]),
      item("nav_next_group", "nav:next_group", ["Navigate", "Next Group"]),
      item("nav_prev_group", "nav:prev_group", ["Navigate", "Previous Group"]),
      item("nav_first_bookmark", "nav:first_bookmark", ["Navigate", "First Bookmark"]),
      item("nav_back", "nav:back", ["Navigate", "Back"]),
      item("nav_forward", "nav:forward", ["Navigate", "Forward"]),
      item("nav_cycle_focus", "nav:cycle_focus", ["Navigate", "Cycle Focus"]),
      item("nav_cycle_focus_back", "nav:cycle_focus_back", ["Navigate", "Cycle Focus Backward"]),
      item("debug_launch", "debug:launch", ["Run", "Launch Script"]),
      item("debug_launch_profile", "debug:launch-profile", ["Run", "Run Selected Configuration"]),
      item("run_configuration", "run-configuration:open-window", ["Run", "Run Configuration"]),
      item("debug_continue", "debug:continue", ["Debug", "Continue"]),
      item("debug_pause", "debug:pause", ["Debug", "Pause"]),
      item("debug_stop", "debug:stop", ["Debug", "Stop"]),
      item("debug_restart", "debug:restart", ["Debug", "Restart"]),
      item("debug_step_into", "debug:step_into", ["Debug", "Step Into"]),
      item("debug_step_instruction", "debug:step_instruction", ["Debug", "Step Instruction"]),
      item("debug_step_over_instruction", "debug:step_over_instruction", ["Debug", "Step Over Instruction"]),
      item("debug_step_out_instruction", "debug:step_out_instruction", ["Debug", "Step Out Instruction"]),
      item("debug_run_to_pc", "debug:run_to_pc", ["Debug", "Run To PC"]),
      item("debug_step_next_group", "debug:step_next_group", ["Debug", "Step To Next Group"]),
      item("debug_step_over", "debug:step_over", ["Debug", "Step Over"]),
      item("debug_step_out", "debug:step_out", ["Debug", "Step Out"]),
      item("debug_run_to_cursor", "debug:run_to_cursor", ["Debug", "Run To Cursor"]),
      item("debug_toggle_breakpoint", "debug:toggle_breakpoint", ["Debug", "Toggle Breakpoint"]),
      item("debug_clear_all_breakpoints", "debug:clear_breakpoints", ["Debug", "Clear All Breakpoints"]),
      item("source_fold_all", "source:fold-all", ["Tools", "Fold Source"]),
      item("source_unfold_all", "source:unfold-all", ["Tools", "Unfold Source"]),
      item("instructions_fold_all", "instructions:fold-all", ["Tools", "Fold Disassembly"]),
      item("instructions_unfold_all", "instructions:unfold-all", ["Tools", "Unfold Disassembly"]),
      item("jumparrows", "jumparrows", ["Tools", "Toggle Jump Arrows"]),
      item("view_graph", "panel:graph", ["Tools", "Show Graph"]),
      item("graph_fit", "graph:fit", ["Tools", "Fit Graph"]),
      item("graph_center_selection", "graph:center-selection", ["Tools", "Center Selection"]),
      item("graph_copy_details", "graph:copy-details", ["Tools", "Copy Graph Details"]),
      item("graph_expand_node", "graph:expand-node", ["Tools", "Expand Graph Node"]),
      item("graph_zoom_in", "graph:zoom-in", ["Tools", "Zoom In"]),
      item("graph_zoom_out", "graph:zoom-out", ["Tools", "Zoom Out"]),
      item("graph_mode_basic_block", "graph:mode-basic-block", ["Tools", "Basic Block Graph Mode"]),
      item("graph_mode_source_snippet", "graph:mode-source-snippet", ["Tools", "Source Snippet Graph Mode"]),
      item("format_preview", "format:preview", ["Tools", "Preview Active Source"]),
      item("format_apply", "format:apply", ["Tools", "Apply Active Source"]),
      item("keybindings_import", "keybindings:import", ["Tools", "Import Keybindings"]),
      item("keybindings_export", "keybindings:export", ["Tools", "Export Keybindings"]),
      item("keybindings_reset", "keybindings:reset", ["Tools", "Reset Keybindings"]),
      item("view_palette", "palette:open", ["Help", "Command Palette"]),
      item("help", "help", ["Help", "Help"]),
    ],
    nativeMenuExclusions: {
      "toolbar:open-file": "toolbar-only duplicate of File > Open",
      "toolbar:debug-launch": "toolbar-only duplicate of Run > Launch Script",
      "toolbar:layout-default": "toolbar-only duplicate of Layout > Default",
      "toolbar:layout-debug": "toolbar-only duplicate of Layout > Debug",
      "toolbar:palette-open": "toolbar-only duplicate of Help > Command Palette",
      "context-menu:copy-active-panel": "context-menu-only duplicate of File > Copy Active Panel",
      "debug:launch-ad-hoc": "ad hoc launch is represented by Run > Launch Script and recent script entries",
      "debug:step_over_instruction": "opcode cursor controls remain toolbar/disassembly scoped",
      "debug:step_out_instruction": "opcode cursor controls remain toolbar/disassembly scoped",
      "debug:run_to_pc": "opcode cursor controls remain toolbar/disassembly scoped",
      "launch-profile:open-window": "legacy launch-profile command superseded by Run Configuration window",
      "launch-profile:create": "legacy launch-profile command superseded by Run Configuration save",
      "launch-profile:edit": "legacy launch-profile command superseded by Run Configuration window",
      "launch-profile:delete": "legacy launch-profile command superseded by Run Configuration delete",
      "run-configuration:save": "run-configuration window local action",
      "run-configuration:delete": "run-configuration window local action",
      "run-configuration:import": "run-configuration window local action",
      "run-configuration:export": "run-configuration window local action",
      "run-configuration:recent": "dynamic Run > Recent Configurations action prefix",
      "overlay:close": "overlay-local escape action",
      "workspace:restore": "workspace-local escape action",
      "workspace:tab-next": "in-panel tab cycling stays keyboard-scoped (Ctrl+Tab); leaves are not native-menu addressable",
      "workspace:tab-prev": "in-panel tab cycling stays keyboard-scoped (Ctrl+Shift+Tab); leaves are not native-menu addressable",
      "workspace:close-tab": "focused-tab close stays keyboard/panel-chrome scoped; the native menu owns window close only",
      "workspace:close-leaf": "focused-panel close stays keyboard/panel-chrome scoped; the native menu owns window close only",
      "workspace:split-right": "panel splitting stays keyboard/panel-header scoped; splits act on the focused leaf",
      "workspace:split-down": "panel splitting stays keyboard/panel-header scoped; splits act on the focused leaf",
      "layout:cycle-preset": "preset cycling shortcut; the named layout presets are individually listed under View",
      "settings:cycle-density": "density cycling shortcut; density is owned by Appearance settings",
      "panel:close": "window-management shortcut already covered by app chrome",
      "command:open": "command-line local action",
      "command:goto": "command-line local action",
      "command:quick_search": "command-line local action",
      trace: "debug/display toggle kept in toolbar/palette",
      maximize: "panel-local layout action",
      "mark:set:a": "mark commands remain keyboard/palette indexed",
      "pin:toggle_a": "pin commands remain keyboard/palette indexed",
      "internal:private": "internal helper command",
    },
  };
}

function item(itemId: string, actionId: string, path: string[]): NativeMenuCoverageItem {
  return { itemId, actionId, path };
}

function defaultNativeMenuExclusionReason(actionId: string): string | null {
  if (actionId.startsWith("mark:")) return "mark commands remain keyboard/palette indexed";
  if (actionId.startsWith("pin:")) return "pin commands remain keyboard/palette indexed";
  if (actionId.startsWith("theme:")) return "theme variants are represented by Appearance settings";
  if (actionId.startsWith("font:")) return "font variants are represented by Appearance settings";
  if (actionId.startsWith("density:")) return "density variants are represented by Appearance settings";
  if (actionId.startsWith("debug:hooks_")) return "bytecode hook details live in Run Configuration";
  if (
    actionId === "debug:step_over_instruction"
    || actionId === "debug:step_out_instruction"
    || actionId === "debug:run_to_pc"
  ) {
    return "opcode cursor controls remain toolbar/disassembly scoped";
  }
  if (actionId.startsWith("graph:detail-")) return "graph detail density remains panel-local";
  if (actionId === "decompile:copy") return "panel copy remains context-menu/palette scoped";
  if (actionId === "provenance:copy") return "panel copy remains context-menu/palette scoped";
  if (actionId === "diagnostics:copy") return "panel copy remains context-menu/palette scoped";
  if (actionId === "detail:copy") return "panel copy remains context-menu/palette scoped";
  if (actionId === "breakpoints:toggle") return "covered by Debug > Toggle Breakpoint";
  if (actionId.startsWith("breakpoints:")) return "breakpoint table commands remain panel-local";
  if (actionId.startsWith("watchpoints:")) return "watchpoint table commands remain panel-local";
  if (actionId.startsWith("variables:")) return "variable table commands remain panel-local";
  if (actionId.startsWith("stack:")) return "stack table commands remain panel-local";
  if (actionId.startsWith("console:")) return "console commands remain panel-local";
  if (actionId.startsWith("internal:")) return "internal helper command";
  return null;
}

function classForAction(action: Pick<CommandDef, "id" | "category">): string {
  const category = action.category.toLowerCase();
  const id = action.id;
  if (id.includes("launch-profile") || id.startsWith("launch-profile:")) return "launch-profile";
  if (id.startsWith("keybindings:")) return "keybinding";
  if (id.startsWith("app:")) return "app-shell";
  if (id.startsWith("toolbar:")) return "toolbar";
  if (id.startsWith("native-menu:")) return "native-menu";
  if (id.startsWith("context-menu:")) return "context-menu";
  if (id.startsWith("breakpoints:") || id.includes("breakpoint")) return "breakpoint";
  if (id.startsWith("watchpoints:") || id.includes("watchpoint")) return "watch";
  if (id.startsWith("variables:")) return "variable";
  if (id.startsWith("stack:")) return "stack";
  if (id.startsWith("console:")) return "console";
  if (id.startsWith("decompile:")) return "decompile";
  if (id.startsWith("diagnostics:")) return "diagnostics";
  if (id.startsWith("provenance:")) return "provenance";
  if (id.startsWith("detail:")) return "detail";
  if (id.startsWith("format:")) return "format";
  if (category.includes("debug")) return "debugger";
  if (category.includes("panel")) return "panel";
  if (category === "actions") return id.startsWith("format:") ? "format" : "toolbar";
  if (requiredActionClasses().includes(category)) return category;
  if (id.startsWith("graph:")) return "graph";
  if (id.startsWith("source:")) return "source";
  if (id.startsWith("instructions:") || id.startsWith("jumparrows")) return "disassembly";
  if (id.startsWith("palette:")) return "command-palette";
  if (id.startsWith("layout:")) return "layout";
  if (id.startsWith("theme:") || id.startsWith("font") || id.startsWith("density:")) return "settings";
  if (id.startsWith("mark:")) return "mark";
  if (id.startsWith("pin:")) return "pin";
  if (id.startsWith("nav:")) return "navigation";
  if (id.startsWith("export:") || id.startsWith("copy:") || id === "open") return "file";
  return category;
}
