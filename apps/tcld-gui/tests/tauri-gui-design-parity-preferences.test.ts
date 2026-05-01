import assert from "node:assert/strict";
import test from "node:test";
import {
  __setInvokeForTests,
  openPreferencesWindow,
  openRunConfigurationWindow,
} from "@/lib/tauri-api";
import {
  closeProductionPage,
  withProductionPage,
} from "./browser-production-harness.ts";
import { DESIGN_PREFERENCE_SECTION_MAPPINGS } from "@/lib/design-surface-contract";

test("V7 native Preferences and Run Configuration window commands stay on the Tauri IPC boundary", async () => {
  const calls: Array<{ cmd: string; args?: Record<string, unknown> }> = [];
  __setInvokeForTests(async (cmd, args) => {
    calls.push({ cmd, args });
    return null as never;
  });
  try {
    await openPreferencesWindow();
    await openRunConfigurationWindow();
  } finally {
    __setInvokeForTests(null);
  }

  assert.deepEqual(calls.map((call) => call.cmd), [
    "open_preferences_window",
    "open_run_configuration_window",
  ]);
});

test("V7 Preferences route hydrates without replacing the server-rendered app shell", async () => {
  const app = await withProductionPage("/?window=preferences&section=Appearance");
  try {
    await app.page.waitForSelector('[data-parity-state="preferences-appearance"]');

    const hydrationDiagnostics = [
      ...app.consoleMessages.map((message) => message.text),
      ...app.pageErrors,
    ].filter((message) =>
      /Hydration failed|server rendered HTML didn't match|hydration mismatch/i.test(message),
    );
    assert.deepEqual(hydrationDiagnostics, []);
  } finally {
    await closeProductionPage(app);
  }
});

test("V7 rendered Preferences route applies, persists, and emits appearance changes through production settings", async () => {
  const app = await withProductionPage("/?window=preferences&section=Appearance");
  try {
    await app.page.waitForSelector('[data-parity-state="preferences-appearance"]');
    await app.page.click('[data-appearance-theme="dracula"]');
    await app.page.click('[data-appearance-type-system="editorial"]');
    await app.page.click('[data-appearance-density="compact"]');
    await app.page.click('[data-appearance-toggle="sparklines"]');
    await app.page.click('[data-appearance-toggle="group-rails"]');
    await app.page.waitForFunction(() => document.documentElement.dataset.theme === "dracula");

    const evidence = await app.page.evaluate(() => ({
      dataset: {
        theme: document.documentElement.dataset.theme,
        typeSystem: document.documentElement.dataset.typeSystem,
        density: document.documentElement.dataset.density,
      },
      cssVars: {
        sparklines: getComputedStyle(document.documentElement).getPropertyValue("--variable-sparklines").trim(),
        rails: getComputedStyle(document.documentElement).getPropertyValue("--rail-display").trim(),
      },
      stored: JSON.parse(localStorage.getItem("tcld-settings") ?? "{}"),
      tauriAppearance: window.__TCLD_TEST_APPEARANCE__,
      events: window.__TCLD_TEST_EVENTS__,
    }));
    assert.deepEqual(evidence.dataset, { theme: "dracula", typeSystem: "editorial", density: "compact" });
    assert.equal(evidence.cssVars.sparklines, "none");
    assert.equal(evidence.cssVars.rails, "none");
    assert.equal(evidence.stored.theme, "dracula");
    assert.equal(evidence.tauriAppearance?.theme, "dracula");
    const appearanceEvents = evidence.events ?? [];
    assert.equal(
      appearanceEvents.some((event) => {
        if (event.event !== "appearance:changed") return false;
        const payload = event.payload;
        return typeof payload === "object" && payload !== null && (payload as { theme?: unknown }).theme === "dracula";
      }),
      true,
    );

    await app.page.click('[data-preferences-section="Decompiler"]');
    await app.page.waitForSelector('[data-parity-state="preferences-decompiler"]');
    await app.page.click('[data-preferences-section="Keybindings"]');
    await app.page.waitForSelector('[data-parity-state="preferences-keybindings"]');
  } finally {
    await closeProductionPage(app);
  }
});

