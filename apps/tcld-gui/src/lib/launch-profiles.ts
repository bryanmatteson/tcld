import type { Settings } from "@/lib/settings";
import type { DebugLaunchRequest } from "@/lib/gui-workflows";
import { buildDebugLaunchRequest } from "@/lib/gui-workflows";
import type { DebugBytecodeHookSettings } from "@/lib/settings";

export interface LaunchProfile {
  id: string;
  name: string;
  path: string;
  args: string;
  env: string;
  cwd: string;
  tclshPath: string;
  bytecodeHooks: DebugBytecodeHookSettings;
}

export interface LaunchProfileSnapshot {
  profiles: LaunchProfile[];
  selectedProfileId: string | null;
}

export type LaunchProfileWindowMessage =
  | { type: "save"; profile: LaunchProfile }
  | { type: "cancel" }
  | { type: "delete"; id: string };

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

const STORAGE_KEY = "tcld-launch-profiles";

export function createLaunchProfileStore(args: { storage: StorageLike; settings: Settings }) {
  let snapshot = loadSnapshot(args.storage);
  function persist() {
    args.storage.setItem(STORAGE_KEY, JSON.stringify(snapshot));
  }
  return {
    saveProfile(profile: LaunchProfile): LaunchProfile {
      const next = normalizeProfile(profile, args.settings);
      snapshot = {
        ...snapshot,
        profiles: [...snapshot.profiles.filter((candidate) => candidate.id !== next.id), next].sort((a, b) => a.name.localeCompare(b.name)),
      };
      persist();
      return next;
    },
    deleteProfile(id: string) {
      snapshot = {
        profiles: snapshot.profiles.filter((candidate) => candidate.id !== id),
        selectedProfileId: snapshot.selectedProfileId === id ? null : snapshot.selectedProfileId,
      };
      persist();
    },
    selectProfile(id: string | null) {
      snapshot = { ...snapshot, selectedProfileId: id };
      persist();
    },
    snapshot(): LaunchProfileSnapshot {
      return { profiles: snapshot.profiles.map((profile) => ({ ...profile })), selectedProfileId: snapshot.selectedProfileId };
    },
  };
}

export function buildDebugLaunchRequestForProfile(profile: LaunchProfile, settings: Settings): DebugLaunchRequest {
  return buildDebugLaunchRequest(
    profile.path,
    profile,
    resolveDebugHookOptions(profile.bytecodeHooks ?? settings.debugger.launch.bytecodeHooks),
  );
}

export function buildSelectedLaunchProfileRequest(
  snapshot: LaunchProfileSnapshot,
  settings: Settings,
): DebugLaunchRequest | null {
  const profile = snapshot.profiles.find((candidate) => candidate.id === snapshot.selectedProfileId);
  return profile ? buildDebugLaunchRequestForProfile(profile, settings) : null;
}

export function launchProfileActionIds(): string[] {
  return [
    "debug:launch-ad-hoc",
    "debug:launch-profile",
    "launch-profile:open-window",
    "launch-profile:create",
    "launch-profile:edit",
    "launch-profile:delete",
  ];
}

export function reduceLaunchProfileWindowMessage(
  snapshot: LaunchProfileSnapshot,
  message: LaunchProfileWindowMessage,
): LaunchProfileSnapshot {
  switch (message.type) {
    case "save":
      return {
        ...snapshot,
        profiles: [
          ...snapshot.profiles.filter((profile) => profile.id !== message.profile.id),
          message.profile,
        ].sort((a, b) => a.name.localeCompare(b.name)),
        selectedProfileId: message.profile.id,
      };
    case "delete":
      return {
        profiles: snapshot.profiles.filter((profile) => profile.id !== message.id),
        selectedProfileId: snapshot.selectedProfileId === message.id ? null : snapshot.selectedProfileId,
      };
    case "cancel":
      return snapshot;
  }
}

function loadSnapshot(storage: StorageLike): LaunchProfileSnapshot {
  const raw = storage.getItem(STORAGE_KEY);
  if (!raw) return { profiles: [], selectedProfileId: null };
  try {
    const parsed = JSON.parse(raw) as LaunchProfileSnapshot;
    return {
      profiles: Array.isArray(parsed.profiles) ? parsed.profiles : [],
      selectedProfileId: parsed.selectedProfileId ?? null,
    };
  } catch {
    return { profiles: [], selectedProfileId: null };
  }
}

function normalizeProfile(profile: LaunchProfile, settings: Settings): LaunchProfile {
  return {
    ...profile,
    id: profile.id || `profile:${crypto.randomUUID()}`,
    bytecodeHooks: profile.bytecodeHooks ?? settings.debugger.launch.bytecodeHooks,
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
