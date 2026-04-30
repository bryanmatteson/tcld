import { createStore } from "zustand/vanilla";
import type { DebugBytecodeHookSettings, Settings } from "@/lib/settings";
import type { DebugLaunchRequest, LaunchChannelRouting, LaunchChannelRoute, LaunchExitBehavior } from "@/lib/gui-workflows";
import { buildDebugLaunchRequest } from "@/lib/gui-workflows";
import type { RecentFile } from "@/lib/recent-files";

/** Debugger backend for a launch: `embedded` (default, in-process) or
 * `external` (tclsh nub over the wire protocol). Optional on stored
 * shapes because pre-existing persisted profiles omit it; normalization
 * always fills `embedded`. */
export type RunConfigurationBackend = "embedded" | "external";

export interface RunConfigurationProfile {
  id: string;
  name: string;
  path: string;
  args: string;
  env: string;
  cwd: string;
  tclshPath: string;
  backend?: RunConfigurationBackend;
  bytecodeHooks: DebugBytecodeHookSettings;
  channelRouting: LaunchChannelRouting;
}

export interface AdHocRunConfiguration {
  args: string;
  env: string;
  cwd: string;
  tclshPath: string;
  backend?: RunConfigurationBackend;
  bytecodeHooks: DebugBytecodeHookSettings;
  channelRouting: LaunchChannelRouting;
}

export interface RunConfigurationState {
  profiles: RunConfigurationProfile[];
  selectedProfileId: string | null;
  adHoc: AdHocRunConfiguration;
  draft: RunConfigurationProfile | null;
}

export type RunConfigurationAction =
  | { type: "hydrate"; state: RunConfigurationState }
  | { type: "select-profile"; id: string | null }
  | { type: "save-profile"; profile: RunConfigurationProfile }
  | { type: "delete-profile"; id: string }
  | { type: "start-draft"; profile: RunConfigurationProfile | null }
  | { type: "update-draft"; patch: Partial<RunConfigurationProfile> }
  | { type: "update-ad-hoc"; patch: Partial<AdHocRunConfiguration> }
  | { type: "cancel-draft" };

export interface RunConfigurationDiagnostic {
  path: string;
  message: string;
}

export interface RunConfigurationImportResult {
  ok: boolean;
  diagnostics: RunConfigurationDiagnostic[];
  state: RunConfigurationState;
}

export interface RunConfigurationStore {
  dispatch(action: RunConfigurationAction): void;
  snapshot(): RunConfigurationState;
  subscribe(listener: () => void): () => void;
  exportJson(): string;
  importJson(text: string): RunConfigurationImportResult;
}

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export interface RecentRunConfiguration {
  id: string;
  label: string;
  actionId: string;
  itemId: string;
  kind: "profile" | "script";
  profileId?: string;
  path?: string;
}

export interface RecentRunConfigurationMenuItem {
  itemId: string;
  actionId: string | null;
  label: string;
  enabled: boolean;
}

const STORAGE_KEY = "tcld-launch-profiles";
export const RECENT_RUN_CONFIGURATION_MENU_SLOTS = 10;

export const DEFAULT_CHANNEL_ROUTING: LaunchChannelRouting = {
  stdout: "console",
  stderr: "console",
  eval: "console",
  system: "console",
  forwardSignals: true,
  onExit: "keep-open",
};

export function initialRunConfigurationState(settings: Settings): RunConfigurationState {
  return {
    profiles: [],
    selectedProfileId: null,
    adHoc: {
      args: "",
      env: "",
      cwd: "",
      tclshPath: "",
      backend: "embedded",
      bytecodeHooks: settings.debugger.launch.bytecodeHooks,
      channelRouting: DEFAULT_CHANNEL_ROUTING,
    },
    draft: null,
  };
}

export function createRunConfigurationStore(args: { storage: StorageLike; settings: Settings }): RunConfigurationStore {
  const store = createStore<RunConfigurationState>(() => loadState(args.storage, args.settings));

  function persist() {
    args.storage.setItem(STORAGE_KEY, JSON.stringify(persistedState(store.getState())));
  }

  return {
    dispatch(action: RunConfigurationAction) {
      store.setState((current) => reduceRunConfigurationState(current, action, args.settings), true);
      persist();
    },
    snapshot() {
      return cloneState(store.getState());
    },
    subscribe(listener: () => void) {
      return store.subscribe(listener);
    },
    exportJson() {
      return exportRunConfigurationsJson(store.getState());
    },
    importJson(text: string) {
      const result = importRunConfigurationsJson(text, store.getState(), args.settings);
      if (result.ok) {
        store.setState(result.state, true);
        persist();
      }
      return result;
    },
  };
}

