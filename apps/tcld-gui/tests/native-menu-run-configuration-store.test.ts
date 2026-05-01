import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import type { CommandContext, CommandDef } from "@/lib/command-registry";
import type { RunConfigurationState } from "@/lib/run-configuration-store";
import type { Settings } from "@/lib/settings";

function fixtureSettings(): Settings {
  return {
    theme: "tokyo-night",
    typeSystem: "mono-brutalist",
    fontFamily: "mono-brutalist",
    fontSize: 13,
    density: "regular",
    showSparklines: true,
    groupRails: true,
    panelColumns: {},
    debugger: {
      launch: {
        bytecodeHooks: {
          mode: "intercept",
          commandPreset: "tbcload",
          customCommands: [],
        },
      },
    },
  };
}

test("native menu dispatch routes Tauri item ids through command registry ids and command context availability", async () => {
  const {
    executeNativeMenuAction,
    nativeMenuActionId,
    nativeMenuAvailability,
    recentConfigurationNativeMenuActionId,
  } = await import("@/lib/gui-workflows");
  const { commandRegistry } = await import("@/lib/command-registry");
  const calls: string[] = [];
  const cleanup = [
    register(commandRegistry, "open", calls),
    register(commandRegistry, "panel:graph", calls, "hasFile"),
    register(commandRegistry, "panel:source", calls, "hasFile"),
    register(commandRegistry, "debug:continue", calls, "debugState:paused,!debugBusy,!isExceptionStop"),
    register(commandRegistry, "run-configuration:open-window", calls),
    register(commandRegistry, "run-configuration:recent:profile:demo", calls),
  ];
  const idle: CommandContext = {
    overlayOpen: false,
    focusedPanel: null,
    debugState: "idle",
    debugBusy: false,
    isExceptionStop: false,
    hasFile: false,
    hasSelection: false,
    debuggerAvailable: true,
  };
  const paused: CommandContext = { ...idle, debugState: "paused", hasFile: true, hasSelection: true };

  try {
    assert.equal(nativeMenuActionId("file_open"), "open");
    assert.equal(nativeMenuActionId("view_graph"), "panel:graph");
    assert.equal(nativeMenuAvailability("view_source", idle, commandRegistry), false);
    assert.equal(nativeMenuAvailability("view_source", paused, commandRegistry), true);
    assert.equal(nativeMenuActionId("run_configuration"), "run-configuration:open-window");
    assert.equal(recentConfigurationNativeMenuActionId("run_recent_configuration:profile:demo"), "run-configuration:recent:profile:demo");
    assert.equal(nativeMenuAvailability("debug_continue", idle, commandRegistry), false);
    assert.equal(nativeMenuAvailability("debug_continue", paused, commandRegistry), true);
    assert.equal(executeNativeMenuAction("file_open", (actionId) => commandRegistry.execute(actionId)), true);
    assert.equal(executeNativeMenuAction("run_recent_configuration:profile:demo", (actionId) => commandRegistry.execute(actionId)), true);
    assert.deepEqual(calls, ["open", "run-configuration:recent:profile:demo"]);
  } finally {
    cleanup.forEach((fn) => fn());
  }
});

