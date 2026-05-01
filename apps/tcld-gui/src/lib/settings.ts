import {
  DESIGN_TYPE_SYSTEMS,
  THEMES,
  buildDesignDensityVars,
  normalizeThemeId,
} from "./themes";
import type { DensityMode, FontId, ThemeId, TypeSystemId } from "./themes";
import type { BytecodeHookMode, DebugLaunchOptions } from "./tauri-api";
import { isTauri, loadAppearanceSettings, saveAppearanceSettings } from "./tauri-api";

export type { DensityMode } from "./themes";

export type BytecodeHookModeSetting = BytecodeHookMode | "backend-default";
export type BytecodeHookCommandPreset = "backend-default" | "tbcload" | "runtime" | "all" | "custom";

export interface DebugBytecodeHookSettings {
  mode: BytecodeHookModeSetting;
  commandPreset: BytecodeHookCommandPreset;
  customCommands: string[];
}

export interface DebugLaunchSettings {
  bytecodeHooks: DebugBytecodeHookSettings;
}

export interface DebuggerSettings {
  launch: DebugLaunchSettings;
}

export interface PanelColumnSettings {
  [panelId: string]: {
    [columnId: string]: boolean;
  };
}

export const DISASSEMBLY_PARITY_COLUMN_IDS = ["group", "address", "opcode", "args", "comment"] as const;

export interface Settings {
  theme: ThemeId;
  typeSystem: TypeSystemId;
  fontFamily: FontId;
  fontSize: number;
  density: DensityMode;
  showSparklines: boolean;
  groupRails: boolean;
  panelColumns: PanelColumnSettings;
  debugger: DebuggerSettings;
}

export interface SerializedAppearanceSettings {
  theme: ThemeId;
  typeSystem: TypeSystemId;
  density: DensityMode;
  showSparklines: boolean;
  groupRails: boolean;
  fontSize: number;
}

export const settingsConfigStorageContract = {
  productionConfigPath: "tcld/appearance-settings.json",
  tauriCommands: {
    load: "load_appearance_settings",
    save: "save_appearance_settings",
  },
  tauriEvents: {
    changed: "appearance:changed",
  },
  browserFallbackStorageKey: "tcld-settings",
} as const;

const STORAGE_KEY = settingsConfigStorageContract.browserFallbackStorageKey;

const DEFAULT_DEBUG_HOOKS: DebugBytecodeHookSettings = {
  mode: "backend-default",
  commandPreset: "backend-default",
  customCommands: [],
};

const DEFAULT_DEBUGGER_SETTINGS: DebuggerSettings = {
  launch: {
    bytecodeHooks: DEFAULT_DEBUG_HOOKS,
  },
};

export const DEFAULT_SETTINGS: Settings = {
  theme: "graphite",
  typeSystem: "mono-brutalist",
  fontFamily: "mono-brutalist",
  fontSize: 13,
  density: "regular",
  showSparklines: true,
  groupRails: true,
  panelColumns: {},
  debugger: DEFAULT_DEBUGGER_SETTINGS,
};

const MIN_FONT_SIZE = 10;
const MAX_FONT_SIZE = 18;

export function loadSettings(): Settings {
  if (typeof window === "undefined") return DEFAULT_SETTINGS;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return DEFAULT_SETTINGS;
    return parseSettings(JSON.parse(raw));
  } catch {
    return DEFAULT_SETTINGS;
  }
}

export async function loadSettingsWithTauriFallback(): Promise<Settings> {
  if (isTauri()) {
    try {
      const stored = await loadAppearanceSettings();
      if (stored == null) return loadSettings();
      return mergeAppearanceSettings(loadSettings(), stored);
    } catch {
      return loadSettings();
    }
  }
  return loadSettings();
}

export function saveSettings(settings: Settings): void {
  if (typeof window === "undefined") return;
  localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
}

export async function saveSettingsWithTauriFallback(settings: Settings): Promise<void> {
  saveSettings(settings);
  if (!isTauri()) return;
  await saveAppearanceSettings(serializeSettingsForTauri(settings));
}

export function serializeSettingsForTauri(settings: Settings): SerializedAppearanceSettings {
  return {
    theme: settings.theme,
    typeSystem: settings.typeSystem,
    density: settings.density,
    showSparklines: settings.showSparklines,
    groupRails: settings.groupRails,
    fontSize: settings.fontSize,
  };
}