export function reduceRunConfigurationState(
  state: RunConfigurationState,
  action: RunConfigurationAction,
  settings: Settings,
): RunConfigurationState {
  switch (action.type) {
    case "hydrate":
      return normalizeState(action.state, settings);
    case "select-profile":
      return {
        ...state,
        selectedProfileId: action.id && state.profiles.some((profile) => profile.id === action.id) ? action.id : null,
      };
    case "save-profile": {
      const profile = normalizeProfile(action.profile, settings);
      const profiles = [
        ...state.profiles.filter((candidate) => candidate.id !== profile.id),
        profile,
      ].sort(compareProfiles);
      return {
        ...state,
        profiles,
        selectedProfileId: profile.id,
        draft: profile,
      };
    }
    case "delete-profile": {
      const profiles = state.profiles.filter((profile) => profile.id !== action.id);
      return {
        ...state,
        profiles,
        selectedProfileId: state.selectedProfileId === action.id ? null : state.selectedProfileId,
        draft: state.draft?.id === action.id ? null : state.draft,
      };
    }
    case "start-draft":
      return {
        ...state,
        draft: action.profile ? normalizeProfile(action.profile, settings) : newRunConfigurationProfile(settings),
      };
    case "update-draft":
      return {
        ...state,
        draft: normalizeProfile({ ...(state.draft ?? newRunConfigurationProfile(settings)), ...action.patch }, settings),
      };
    case "update-ad-hoc":
      return {
        ...state,
        adHoc: normalizeAdHoc({ ...state.adHoc, ...action.patch }, settings),
      };
    case "cancel-draft":
      return {
        ...state,
        draft: null,
      };
  }
}

export function runConfigurationActionIds(): string[] {
  return [
    "debug:launch-ad-hoc",
    "debug:launch-profile",
    "run-configuration:open-window",
    "run-configuration:save",
    "run-configuration:delete",
    "run-configuration:import",
    "run-configuration:export",
    "run-configuration:recent",
  ];
}

export function buildRunConfigurationLaunchRequest(
  state: RunConfigurationState,
  settings: Settings,
  recent?: RecentRunConfiguration,
): DebugLaunchRequest | null {
  if (recent?.kind === "script" && recent.path) {
    return buildDebugLaunchRequest(recent.path, state.adHoc, resolveDebugHookOptions(state.adHoc.bytecodeHooks));
  }
  const profileId = recent?.profileId ?? state.selectedProfileId;
  const profile = state.profiles.find((candidate) => candidate.id === profileId);
  if (!profile) return null;
  return buildDebugLaunchRequest(profile.path, profile, resolveDebugHookOptions(profile.bytecodeHooks ?? settings.debugger.launch.bytecodeHooks));
}

export function buildRecentRunConfigurations(
  state: RunConfigurationState,
  recentFiles: RecentFile[],
): RecentRunConfiguration[] {
  const selected = state.profiles.find((profile) => profile.id === state.selectedProfileId);
  const profiles = [
    ...(selected ? [selected] : []),
    ...state.profiles.filter((profile) => profile.id !== selected?.id),
  ];
  const profileItems = profiles.map((profile) => ({
    id: `profile:${profile.id}`,
    label: profile.name,
    actionId: `run-configuration:recent:profile:${stripRecentId(profile.id)}`,
    itemId: `run_recent_configuration:profile:${stripRecentId(profile.id)}`,
    kind: "profile" as const,
    profileId: profile.id,
    path: profile.path,
  }));
  const scriptItems = recentFiles
    .filter((entry) => entry.mode === "debug")
    .map((entry) => ({
      id: `script:${entry.path}`,
      label: entry.filename,
      actionId: `run-configuration:recent:script:${entry.path}`,
      itemId: `run_recent_configuration:script:${entry.path}`,
      kind: "script" as const,
      path: entry.path,
    }));
  return [...profileItems, ...scriptItems].slice(0, 10);
}

