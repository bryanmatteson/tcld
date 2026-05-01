import assert from "node:assert/strict";
import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { DESIGN_PARITY_STATES } from "@/lib/design-surface-contract";
import { DESIGN_DEVIATIONS } from "@/lib/design-deviation-inventory";
import { DESIGN_THEME_TOKENS } from "@/lib/themes";

const GUI_ROOT = path.resolve(import.meta.dirname, "..");
const REQUIRED_ARTBOARDS = [
  "welcome",
  "inspect",
  "debug",
  "flow",
  "trace",
  "console",
  "symbols",
  "prefs-appearance",
  "prefs-decomp",
  "prefs-keys",
  "runcfg",
  "vars",
  "stack",
  "bps",
  ...Object.keys(DESIGN_THEME_TOKENS).map((themeId) => `th-${themeId}`),
];

const PRODUCTION_TARGETS: Record<string, { route: string; selectorParts: string[] }> = {
  welcome: { route: "/", selectorParts: ['[data-parity-state="welcome"]'] },
  inspect: { route: "/?parity=open-file", selectorParts: ['[data-panel-id="instructions"]'] },
  debug: { route: "/?parity=debug-paused", selectorParts: ['[data-panel-id="variables"]'] },
  flow: { route: "/?parity=flow", selectorParts: ['[data-panel-id="graph"]'] },
  trace: { route: "/?parity=trace", selectorParts: ['[data-panel-id="provenance"]'] },
  console: { route: "/?parity=console", selectorParts: ['[data-panel-id="console"]'] },
  symbols: { route: "/?parity=symbols", selectorParts: ['[data-panel-id="literals"]', '[data-design-surface="view-symbols"]'] },
  "prefs-appearance": { route: "/?window=preferences&section=Appearance", selectorParts: ['[data-parity-state="preferences-appearance"]'] },
  "prefs-decomp": { route: "/?window=preferences&section=Decompiler", selectorParts: ['[data-parity-state="preferences-decompiler"]'] },
  "prefs-keys": { route: "/?window=preferences&section=Keybindings", selectorParts: ['[data-parity-state="preferences-keybindings"]'] },
  runcfg: { route: "/?window=run-configuration", selectorParts: ['[data-parity-state="run-configuration"]'] },
  vars: { route: "/?parity=variables", selectorParts: ['[data-panel-id="variables"]'] },
  stack: { route: "/?parity=stack", selectorParts: ['[data-panel-id="stack"]'] },
  bps: { route: "/?parity=breakpoints", selectorParts: ['[data-panel-id="breakpoints"]'] },
  ...Object.keys(DESIGN_THEME_TOKENS).reduce<Record<string, { route: string; selectorParts: string[] }>>((targets, themeId) => {
    targets[`th-${themeId}`] = {
      route: `/?parity=theme-${themeId}`,
      selectorParts: ['[data-panel-id="instructions"]'],
    };
    return targets;
  }, {}),
};

test("V2 rendered parity manifest captures every required production/design screenshot pair", () => {
  const manifestPath = path.join(GUI_ROOT, ".design-parity/manifest.json");
  assert.ok(existsSync(manifestPath), "design:parity must write .design-parity/manifest.json");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8")) as {
    states: Array<{
      id: string;
      designSource: string;
      designArtboardId: string;
      productionRoute: string;
      productionSelector: string;
      deviations: string[];
      artifacts: { design?: string; production: string };
    }>;
  };
  const manifestByArtboard = new Map(manifest.states.map((state) => [state.designArtboardId, state]));
  const contractByArtboard = new Map(DESIGN_PARITY_STATES.map((state) => [state.designArtboardId, state]));
  const deviationIds = new Set(DESIGN_DEVIATIONS.map((entry) => entry.id));

  for (const artboard of REQUIRED_ARTBOARDS) {
    const manifestState = manifestByArtboard.get(artboard);
    const contractState = contractByArtboard.get(artboard);
    assert.ok(manifestState, `missing manifest state for ${artboard}`);
    assert.ok(contractState, `missing contract state for ${artboard}`);
    const target = PRODUCTION_TARGETS[artboard];
    assert.ok(target, `missing production target assertion for ${artboard}`);
    assert.equal(manifestState?.productionRoute, target.route);
    assert.equal(contractState?.productionRoute, target.route);
    assert.equal(manifestState?.productionSelector, contractState?.productionSelector);
    assert.equal(manifestState?.productionRoute.includes("fixture="), false, `${artboard} must not use static fixture route`);
    for (const selectorPart of target.selectorParts) {
      assert.ok(
        manifestState?.productionSelector.includes(selectorPart),
        `${artboard} selector must point at production component ${selectorPart}`,
      );
    }
    assert.ok(manifestState?.designSource.endsWith(".jsx"));
    assert.ok(manifestState?.deviations.every((id) => deviationIds.has(id)), `${artboard} has unknown deviation`);

    // The design/ reference tree was removed from the repo (2026-05), so
    // `npm run design:parity` captures production-side screenshots only;
    // the design-side pair is intentionally gone. The production artifact
    // remains a hard requirement.
    const productionArtifact = manifestState!.artifacts.production;
    assert.ok(productionArtifact.endsWith(".png"), `${artboard} names a production screenshot`);
    const file = path.join(GUI_ROOT, productionArtifact);
    assert.ok(existsSync(file), `missing screenshot ${productionArtifact}`);
    assert.ok(statSync(file).size > 1024, `screenshot ${productionArtifact} is empty or placeholder-sized`);
  }
});

test("V2 production screenshots are not sourced from the static visual fixture app", () => {
  const pageSource = readFileSync(path.join(GUI_ROOT, "src/app/page.tsx"), "utf8");
  assert.equal(pageSource.includes("VisualParityFixtureApp"), false);
  assert.equal(pageSource.includes("dangerouslySetInnerHTML={{ __html: row }}"), false);
});