export function applySettings(settings: Settings, root?: HTMLElement): void {
  const target = root ?? (typeof document === "undefined" ? undefined : document.documentElement);
  if (!target) return;

  const themeId = normalizeThemeId(settings.theme);
  const theme = THEMES[themeId];
  for (const [prop, value] of Object.entries(theme.colors)) {
    target.style.setProperty(prop, value);
  }

  const typeSystem = DESIGN_TYPE_SYSTEMS[settings.typeSystem] ?? DESIGN_TYPE_SYSTEMS["mono-brutalist"];
  target.style.setProperty("--ui", typeSystem.ui);
  target.style.setProperty("--mono", typeSystem.mono);
  target.style.setProperty("--font-mono", typeSystem.mono);
  target.style.setProperty("--font-ui", typeSystem.ui);
  target.style.setProperty("--type-tracking", typeSystem.tracking);
  target.style.setProperty("--type-weight-bias", typeSystem.weight);
  target.style.setProperty("--font-size-base", `${settings.fontSize}px`);
  target.style.setProperty("--type-ui", `${13 * typeSystem.scale}px`);
  target.style.setProperty("--type-label", "10.5px");
  target.style.setProperty("--type-status", "11px");
  target.style.setProperty("--type-code", "12px");
  target.style.setProperty("--type-disassembly", "11.5px");
  target.style.setProperty("--rail-display", settings.groupRails ? "block" : "none");
  target.style.setProperty("--variable-sparklines", settings.showSparklines ? "block" : "none");

  for (const [prop, value] of Object.entries(buildDesignDensityVars(settings.density))) {
    target.style.setProperty(prop, value);
  }
  target.dataset.density = settings.density;
  target.dataset.typeSystem = settings.typeSystem;
  target.dataset.theme = themeId;
}

export function clampFontSize(size: number): number {
  return Math.max(MIN_FONT_SIZE, Math.min(MAX_FONT_SIZE, Math.round(size)));
}

export function changeFontSize(current: Settings, delta: number): Settings {
  return { ...current, fontSize: clampFontSize(current.fontSize + delta) };
}

export function resetFontSize(current: Settings): Settings {
  return { ...current, fontSize: DEFAULT_SETTINGS.fontSize };
}

export function setDensity(current: Settings, density: DensityMode): Settings {
  return { ...current, density };
}

export function getPanelColumns(
  panelId: string,
  defaults: Record<string, boolean>,
): Record<string, boolean> {
  const settings = loadSettings();
  const stored = settings.panelColumns?.[panelId] ?? {};
  return { ...defaults, ...stored };
}

export function withDisassemblyParityColumns(columns: Record<string, boolean>): Record<string, boolean> {
  const next = { ...columns };
  for (const columnId of DISASSEMBLY_PARITY_COLUMN_IDS) {
    next[columnId] = true;
  }
  return next;
}

export function savePanelColumns(
  panelId: string,
  nextColumns: Record<string, boolean>,
): void {
  const settings = loadSettings();
  saveSettings({
    ...settings,
    panelColumns: {
      ...settings.panelColumns,
      [panelId]: { ...nextColumns },
    },
  });
}

export function setDebugHookMode(current: Settings, mode: BytecodeHookModeSetting): Settings {
  return {
    ...current,
    debugger: {
      ...current.debugger,
      launch: {
        ...current.debugger.launch,
        bytecodeHooks: {
          ...current.debugger.launch.bytecodeHooks,
          mode,
        },
      },
    },
  };
}

export function setDebugHookCommandPreset(current: Settings, commandPreset: BytecodeHookCommandPreset): Settings {
  return {
    ...current,
    debugger: {
      ...current.debugger,
      launch: {
        ...current.debugger.launch,
        bytecodeHooks: {
          ...current.debugger.launch.bytecodeHooks,
          commandPreset,
        },
      },
    },
  };
}

export function setDebugHookCustomCommands(current: Settings, customCommands: string[]): Settings {
  return {
    ...current,
    debugger: {
      ...current.debugger,
      launch: {
        ...current.debugger.launch,
        bytecodeHooks: {
          ...current.debugger.launch.bytecodeHooks,
          customCommands: normalizeCommandList(customCommands),
        },
      },
    },
  };
}

export function getDebugLaunchOptions(settings: Settings): DebugLaunchOptions | undefined {
  const hooks = settings.debugger.launch.bytecodeHooks;
  const launchOptions: DebugLaunchOptions = {};

  if (hooks.mode !== "backend-default") {
    launchOptions.bytecodeHookMode = hooks.mode;
  }

  const commands = resolveHookCommands(hooks);
  if (commands !== undefined) {
    launchOptions.bytecodeHookCommands = commands;
  }

  if (!launchOptions.bytecodeHookMode && !launchOptions.bytecodeHookCommands) {
    return undefined;
  }
  return launchOptions;
}

export function describeDebugHookSettings(settings: Settings): string {
  const hooks = settings.debugger.launch.bytecodeHooks;
  const modeLabel = hooks.mode === "backend-default" ? "default mode" : hooks.mode;
  const presetLabel = hooks.commandPreset === "backend-default" ? "default commands" : `${hooks.commandPreset} commands`;
  return `Debug hooks: ${modeLabel} · ${presetLabel}`;
}

