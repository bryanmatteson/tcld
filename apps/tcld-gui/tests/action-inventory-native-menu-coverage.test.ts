import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import type { CommandDef } from "@/lib/command-registry";

test("action inventory reports native-menu coverage metadata for every command", async () => {
  const { buildActionInventory } = await import("@/lib/action-inventory");
  const commands: CommandDef[] = [
    command("open", "File"),
    command("panel:graph", "Panels"),
    command("debug:continue", "Debug"),
    command("keybindings:import", "Settings"),
    command("internal:private", "Internal"),
  ];
  const inventory = buildActionInventory(commands, {
    nativeMenuItems: [
      { itemId: "file_open", actionId: "open", path: ["File", "Open"] },
      { itemId: "view_graph", actionId: "panel:graph", path: ["Tools", "Show Graph"] },
      { itemId: "debug_continue", actionId: "debug:continue", path: ["Debug", "Continue"] },
      { itemId: "keybindings_import", actionId: "keybindings:import", path: ["Tools", "Import Keybindings"] },
    ],
    nativeMenuExclusions: {
      "internal:private": "internal helper command",
    },
  });

  assert.deepEqual(inventory.nativeMenuMissingCoverageIds, []);
  assert.deepEqual(inventory.nativeMenuUnknownCommandIds, []);
  assert.equal(inventory.nativeMenuCoverage.get("open")?.status, "included");
  const internalCoverage = inventory.nativeMenuCoverage.get("internal:private");
  assert.equal(internalCoverage?.status, "excluded");
  assert.match(internalCoverage?.status === "excluded" ? internalCoverage.reason : "", /internal helper/);
});

test("action inventory fails closed on missing metadata and unknown native menu command ids", async () => {
  const { buildActionInventory } = await import("@/lib/action-inventory");
  const inventory = buildActionInventory(
    [command("open", "File"), command("panel:graph", "Panels")],
    {
      nativeMenuItems: [
        { itemId: "file_open", actionId: "open", path: ["File", "Open"] },
        { itemId: "unknown_action", actionId: "missing:action", path: ["Debug", "Missing"] },
      ],
    },
  );

  assert.deepEqual(inventory.nativeMenuMissingCoverageIds, ["panel:graph"]);
  assert.deepEqual(inventory.nativeMenuUnknownCommandIds, ["missing:action"]);
  assert.equal(inventory.nativeMenuCoverage.get("panel:graph")?.status, "missing");
});

test("production native-menu coverage includes broad app surfaces without unknown command ids", async () => {
  const { defaultNativeMenuCoverageContract, buildActionInventory } = await import("@/lib/action-inventory");
  const contract = defaultNativeMenuCoverageContract();
  const commands = realProductionCommandIds().map((id) => command(id, categoryFor(id)));
  const inventory = buildActionInventory(commands, contract);

  assert.deepEqual(inventory.nativeMenuUnknownCommandIds, []);
  assert.deepEqual(inventory.nativeMenuMissingCoverageIds, []);
  for (const id of [
    "open",
    "panel:graph",
    "debug:run_to_cursor",
    "source:fold-all",
    "instructions:fold-all",
    "format:apply",
    "appearance:open",
    "keybindings:import",
    "palette:open",
  ]) {
    assert.equal(inventory.nativeMenuCoverage.get(id)?.status, "included", `${id} should be in the native menu`);
  }
  assert.deepEqual([...new Set(contract.nativeMenuItems.map((item) => item.path[0]))], [
    "File",
    "View",
    "Navigate",
    "Run",
    "Debug",
    "Tools",
    "Help",
  ]);
  assert.equal(contract.nativeMenuItems.some((item) => item.path[0] === "Graph"), false);
  assert.equal(contract.nativeMenuItems.some((item) => item.path[0] === "Keybindings"), false);
});

test("active-file native menu commands use hasFile command predicates in the real page inventory", async () => {
  const { defaultNativeMenuCoverageContract } = await import("@/lib/action-inventory");
  const page = readProductionFile("../src/app/page.tsx");
  const activeFileActionIds = defaultNativeMenuCoverageContract()
    .nativeMenuItems
    .filter((item) => !item.actionId.startsWith("graph:"))
    .filter((item) => activeFileNativeMenuItemIds().has(item.itemId))
    .map((item) => item.actionId);

  for (const actionId of activeFileActionIds) {
    assert.match(
      page,
      new RegExp(`id: "${escapeRegExp(actionId)}"[\\s\\S]{0,180}when: "[^"]*hasFile`),
      `${actionId} must be disabled by CommandContext.hasFile when no file is loaded`,
    );
  }
});

function command(id: string, category: string): CommandDef {
  return {
    id,
    title: id,
    category,
    run: () => {},
  };
}

function categoryFor(id: string): string {
  if (id.startsWith("panel:")) return "Panels";
  if (id.startsWith("debug:")) return "Debug";
  if (id.startsWith("source:")) return "Source";
  if (id.startsWith("instructions:")) return "Disassembly";
  if (id.startsWith("format:")) return "Actions";
  if (id.startsWith("keybindings:")) return "Settings";
  if (id.startsWith("palette:") || id === "help") return "General";
  if (id.startsWith("layout:")) return "Layout";
  if (id.startsWith("appearance:")) return "Settings";
  return "File";
}

function realProductionCommandIds(): string[] {
  const sources = [
    readProductionFile("../src/app/page.tsx"),
    readProductionFile("../src/components/panels/GraphPanel.tsx"),
  ];
  const literalIds = sources.flatMap((source) => [...source.matchAll(/id: "([^"]+)"/g)].map((match) => match[1]!));
  const dynamicIds = [
    "layout:default",
    "layout:analysis",
    "layout:wide",
    "layout:decompile",
    "layout:debug",
    "layout:overview",
    "theme:tokyo-night",
    "font:jetbrains-mono",
    "density:normal",
    "density:compact",
    "density:ultra",
    ..."abcdefghijklmnopqrstuvwxyz".split("").flatMap((letter) => [
      `mark:set:${letter}`,
      `mark:goto:${letter}`,
      `mark:clear:${letter}`,
    ]),
  ];
  return [...new Set([...literalIds, ...dynamicIds])].sort((a, b) => a.localeCompare(b));
}

function activeFileNativeMenuItemIds(): Set<string> {
  return new Set([
    "file_copy_panel",
    "file_copy_diagnostics",
    "file_copy_full_report",
    "file_export_decompile",
    "file_export_disassembly",
    "file_export_diagnostics",
    "file_export_provenance",
    "file_export_comparison",
    "file_export_full_report",
    "view_instructions",
    "view_decompile",
    "view_source",
    "view_comparison",
    "view_diagnostics",
    "view_provenance",
    "view_project",
    "view_detail",
    "view_literals",
    "view_exceptions",
    "view_graph",
    "view_summary",
    "source_fold_all",
    "source_unfold_all",
    "instructions_fold_all",
    "instructions_unfold_all",
    "jumparrows",
    "format_preview",
    "format_apply",
  ]);
}

function readProductionFile(relativePath: string): string {
  return readFileSync(new URL(relativePath, import.meta.url), "utf8");
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
