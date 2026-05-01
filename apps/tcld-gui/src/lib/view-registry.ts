import type { ViewDef, ViewId } from "@/types";
import { GLYPH } from "./theme";

export type ViewIconRole =
  | "cpu"
  | "list"
  | "file"
  | "source-map"
  | "compare"
  | "alert"
  | "trace"
  | "project"
  | "inspector"
  | "exception"
  | "flow"
  | "summary"
  | "variables"
  | "stack"
  | "console"
  | "breakpoint"
  | "watchpoint"
  | "command";

export const VIEW_DEFINITIONS: ViewDef[] = [
  { id: "instructions", label: "Disassembly", icon: GLYPH.iconInstructions, iconRole: "cpu", key: "i" },
  { id: "literals", label: "Symbols", icon: GLYPH.iconLiterals, iconRole: "list", key: "l" },
  { id: "decompile", label: "Source View", icon: GLYPH.iconDecompile, iconRole: "file", key: "d" },
  { id: "source", label: "Source Map", icon: "src", iconRole: "source-map", key: "" },
  { id: "comparison", label: "Compare", icon: "cmp", iconRole: "compare", key: "" },
  { id: "diagnostics", label: "Issues", icon: "◇", iconRole: "alert", key: "" },
  { id: "provenance", label: "Provenance", icon: "⌁", iconRole: "trace", key: "" },
  { id: "project", label: "Project", icon: "proj", iconRole: "project", key: "" },
  { id: "detail", label: "Inspector", icon: "insp", iconRole: "inspector", key: "" },
  { id: "exceptions", label: "Exceptions", icon: "exc", iconRole: "exception", key: "e" },
  { id: "graph", label: "Control flow", icon: "flow", iconRole: "flow", key: "x" },
  { id: "summary", label: "Summary", icon: "sum", iconRole: "summary", key: "s" },
  { id: "variables", label: "Variables", icon: "var", iconRole: "variables", key: "" },
  { id: "stack", label: "Call Stack", icon: "stack", iconRole: "stack", key: "" },
  { id: "console", label: "Console", icon: "▤", iconRole: "console", key: "" },
  { id: "breakpoints", label: "Breakpoints", icon: "bp", iconRole: "breakpoint", key: "" },
  { id: "watchpoints", label: "Watchpoints", icon: "wp", iconRole: "watchpoint", key: "" },
  { id: "commandResolver", label: "Command Resolver", icon: "⌘", iconRole: "command", key: "" },
];

export const ALL_VIEW_IDS = VIEW_DEFINITIONS.map((view) => view.id);

export const VIEW_TITLES: Record<ViewId, string> = Object.fromEntries(
  VIEW_DEFINITIONS.map((view) => [view.id, view.label]),
) as Record<ViewId, string>;

export const VIEW_ICONS: Record<ViewId, string> = Object.fromEntries(
  VIEW_DEFINITIONS.map((view) => [view.id, view.icon]),
) as Record<ViewId, string>;

export const VIEW_ICON_ROLES: Record<ViewId, ViewIconRole> = Object.fromEntries(
  VIEW_DEFINITIONS.map((view) => [view.id, view.iconRole]),
) as Record<ViewId, ViewIconRole>;

export const VIEW_KEYS: Record<ViewId, string> = Object.fromEntries(
  VIEW_DEFINITIONS.map((view) => [view.id, view.key]),
) as Record<ViewId, string>;

export function isViewId(value: unknown): value is ViewId {
  return typeof value === "string" && ALL_VIEW_IDS.includes(value as ViewId);
}