export function buildRecentRunConfigurationMenuItems(
  recent: RecentRunConfiguration[],
  slots = RECENT_RUN_CONFIGURATION_MENU_SLOTS,
): RecentRunConfigurationMenuItem[] {
  return Array.from({ length: slots }, (_, index) => {
    const item = recent[index];
    return {
      itemId: `run_recent_configuration:${index}`,
      actionId: item?.actionId ?? null,
      label: item?.label ?? `Recent Configuration ${index + 1}`,
      enabled: item != null,
    };
  });
}

export function exportRunConfigurationsJson(state: RunConfigurationState): string {
  const out = persistedState(normalizeState(state, {
    debugger: { launch: { bytecodeHooks: state.adHoc.bytecodeHooks } },
  } as Settings));
  return `${JSON.stringify(out, null, 2)}\n`;
}

export function importRunConfigurationsJson(
  text: string,
  previousState: RunConfigurationState,
  settings: Settings,
): RunConfigurationImportResult {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return {
      ok: false,
      diagnostics: [{ path: "$", message: "Malformed run configuration JSON." }],
      state: previousState,
    };
  }

  const diagnostics: RunConfigurationDiagnostic[] = [];
  const next = parseState(parsed, settings, diagnostics);
  if (diagnostics.length > 0 || !next) {
    return { ok: false, diagnostics, state: previousState };
  }
  return { ok: true, diagnostics: [], state: next };
}

export function newRunConfigurationProfile(settings: Settings): RunConfigurationProfile {
  return {
    id: `profile:${randomId()}`,
    name: "New Configuration",
    path: "",
    args: "",
    env: "",
    cwd: "",
    tclshPath: "",
    backend: "embedded",
    bytecodeHooks: settings.debugger.launch.bytecodeHooks,
    channelRouting: DEFAULT_CHANNEL_ROUTING,
  };
}

function loadState(storage: StorageLike, settings: Settings): RunConfigurationState {
  const raw = storage.getItem(STORAGE_KEY);
  if (!raw) return initialRunConfigurationState(settings);
  try {
    const diagnostics: RunConfigurationDiagnostic[] = [];
    return parseState(JSON.parse(raw), settings, diagnostics) ?? initialRunConfigurationState(settings);
  } catch {
    return initialRunConfigurationState(settings);
  }
}

function parseState(
  parsed: unknown,
  settings: Settings,
  diagnostics: RunConfigurationDiagnostic[],
): RunConfigurationState | null {
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    diagnostics.push({ path: "$", message: "Run configuration JSON must be an object." });
    return null;
  }
  const record = parsed as Record<string, unknown>;
  const rawProfiles = record.profiles;
  if (rawProfiles != null && !Array.isArray(rawProfiles)) {
    diagnostics.push({ path: "$.profiles", message: "Profiles must be an array." });
    return null;
  }

  const profiles: RunConfigurationProfile[] = [];
  for (const [index, rawProfile] of (rawProfiles ?? []).entries()) {
    const profile = parseProfile(rawProfile, settings, `$.profiles[${index}]`, diagnostics);
    if (profile) profiles.push(profile);
  }

  const adHoc = parseAdHoc(record.adHoc, settings, diagnostics);
  const selectedProfileId = typeof record.selectedProfileId === "string"
    && profiles.some((profile) => profile.id === record.selectedProfileId)
    ? record.selectedProfileId
    : null;

  if (diagnostics.length > 0) return null;
  return normalizeState({
    profiles,
    selectedProfileId,
    adHoc,
    draft: null,
  }, settings);
}