test("run configuration reducer owns profiles, drafts, ad hoc settings, import/export, and launch requests", async () => {
  const {
    buildRunConfigurationLaunchRequest,
    createRunConfigurationStore,
    exportRunConfigurationsJson,
    importRunConfigurationsJson,
    initialRunConfigurationState,
    reduceRunConfigurationState,
    runConfigurationActionIds,
  } = await import("@/lib/run-configuration-store");
  const settings = fixtureSettings();
  const storage = memoryStorage();
  const store = createRunConfigurationStore({ storage, settings });

  store.dispatch({
    type: "save-profile",
    profile: {
      id: "profile:demo",
      name: "Demo",
      path: "/tmp/demo.tcl",
      args: "--flag two",
      env: "A=1 B=2",
      cwd: "/tmp",
      tclshPath: "/usr/bin/tclsh",
      backend: "external",
      bytecodeHooks: { mode: "intercept", commandPreset: "tbcload", customCommands: [] },
      channelRouting: {
        stdout: "discard",
        stderr: "console",
        eval: "console",
        system: "discard",
        forwardSignals: false,
        onExit: "restart",
      },
    },
  });
  store.dispatch({ type: "select-profile", id: "profile:demo" });
  store.dispatch({ type: "update-ad-hoc", patch: { args: "adhoc", env: "C=3" } });

  const snapshot = store.snapshot();
  const selected = buildRunConfigurationLaunchRequest(snapshot, settings);
  const exported = exportRunConfigurationsJson(snapshot);
  const imported = importRunConfigurationsJson(exported, initialRunConfigurationState(settings), settings);
  const malformed = importRunConfigurationsJson("{", snapshot, settings);
  const rejected = importRunConfigurationsJson(JSON.stringify({ profiles: [{ id: 4, name: "bad" }] }), snapshot, settings);
  const deleted = reduceRunConfigurationState(snapshot, { type: "delete-profile", id: "profile:demo" }, settings);

  assert.deepEqual(runConfigurationActionIds().sort(), [
    "debug:launch-ad-hoc",
    "debug:launch-profile",
    "run-configuration:delete",
    "run-configuration:export",
    "run-configuration:import",
    "run-configuration:open-window",
    "run-configuration:recent",
    "run-configuration:save",
  ].sort());
  assert.deepEqual(selected?.args, ["--flag", "two"]);
  assert.deepEqual(selected?.launchOptions.env, { A: "1", B: "2" });
  assert.equal(selected?.launchOptions.backend, "external", "profile backend flows into launch options");
  assert.deepEqual(selected?.channelRouting, {
    stdout: "discard",
    stderr: "console",
    eval: "console",
    system: "discard",
    forwardSignals: false,
    onExit: "restart",
  });
  assert.match(exported, /"selectedProfileId": "profile:demo"/);
  assert.match(exported, /"channelRouting"/);
  assert.match(exported, /"stdout": "discard"/);
  assert.match(exported, /"backend": "external"/);
  assert.equal(imported.ok, true);
  assert.equal(imported.state.profiles[0]?.name, "Demo");
  assert.equal(imported.state.profiles[0]?.channelRouting.stdout, "discard");
  assert.equal(imported.state.profiles[0]?.backend, "external", "backend round-trips through export/import");
  assert.equal(
    imported.state.adHoc.backend,
    "embedded",
    "ad hoc configuration defaults to the embedded backend",
  );
  // Legacy persisted profiles (no backend field) and junk tokens both
  // normalize to embedded instead of failing the import.
  const legacy = importRunConfigurationsJson(
    JSON.stringify({
      version: 1,
      selectedProfileId: null,
      profiles: [
        { id: "profile:legacy", name: "Legacy" },
        { id: "profile:junk", name: "Junk", backend: "quantum" },
      ],
    }),
    initialRunConfigurationState(settings),
    settings,
  );
  assert.equal(legacy.ok, true);
  assert.equal(legacy.state.profiles[0]?.backend, "embedded");
  assert.equal(legacy.state.profiles[1]?.backend, "embedded");
  assert.equal(malformed.ok, false);
  assert.match(malformed.diagnostics[0]?.message ?? "", /malformed/i);
  assert.equal(rejected.ok, false);
  assert.deepEqual(rejected.state, snapshot, "invalid import leaves existing state unchanged");
  assert.equal(deleted.selectedProfileId, null);
});

test("public_complex_fixture.tbc run configuration keeps embedded launch and TclPro bytecode hook options", async () => {
  const {
    buildRunConfigurationLaunchRequest,
    initialRunConfigurationState,
    reduceRunConfigurationState,
  } = await import("@/lib/run-configuration-store");
  const settings = fixtureSettings();
  const state = reduceRunConfigurationState(initialRunConfigurationState(settings), {
    type: "save-profile",
    profile: {
      id: "profile:fixture",
      name: "public_complex_fixture.tbc",
      path: "/repo/testing/fixtures/public/public_complex_fixture.tbc",
      args: "",
      env: "CSC_SERVER_HOME=/tmp/csc",
      cwd: "/repo",
      tclshPath: "",
      backend: "embedded",
      bytecodeHooks: { mode: "intercept", commandPreset: "tbcload", customCommands: [] },
      channelRouting: {
        stdout: "console",
        stderr: "console",
        eval: "console",
        system: "console",
        forwardSignals: true,
        onExit: "keep-open",
      },
    },
  }, settings);

  const request = buildRunConfigurationLaunchRequest(state, settings);

  assert.equal(request?.path, "/repo/testing/fixtures/public/public_complex_fixture.tbc");
  assert.equal(request?.launchOptions.backend, undefined, "embedded remains the backend default");
  assert.equal(request?.launchOptions.cwd, "/repo");
  assert.deepEqual(request?.launchOptions.env, { CSC_SERVER_HOME: "/tmp/csc" });
  assert.equal(request?.launchOptions.bytecodeHookMode, "intercept");
  assert.deepEqual(request?.launchOptions.bytecodeHookCommands, [
    "::tbcload::bceval",
    "::tbcload::bcproc",
  ]);
});

