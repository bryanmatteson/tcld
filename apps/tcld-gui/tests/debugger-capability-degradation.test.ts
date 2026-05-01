import assert from "node:assert/strict";
import test from "node:test";
import type { Page } from "puppeteer";
import {
  closeProductionPage,
  waitForVisibleText,
  withProductionPage,
} from "./browser-production-harness.ts";

/**
 * B6 — explicit degradation when the Rust backend was built without the
 * `debugger` feature. The `get_app_capabilities` handshake drives:
 *   - the main-toolbar run cluster (disabled + tooltip),
 *   - Debug commands in the registry / native menu availability,
 *   - gutter breakpoint toggles,
 *   - debug-only panels (Variables / Stack / Breakpoints / Watchpoints),
 *   - the debug store's launch refusal (no raw IPC "command not found").
 */

const UNAVAILABLE_PATTERN = /This build does not include the debugger/;
const DEBUG_VIEW_IDS = ["variables", "stack", "breakpoints", "watchpoints"] as const;

/**
 * Activate a view by its tab, falling back to the "+N" overflow menu when
 * the tab strip spills (same helper shape as the design-parity debug suite).
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

test("B6 capabilities module normalizes the handshake and fetches it once", async () => {
  const { __setInvokeForTests } = await import("@/lib/tauri-api");
  const { loadAppCapabilities, isDebuggerAvailable, __resetAppCapabilitiesForTests } = await import(
    "@/lib/capabilities"
  );

  const calls: string[] = [];
  const respond = (value: unknown) =>
    async <T>(cmd: string): Promise<T> => {
      calls.push(cmd);
      return value as T;
    };

  try {
    __resetAppCapabilitiesForTests();
    assert.equal(isDebuggerAvailable(), true, "default before the handshake resolves is available");

    __setInvokeForTests(respond({ debugger_available: false }));
    await loadAppCapabilities();
    assert.equal(isDebuggerAvailable(), false, "explicit false degrades the debugger");
    await loadAppCapabilities();
    assert.deepEqual(calls, ["get_app_capabilities"], "handshake is fetched once per app lifetime");

    // Malformed payloads keep the historical default instead of hiding UI.
    __resetAppCapabilitiesForTests();
    __setInvokeForTests(respond(null));
    await loadAppCapabilities();
    assert.equal(isDebuggerAvailable(), true, "null payload keeps debugger available");

    // Older backends without the command resolve to available.
    __resetAppCapabilitiesForTests();
    __setInvokeForTests(async <T>(): Promise<T> => {
      throw new Error("command get_app_capabilities not found");
    });
    await loadAppCapabilities();
    assert.equal(isDebuggerAvailable(), true, "missing command keeps debugger available");
  } finally {
    __setInvokeForTests(null);
    __resetAppCapabilitiesForTests();
  }
});

test("B6 debugger-less backend disables run controls, shows unavailable states, and never invokes debug IPC", async () => {
  const app = await withProductionPage("/", {
    dialogPath: "/tmp/debug-open.tbc",
    debugPath: "/tmp/debug-script.tcl",
    debuggerAvailable: false,
  });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /debug-open\.tbc/);

    // Main-toolbar run cluster: disabled with the explanatory tooltip once
    // the handshake resolves.
    await app.page.waitForSelector('[data-actionbar-launch-item="run"][data-launch-unavailable="true"][disabled]');
    const runTitle = await app.page.$eval(
      '[data-actionbar-launch-item="run"]',
      (el) => el.getAttribute("title") ?? "",
    );
    assert.match(runTitle, UNAVAILABLE_PATTERN);

    // Clicking the disabled control does nothing (no dialog, no launch).
    await app.page.click('[data-actionbar-launch-item="run"]');

    // A native Debug-menu dispatch (menu:action) degrades to a status
    // message via the launch-path guard instead of a file dialog + IPC.
    await app.page.evaluate(() => window.__TCLD_TEST_TAURI__?.emit?.("menu:action", "debug_launch"));
    await waitForVisibleText(app.page, '[data-design-surface="status-bar"]', UNAVAILABLE_PATTERN);

    // Native menu availability payload reports every debug item disabled
    // (the command registry gates them on ctx.debuggerAvailable).
    await app.page.waitForFunction(() => {
      const events = window.__TCLD_TEST_EVENTS__ ?? [];
      const last = [...events].reverse().find((entry) => entry.event === "menu:availability");
      const payload = (last?.payload ?? null) as Record<string, boolean> | null;
      return payload !== null && payload.debug_launch === false;
    });
    const availability = await app.page.evaluate(() => {
      const events = window.__TCLD_TEST_EVENTS__ ?? [];
      const last = [...events].reverse().find((entry) => entry.event === "menu:availability");
      return (last?.payload ?? {}) as Record<string, boolean>;
    });
    for (const itemId of [
      "debug_launch",
      "debug_launch_profile",
      "debug_continue",
      "debug_step_into",
      "debug_toggle_breakpoint",
      "debug_clear_all_breakpoints",
    ]) {
      assert.equal(availability[itemId], false, `${itemId} must be reported unavailable to the native menu`);
    }
    assert.equal(availability.file_open, true, "non-debug menu items stay enabled");

    // Gutter breakpoint toggles render disabled with the tooltip and a
    // click records nothing.
    await app.page.waitForSelector('[data-panel-id="decompile"] [data-source-breakpoint-gutter="true"]');
    const gutter = await app.page.$eval(
      '[data-panel-id="decompile"] [data-source-breakpoint-gutter="true"]',
      (el) => ({
        title: el.getAttribute("title") ?? "",
        unavailable: el.getAttribute("data-source-breakpoint-unavailable"),
      }),
    );
    assert.equal(gutter.unavailable, "true");
    assert.match(gutter.title, UNAVAILABLE_PATTERN);
    await app.page.click('[data-panel-id="decompile"] [data-source-breakpoint-gutter="true"]');
    assert.equal(
      await app.page.$('[data-source-breakpoint-active="true"]'),
      null,
      "gutter click must not set a breakpoint when the debugger is unavailable",
    );

    // Debug-only panels show the explicit unavailable empty state rather
    // than the ordinary "No debug session active" hint.
    for (const viewId of DEBUG_VIEW_IDS) {
      await app.page.click('[data-testid="panel-picker-trigger"]');
      await app.page.waitForSelector(`[data-panel-picker-option="${viewId}"]`);
      await app.page.click(`[data-panel-picker-option="${viewId}"]`);
      await app.page.waitForSelector(`[data-panel-id="${viewId}"] [data-debug-empty-state="unavailable"]`);
      const text = await app.page.$eval(`[data-panel-id="${viewId}"]`, (node) => node.textContent ?? "");
      assert.match(text, UNAVAILABLE_PATTERN, `${viewId} panel must explain the missing debugger`);
      assert.doesNotMatch(text, /No debug session active/, `${viewId} must not fall back to the no-session state`);
    }

    // None of the interactions above reached the debug IPC boundary. The
    // one exemption is debug_get_state: the store's mount-time backend sync
    // intentionally runs before the handshake resolves (default-available
    // avoids flicker on normal builds) and its failure is swallowed.
    const debugInvokes = await app.page.evaluate(() =>
      (window.__TCLD_TEST_INVOKES__ ?? [])
        .map((entry) => entry.cmd)
        .filter((cmd) => cmd.startsWith("debug_") && cmd !== "debug_get_state"),
    );
    assert.deepEqual(debugInvokes, [], "no debug IPC may be dispatched when the debugger is unavailable");
  } finally {
    await closeProductionPage(app);
  }
});

test("B6 default capabilities keep run controls and debug launch behavior unchanged", async () => {
  const app = await withProductionPage("/", {
    dialogPath: "/tmp/debug-open.tbc",
    debugPath: "/tmp/debug-script.tcl",
  });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /debug-open\.tbc/);

    // The handshake is requested during store bootstrap.
    await app.page.waitForFunction(() =>
      (window.__TCLD_TEST_INVOKES__ ?? []).some((entry) => entry.cmd === "get_app_capabilities"),
    );

    // Run cluster stays enabled with the normal tooltip.
    const run = await app.page.$eval('[data-actionbar-launch-item="run"]', (el) => ({
      disabled: (el as HTMLButtonElement).disabled,
      title: el.getAttribute("title"),
      unavailable: el.getAttribute("data-launch-unavailable"),
    }));
    assert.equal(run.disabled, false, "run control must stay enabled on debugger builds");
    assert.equal(run.title, "Run / Debug (F5)");
    assert.equal(run.unavailable, null);

    // Launch still flows through the debug IPC boundary.
    await app.page.click('[title="Run / Debug (F5)"]');
    await app.page.waitForSelector('.relative[data-design-surface="debug-paused-overlay"]');
    const invoked = await app.page.evaluate(() =>
      (window.__TCLD_TEST_INVOKES__ ?? []).map((entry) => entry.cmd),
    );
    assert.ok(invoked.includes("debug_launch"), "debug_launch must dispatch on debugger builds");

    // The capability empty state never renders on a debugger build — the
    // variables panel shows live session data.
    await openDebugTab(app.page, "variables");
    await waitForVisibleText(app.page, '[data-panel-id="variables"]', /argv/);
    assert.equal(
      await app.page.$('[data-debug-empty-state="unavailable"]'),
      null,
      "unavailable empty states must not render when the debugger is available",
    );
  } finally {
    await closeProductionPage(app);
  }
});