function parseSettings(raw: unknown): Settings {
  const parsed = parseObject(raw);
  const appearance = parseAppearanceSettings(parsed);
  const debugHooks = parseDebugBytecodeHookSettings(parsed.debugger);
  const panelColumns = parsePanelColumns(parsed.panelColumns);
  return {
    ...DEFAULT_SETTINGS,
    ...appearance,
    panelColumns,
    debugger: {
      launch: {
        bytecodeHooks: debugHooks,
      },
    },
  };
}

function parseAppearanceSettings(raw: unknown): SerializedAppearanceSettings {
  const parsed = parseObject(raw);
  const theme = normalizeThemeId(asString(parsed.theme, DEFAULT_SETTINGS.theme));
  const typeSystem = isTypeSystemId(parsed.typeSystem)
    ? parsed.typeSystem
    : (isTypeSystemId(parsed.fontFamily) ? parsed.fontFamily : DEFAULT_SETTINGS.typeSystem);
  return {
    theme,
    typeSystem,
    density: parseDensityMode(parsed.density),
    showSparklines: asBoolean(parsed.showSparklines, DEFAULT_SETTINGS.showSparklines),
    groupRails: asBoolean(parsed.groupRails, DEFAULT_SETTINGS.groupRails),
    fontSize: clampFontSize(asNumber(parsed.fontSize, DEFAULT_SETTINGS.fontSize)),
  };
}

export function mergeAppearanceSettings(base: Settings, appearance: unknown): Settings {
  const parsed = parseAppearanceSettings(appearance);
  return {
    ...base,
    ...parsed,
    fontFamily: parsed.typeSystem,
  };
}

function resolveHookCommands(hooks: DebugBytecodeHookSettings): string[] | undefined {
  switch (hooks.commandPreset) {
    case "backend-default":
      return undefined;
    case "tbcload":
      return ["::tbcload::bceval", "::tbcload::bcproc"];
    case "runtime":
      return ["::tbceval", "::tbcproc"];
    case "all":
      return ["::tbcload::bceval", "::tbcload::bcproc", "::tbceval", "::tbcproc"];
    case "custom": {
      const commands = normalizeCommandList(hooks.customCommands);
      return commands.length > 0 ? commands : undefined;
    }
    default:
      return undefined;
  }
}

function parseDebugBytecodeHookSettings(rawDebugger: unknown): DebugBytecodeHookSettings {
  const debuggerObj = parseObject(rawDebugger);
  const launchObj = parseObject(debuggerObj.launch);
  const hookObj = parseObject(launchObj.bytecodeHooks);

  const mode = isHookModeSetting(hookObj.mode) ? hookObj.mode : DEFAULT_DEBUG_HOOKS.mode;
  const commandPreset = isHookPreset(hookObj.commandPreset) ? hookObj.commandPreset : DEFAULT_DEBUG_HOOKS.commandPreset;
  const customCommands = normalizeCommandList(asStringArray(hookObj.customCommands));

  return {
    mode,
    commandPreset,
    customCommands,
  };
}

function normalizeCommandList(input: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of input) {
    const trimmed = raw.trim();
    if (!trimmed) continue;
    const normalized = trimmed.startsWith("::") ? trimmed : `::${trimmed}`;
    if (seen.has(normalized)) continue;
    seen.add(normalized);
    out.push(normalized);
  }
  return out;
}

function parsePanelColumns(value: unknown): PanelColumnSettings {
  const obj = parseObject(value);
  const out: PanelColumnSettings = {};
  for (const [panelId, rawColumns] of Object.entries(obj)) {
    const columnsObj = parseObject(rawColumns);
    const columns: Record<string, boolean> = {};
    for (const [columnId, rawEnabled] of Object.entries(columnsObj)) {
      if (typeof rawEnabled === "boolean") {
        columns[columnId] = rawEnabled;
      }
    }
    out[panelId] = columns;
  }
  return out;
}

function isTypeSystemId(value: unknown): value is TypeSystemId {
  return typeof value === "string" && value in DESIGN_TYPE_SYSTEMS;
}

function isHookModeSetting(value: unknown): value is BytecodeHookModeSetting {
  return value === "backend-default" || value === "passthrough" || value === "intercept";
}

function parseDensityMode(value: unknown): DensityMode {
  if (value === "compact" || value === "regular" || value === "comfy") return value;
  if (value === "normal") return "regular";
  if (value === "ultra") return "compact";
  return DEFAULT_SETTINGS.density;
}

function isHookPreset(value: unknown): value is BytecodeHookCommandPreset {
  return (
    value === "backend-default"
    || value === "tbcload"
    || value === "runtime"
    || value === "all"
    || value === "custom"
  );
}

function parseObject(value: unknown): Record<string, unknown> {
  return value != null && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function asNumber(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function asString(value: unknown, fallback: string): string {
  return typeof value === "string" ? value : fallback;
}

function asBoolean(value: unknown, fallback: boolean): boolean {
  return typeof value === "boolean" ? value : fallback;
}

function asStringArray(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}