function parseProfile(
  raw: unknown,
  settings: Settings,
  path: string,
  diagnostics: RunConfigurationDiagnostic[],
): RunConfigurationProfile | null {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    diagnostics.push({ path, message: "Profile must be an object." });
    return null;
  }
  const record = raw as Record<string, unknown>;
  for (const field of ["id", "name", "path", "args", "env", "cwd", "tclshPath"]) {
    if (record[field] != null && typeof record[field] !== "string") {
      diagnostics.push({ path: `${path}.${field}`, message: `Profile field '${field}' must be a string.` });
    }
  }
  if (typeof record.id !== "string" || typeof record.name !== "string") {
    diagnostics.push({ path, message: "Profile requires string id and name." });
    return null;
  }
  return normalizeProfile({
    id: record.id,
    name: record.name,
    path: typeof record.path === "string" ? record.path : "",
    args: typeof record.args === "string" ? record.args : "",
    env: typeof record.env === "string" ? record.env : "",
    cwd: typeof record.cwd === "string" ? record.cwd : "",
    tclshPath: typeof record.tclshPath === "string" ? record.tclshPath : "",
    backend: normalizeBackend(record.backend),
    bytecodeHooks: parseHooks(record.bytecodeHooks, settings.debugger.launch.bytecodeHooks),
    channelRouting: parseChannelRouting(record.channelRouting),
  }, settings);
}

function parseAdHoc(
  raw: unknown,
  settings: Settings,
  diagnostics: RunConfigurationDiagnostic[],
): AdHocRunConfiguration {
  if (raw == null) return initialRunConfigurationState(settings).adHoc;
  if (typeof raw !== "object" || Array.isArray(raw)) {
    diagnostics.push({ path: "$.adHoc", message: "Ad hoc configuration must be an object." });
    return initialRunConfigurationState(settings).adHoc;
  }
  const record = raw as Record<string, unknown>;
  return normalizeAdHoc({
    args: typeof record.args === "string" ? record.args : "",
    env: typeof record.env === "string" ? record.env : "",
    cwd: typeof record.cwd === "string" ? record.cwd : "",
    tclshPath: typeof record.tclshPath === "string" ? record.tclshPath : "",
    backend: normalizeBackend(record.backend),
    bytecodeHooks: parseHooks(record.bytecodeHooks, settings.debugger.launch.bytecodeHooks),
    channelRouting: parseChannelRouting(record.channelRouting),
  }, settings);
}

function normalizeState(state: RunConfigurationState, settings: Settings): RunConfigurationState {
  const profiles = state.profiles.map((profile) => normalizeProfile(profile, settings)).sort(compareProfiles);
  return {
    profiles,
    selectedProfileId: state.selectedProfileId && profiles.some((profile) => profile.id === state.selectedProfileId)
      ? state.selectedProfileId
      : null,
    adHoc: normalizeAdHoc(state.adHoc, settings),
    draft: state.draft ? normalizeProfile(state.draft, settings) : null,
  };
}

function normalizeProfile(profile: RunConfigurationProfile, settings: Settings): RunConfigurationProfile {
  return {
    id: profile.id || `profile:${randomId()}`,
    name: profile.name || "Untitled Configuration",
    path: profile.path ?? "",
    args: profile.args ?? "",
    env: profile.env ?? "",
    cwd: profile.cwd ?? "",
    tclshPath: profile.tclshPath ?? "",
    backend: normalizeBackend(profile.backend),
    bytecodeHooks: profile.bytecodeHooks ?? settings.debugger.launch.bytecodeHooks,
    channelRouting: normalizeChannelRouting(profile.channelRouting),
  };
}

function normalizeAdHoc(adHoc: AdHocRunConfiguration, settings: Settings): AdHocRunConfiguration {
  return {
    args: adHoc.args ?? "",
    env: adHoc.env ?? "",
    cwd: adHoc.cwd ?? "",
    tclshPath: adHoc.tclshPath ?? "",
    backend: normalizeBackend(adHoc.backend),
    bytecodeHooks: adHoc.bytecodeHooks ?? settings.debugger.launch.bytecodeHooks,
    channelRouting: normalizeChannelRouting(adHoc.channelRouting),
  };
}

function normalizeBackend(value: unknown): RunConfigurationBackend {
  return value === "external" ? "external" : "embedded";
}

function persistedState(state: RunConfigurationState) {
  return {
    version: 1,
    selectedProfileId: state.selectedProfileId,
    profiles: state.profiles.map((profile) => ({
      id: profile.id,
      name: profile.name,
      path: profile.path,
      args: profile.args,
      env: profile.env,
      cwd: profile.cwd,
      tclshPath: profile.tclshPath,
      backend: normalizeBackend(profile.backend),
      bytecodeHooks: profile.bytecodeHooks,
      channelRouting: profile.channelRouting,
    })),
    adHoc: state.adHoc,
  };
}

