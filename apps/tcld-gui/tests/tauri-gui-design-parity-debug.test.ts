import assert from "node:assert/strict";
import test from "node:test";
import type { Page } from "puppeteer";
import {
  closeProductionPage,
  waitForText,
  waitForVisibleText,
  withProductionPage,
} from "./browser-production-harness.ts";

/**
 * Activate a debug-layout view by its tab. Tabs that no longer fit inline
 * spill into the "+N" overflow menu, so fall back to selecting through it.
 */
async function openDebugTab(page: Page, viewId: string): Promise<void> {
  const inline = await page.$(`[data-tab-view-id="${viewId}"]`);
  if (inline) {
    await inline.click();
  } else {
    await page.click('[data-tab-overflow-trigger="true"]');
    await page.waitForSelector(`[data-overflow-tab-id="${viewId}"] [role="menuitem"]`);
    await page.click(`[data-overflow-tab-id="${viewId}"] [role="menuitem"]`);
  }
  await page.waitForSelector(`[data-panel-id="${viewId}"]`);
}

test("V7 Run transitions to DebugOverlay, renders current source, and dispatches debug controls", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/debug-open.tbc", debugPath: "/tmp/debug-script.tcl" });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /debug-open\.tbc/);

    await app.page.click('[title="Run / Debug (F5)"]');
    await app.page.waitForSelector('.relative[data-design-surface="debug-paused-overlay"]');
    assert.equal(await app.page.$(".tcld-main-toolbar"), null, "ActionBar must be replaced while debug is active");
    await app.page.waitForSelector('[data-panel-id="decompile"] [data-execution="true"]');
    // Run debugs the currently-loaded document (no script dialog), so the
    // overlay reports the loaded .tbc path.
    await waitForText(app.page, '.relative[data-design-surface="debug-paused-overlay"]', /debug-open\.tbc:2|debug-open\.tbc/);

    await app.page.click('[title="Step Into (F11)"]');
    await app.page.click('[title="Step Over (F10)"]');
    await app.page.click('[title="Continue (F5)"]');
    await app.page.click('[title="Stop (⇧F5)"]');

    const invoked = await app.page.evaluate(() =>
      (window.__TCLD_TEST_INVOKES__ ?? []).map((entry) => entry.cmd),
    );
    for (const cmd of ["debug_launch", "debug_step_into", "debug_step_over", "debug_continue", "debug_stop"]) {
      assert.ok(invoked.includes(cmd), `${cmd} must dispatch through the Tauri debug boundary`);
    }
  } finally {
    await closeProductionPage(app);
  }
});

test("V7 debug rail supports Variables search/scope/expand/changed rows, stack selection, and breakpoint metadata", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/debug-open.tbc", debugPath: "/tmp/debug-script.tcl" });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /debug-open\.tbc/);
    await app.page.click('[title="Run / Debug (F5)"]');
    await app.page.waitForSelector('.relative[data-design-surface="debug-paused-overlay"]');

    await app.page.evaluate(() =>
      window.__TCLD_TEST_TAURI__?.emit?.("debugger:variables", {
        level: 1,
        vars: [
          {
            name: "argv",
            value: "{demo.tbc --trace}",
            type: "list",
            scope: "local",
            changed: true,
            children: [
              { name: "argv[0]", value: "demo.tbc", type: "string", scope: "local" },
              { name: "argv[1]", value: "--trace", type: "string", scope: "local" },
            ],
          },
          { name: "argc", value: "2", type: "int", scope: "local" },
          { name: "env(PATH)", value: "/usr/bin:/bin", type: "string", scope: "global" },
        ],
      }),
    );

    await openDebugTab(app.page, "variables");
    await waitForVisibleText(app.page, '[data-panel-id="variables"]', /argv/);
    await app.page.waitForSelector('[data-panel-id="variables"] [data-variable-changed="true"]');
    await app.page.click('[data-panel-id="variables"] .tcld-variable-row');
    await waitForVisibleText(app.page, '[data-panel-id="variables"]', /argv\[0\]/);

    await app.page.$$eval('[data-panel-id="variables"] button', (buttons) => {
      const global = buttons.find((button) => button.textContent?.trim() === "global") as HTMLButtonElement | undefined;
      global?.click();
    });
    const afterScopeToggle = await app.page.$eval('[data-panel-id="variables"]', (node) => node.textContent ?? "");
    assert.doesNotMatch(afterScopeToggle, /env\(PATH\)/);

    await app.page.type('[data-panel-id="variables"] input[placeholder="Filter variables..."]', "argc");
    await waitForVisibleText(app.page, '[data-panel-id="variables"]', /argc/);
    const afterSearch = await app.page.$eval('[data-panel-id="variables"]', (node) => node.textContent ?? "");
    assert.match(afterSearch, /argc/);
    assert.doesNotMatch(afterSearch, /argv\[0\]/);

    await openDebugTab(app.page, "stack");
    await waitForVisibleText(app.page, '[data-panel-id="stack"]', /sample_proc/);
    const topFrameName = await app.page.$eval(
      '[data-panel-id="stack"] .tcld-stack-card:first-child .tcld-stack-name',
      (node) => node.textContent ?? "",
    );
    assert.equal(topFrameName, "sample_proc");
    await app.page.click('[data-panel-id="stack"] .tcld-stack-card:nth-of-type(2)');
    await app.page.waitForFunction(() =>
      (window.__TCLD_TEST_INVOKES__ ?? []).some((entry) => entry.cmd === "debug_get_variables" && entry.args?.level === 1),
    );

    // The backend-seeded breakpoint (condition + hit count) reaches the
    // panel through the on-stop breakpoint sync.
    await openDebugTab(app.page, "breakpoints");
    await waitForVisibleText(app.page, '[data-panel-id="breakpoints"]', /debug-open\.tbc/);
    await waitForVisibleText(app.page, '[data-panel-id="breakpoints"]', /\$argc > 0/);
    await waitForVisibleText(app.page, '[data-panel-id="breakpoints"]', /3h/);
    const hitCountMarker = await app.page.$eval('[data-panel-id="breakpoints"] [title="3 hits"]', (node) => node.textContent ?? "");
    assert.match(hitCountMarker, /3h/, "breakpoint rows surface the backend hit count");
    await app.page.click('[data-panel-id="breakpoints"] .bp-row .dot');
    await app.page.waitForFunction(() =>
      (window.__TCLD_TEST_INVOKES__ ?? []).some((entry) => entry.cmd === "debug_set_breakpoint_enabled"),
    );
  } finally {
    await closeProductionPage(app);
  }
});