test("V8 Preferences sidebar exposes every README section with live behavior or explicit disabled-state evidence", async () => {
  const app = await withProductionPage("/?window=preferences&section=Appearance");
  try {
    const groups = await app.page.$$eval("[data-preferences-group]", (nodes) =>
      nodes.map((node) => ({
        group: node.getAttribute("data-preferences-group"),
        title: node.querySelector("[data-preferences-group-title]")?.textContent?.trim() ?? "",
        sections: [...node.querySelectorAll("[data-preferences-section]")].map((section) =>
          section.querySelector("[data-preferences-label]")?.textContent?.trim() ?? section.textContent?.trim() ?? "",
        ),
      })),
    );
    assert.deepEqual(groups, [
      { group: "Workspace", title: "Workspace", sections: ["Appearance", "Editor", "Keybindings"] },
      { group: "Tools", title: "Tools", sections: ["Decompiler", "Debugger", "Run profiles"] },
      { group: "System", title: "System", sections: ["Telemetry", "Advanced"] },
    ]);
    assert.equal(
      await app.page.$eval('[data-preferences-section="Run profiles"] [data-preferences-badge]', (node) => node.textContent?.trim()),
      "3",
    );

    for (const section of DESIGN_PREFERENCE_SECTION_MAPPINGS) {
      await app.page.click(`[data-preferences-section="${section.label}"]`);
      await app.page.waitForSelector(section.productionSelector);
      const evidence = await app.page.$eval('[data-design-surface="preferences-window"]', (node) => ({
        parityState: node.getAttribute("data-parity-state"),
        readiness: node.querySelector("[data-preferences-readiness]")?.getAttribute("data-preferences-readiness") ?? null,
        disabled: node.querySelector("[data-preferences-readiness]")?.getAttribute("aria-disabled") ?? null,
        heading: node.querySelector("h1")?.textContent ?? "",
        text: node.textContent ?? "",
      }));
      assert.equal(evidence.parityState, `preferences-${section.slug}`);
      assert.match(evidence.heading, new RegExp(section.label.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));

      if (section.label === "Appearance") {
        assert.match(evidence.text, /Theme/);
        assert.equal(evidence.readiness, null);
      } else {
        assert.equal(evidence.readiness, section.slug);
        assert.equal(evidence.disabled, "true");
        assert.match(evidence.text, /intentionally present for visual parity/);
      }
    }
  } finally {
    await closeProductionPage(app);
  }
});

test("V7 rendered Run Configuration route saves, cancels, deletes, imports, and exports through production window events", async () => {
  const app = await withProductionPage("/?window=run-configuration");
  try {
    await app.page.waitForSelector('[data-parity-state="run-configuration"]');
    await app.page.click('[data-run-config-action="new"]');
    await app.page.click('[data-run-config-field="name"]', { clickCount: 3 });
    await app.page.type('[data-run-config-field="name"]', "Design Demo");
    await app.page.click('[data-run-config-field="path"]', { clickCount: 3 });
    await app.page.type('[data-run-config-field="path"]', "/tmp/design-demo.tcl");
    await app.page.click('[data-run-config-channel="stdout"]');
    await app.page.click('[data-run-config-channel="stderr"]');
    await app.page.select('[data-run-config-field="onExit"]', "restart");
    await app.page.click('[data-run-config-action="save"]');
    await app.page.waitForFunction(() => window.__TCLD_TEST_EVENTS__?.some((event) => event.event === "run-configuration:save"));

    await app.page.click('[data-run-config-action="export"]');
    await app.page.waitForFunction(() => Boolean(window.__TCLD_TEST_CLIPBOARD__));
    const exported = await app.page.evaluate(() => window.__TCLD_TEST_CLIPBOARD__);
    assert.ok(exported, "export must write the profile JSON to the clipboard");
    assert.match(exported, /Design Demo/);
    assert.match(exported, /"channelRouting"/);
    assert.match(exported, /"stdout": "discard"/);
    assert.match(exported, /"stderr": "discard"/);
    assert.match(exported, /"onExit": "restart"/);

    await app.page.click('[data-run-config-action="delete"]');
    await app.page.waitForFunction(() => window.__TCLD_TEST_EVENTS__?.some((event) => event.event === "run-configuration:delete"));

    await app.page.click('[data-run-config-action="import"]');
    await app.page.waitForFunction(() => window.__TCLD_TEST_EVENTS__?.some((event) => event.event === "run-configuration:import"));
    await app.page.waitForSelector('[data-run-config-profile]');

    await app.page.click('[data-run-config-action="cancel"]');
    await app.page.waitForFunction(() => window.__TCLD_TEST_EVENTS__?.some((event) => event.event === "run-configuration:cancel"));

    const events = await app.page.evaluate(() => (window.__TCLD_TEST_EVENTS__ ?? []).map((event) => event.event));
    assert.deepEqual(
      ["run-configuration:save", "run-configuration:export", "run-configuration:delete", "run-configuration:import", "run-configuration:cancel"].every((event) => events.includes(event)),
      true,
    );
  } finally {
    await closeProductionPage(app);
  }
});