test("recent configurations derive profile and recent script entries and launch through the same builder", async () => {
  const {
    buildRecentRunConfigurations,
    buildRecentRunConfigurationMenuItems,
    buildRunConfigurationLaunchRequest,
    reduceRunConfigurationState,
  } = await import("@/lib/run-configuration-store");
  const settings = fixtureSettings();
  const state = reduceRunConfigurationState(initialState(settings), {
    type: "save-profile",
    profile: {
      id: "profile:demo",
      name: "Demo",
      path: "/tmp/demo.tcl",
      args: "--flag",
      env: "",
      cwd: "",
      tclshPath: "",
      bytecodeHooks: settings.debugger.launch.bytecodeHooks,
      channelRouting: {
        stdout: "console",
        stderr: "console",
        eval: "console",
        system: "console",
        forwardSignals: true,
        onExit: "keep-open",
      },
    },
  }, settings);
  const recent = buildRecentRunConfigurations(state, [
    { path: "/tmp/recent.tcl", filename: "recent.tcl", mode: "debug", timestamp: 2 },
    { path: "/tmp/open.tcl", filename: "open.tcl", mode: "open", timestamp: 1 },
  ]);
  const profileItem = recent.find((item) => item.kind === "profile");
  const scriptItem = recent.find((item) => item.kind === "script");

  assert.equal(profileItem?.actionId, "run-configuration:recent:profile:demo");
  assert.equal(scriptItem?.actionId, "run-configuration:recent:script:/tmp/recent.tcl");
  assert.deepEqual(buildRecentRunConfigurationMenuItems(recent, 3), [
    {
      itemId: "run_recent_configuration:0",
      actionId: "run-configuration:recent:profile:demo",
      label: "Demo",
      enabled: true,
    },
    {
      itemId: "run_recent_configuration:1",
      actionId: "run-configuration:recent:script:/tmp/recent.tcl",
      label: "recent.tcl",
      enabled: true,
    },
    {
      itemId: "run_recent_configuration:2",
      actionId: null,
      label: "Recent Configuration 3",
      enabled: false,
    },
  ]);
  assert.deepEqual(buildRunConfigurationLaunchRequest(state, settings, profileItem)?.path, "/tmp/demo.tcl");
  assert.deepEqual(buildRunConfigurationLaunchRequest(state, settings, scriptItem)?.path, "/tmp/recent.tcl");
});

test("main app removes inline run configuration fields and routes full editing to the native window", () => {
  const page = readProductionFile("../src/app/page.tsx");

  assert.match(page, /windowMode === "run-configuration"/);
  assert.match(page, /function RunConfigurationWindowApp\(/);
  assert.match(page, /createRunConfigurationStore\(\{ storage: browserStorage\(\), settings \}\)/);
  assert.match(page, /run-configuration:open-window/);
  assert.match(page, /openRunConfigurationWindow\(\)/);
  assert.match(page, /menu:recent-configurations/);
  assert.match(page, /recentRunConfigurationActionByMenuItem/);
  assert.doesNotMatch(page, /setLaunchConfig/);
  assert.doesNotMatch(page, /value=\{launchConfig/);
});

function initialState(settings: Settings): RunConfigurationState {
  return {
    profiles: [],
    selectedProfileId: null,
    adHoc: {
      args: "",
      env: "",
      cwd: "",
      tclshPath: "",
      bytecodeHooks: settings.debugger.launch.bytecodeHooks,
      channelRouting: {
        stdout: "console",
        stderr: "console",
        eval: "console",
        system: "console",
        forwardSignals: true,
        onExit: "keep-open",
      },
    },
    draft: null,
  };
}

function register(
  commandRegistry: typeof import("@/lib/command-registry").commandRegistry,
  id: string,
  calls: string[],
  when?: string,
): () => void {
  const command: CommandDef = {
    id,
    title: id,
    category: "Test",
    when,
    run: () => calls.push(id),
  };
  return commandRegistry.register(command);
}

function memoryStorage() {
  const data = new Map<string, string>();
  return {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => {
      data.set(key, value);
    },
    removeItem: (key: string) => {
      data.delete(key);
    },
  };
}

function readProductionFile(relativePath: string): string {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}