function cloneState(state: RunConfigurationState): RunConfigurationState {
  return {
    profiles: state.profiles.map((profile) => ({
      ...profile,
      bytecodeHooks: cloneHooks(profile.bytecodeHooks),
      channelRouting: cloneChannelRouting(profile.channelRouting),
    })),
    selectedProfileId: state.selectedProfileId,
    adHoc: {
      ...state.adHoc,
      bytecodeHooks: cloneHooks(state.adHoc.bytecodeHooks),
      channelRouting: cloneChannelRouting(state.adHoc.channelRouting),
    },
    draft: state.draft
      ? {
        ...state.draft,
        bytecodeHooks: cloneHooks(state.draft.bytecodeHooks),
        channelRouting: cloneChannelRouting(state.draft.channelRouting),
      }
      : null,
  };
}

function parseChannelRouting(raw: unknown): LaunchChannelRouting {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    return cloneChannelRouting(DEFAULT_CHANNEL_ROUTING);
  }
  const record = raw as Partial<Record<keyof LaunchChannelRouting, unknown>>;
  return normalizeChannelRouting({
    stdout: normalizeRoute(record.stdout),
    stderr: normalizeRoute(record.stderr),
    eval: normalizeRoute(record.eval),
    system: normalizeRoute(record.system),
    forwardSignals: typeof record.forwardSignals === "boolean" ? record.forwardSignals : DEFAULT_CHANNEL_ROUTING.forwardSignals,
    onExit: normalizeExit(record.onExit),
  });
}

function normalizeChannelRouting(routing: LaunchChannelRouting | undefined): LaunchChannelRouting {
  return {
    stdout: normalizeRoute(routing?.stdout),
    stderr: normalizeRoute(routing?.stderr),
    eval: normalizeRoute(routing?.eval),
    system: normalizeRoute(routing?.system),
    forwardSignals: typeof routing?.forwardSignals === "boolean" ? routing.forwardSignals : DEFAULT_CHANNEL_ROUTING.forwardSignals,
    onExit: normalizeExit(routing?.onExit),
  };
}

function cloneChannelRouting(routing: LaunchChannelRouting): LaunchChannelRouting {
  return { ...normalizeChannelRouting(routing) };
}

function normalizeRoute(value: unknown): LaunchChannelRoute {
  return value === "discard" || value === "console" ? value : "console";
}

function normalizeExit(value: unknown): LaunchExitBehavior {
  return value === "close" || value === "restart" || value === "keep-open" ? value : "keep-open";
}

function parseHooks(raw: unknown, fallback: DebugBytecodeHookSettings): DebugBytecodeHookSettings {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return cloneHooks(fallback);
  const record = raw as Partial<DebugBytecodeHookSettings>;
  return {
    mode: record.mode ?? fallback.mode,
    commandPreset: record.commandPreset ?? fallback.commandPreset,
    customCommands: Array.isArray(record.customCommands)
      ? record.customCommands.filter((command): command is string => typeof command === "string")
      : [...fallback.customCommands],
  };
}

function cloneHooks(hooks: DebugBytecodeHookSettings): DebugBytecodeHookSettings {
  return {
    mode: hooks.mode,
    commandPreset: hooks.commandPreset,
    customCommands: [...hooks.customCommands],
  };
}

function resolveDebugHookOptions(hooks: DebugBytecodeHookSettings) {
  const launchOptions: DebugLaunchRequest["launchOptions"] = {};
  if (hooks.mode !== "backend-default") {
    launchOptions.bytecodeHookMode = hooks.mode;
  }
  const commands = resolveHookCommands(hooks);
  if (commands) {
    launchOptions.bytecodeHookCommands = commands;
  }
  return launchOptions;
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
    case "custom":
      return hooks.customCommands.length > 0 ? hooks.customCommands : undefined;
  }
}

function compareProfiles(a: RunConfigurationProfile, b: RunConfigurationProfile): number {
  return a.name.localeCompare(b.name) || a.id.localeCompare(b.id);
}

function stripRecentId(id: string): string {
  return id.startsWith("profile:") ? id.slice("profile:".length) : id;
}

function randomId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `${Date.now()}`;
}
