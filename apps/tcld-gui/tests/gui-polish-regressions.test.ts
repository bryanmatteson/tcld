import assert from "node:assert/strict";
import test from "node:test";
import { closeProductionPage, withProductionPage } from "./browser-production-harness.ts";

test("palette keeps keyboard selection visible, respects context, and restores launcher focus", async () => {
  const handle = await withProductionPage("/");
  const { page } = handle;
  try {
    await page.click('[aria-label="Command Palette"]');
    await page.waitForSelector('[role="combobox"]');
    await page.keyboard.press("End");
    await page.waitForFunction(() => {
      const option = document.querySelector('[role="option"][aria-selected="true"]');
      const list = document.querySelector('[role="listbox"]');
      if (!option || !list) return false;
      const a = option.getBoundingClientRect(), b = list.getBoundingClientRect();
      return a.top >= b.top && a.bottom <= b.bottom + 1;
    });
    await page.keyboard.press("Tab");
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("role")), "combobox");
    await page.type('[role="combobox"]', "Step Into");
    assert.equal(await page.$$eval('[role="option"]', (options) => options.length), 0);
    await page.keyboard.press("Escape");
    await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "Command Palette");
    assert.deepEqual(handle.pageErrors, []);
  } finally { await closeProductionPage(handle); }
});

test("examples contain focus, close with Escape, and fit a narrow short window", async () => {
  const handle = await withProductionPage("/");
  const { page } = handle;
  try {
    await page.setViewport({ width: 480, height: 540 });
    await page.click('[data-testid="welcome-browse-examples"]');
    await page.waitForSelector('[aria-label="Browse example Tcl programs"]');
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("aria-label")), "Close examples");
    await page.keyboard.down("Shift");
    await page.keyboard.press("Tab");
    await page.keyboard.up("Shift");
    assert.equal(await page.evaluate(() => document.activeElement?.hasAttribute("data-example-id")), true);
    await page.keyboard.press("Tab");
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("aria-label")), "Close examples");
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true);
    await page.keyboard.press("Escape");
    await page.waitForSelector('[aria-label="Browse example Tcl programs"]', { hidden: true });
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("data-testid")), "welcome-browse-examples");
    assert.deepEqual(handle.pageErrors, []);
  } finally { await closeProductionPage(handle); }
});

test("loading preserves workspace, newer opens win, errors stay visible, and closed files reread disk", async () => {
  const handle = await withProductionPage("/");
  const { page } = handle;
  try {
    await page.evaluate(() => {
      const bridge = window.__TCLD_TEST_TAURI__!;
      const original = bridge.invoke!;
      let count = 0;
      bridge.openFileDialog = async () => `/tmp/file-${++count}.tbc`;
      const pending = new Map<string, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();
      bridge.invoke = async <T,>(command: string, args?: Record<string, unknown>): Promise<T> => {
        if (command !== "load_file") return original<T>(command, args);
        return new Promise<T>((resolve, reject) => pending.set(String(args?.path), { resolve: resolve as (value: unknown) => void, reject }));
      };
      Object.assign(window, {
        finishLoad(path: string, error?: string) {
          const request = pending.get(path)!;
          if (error) request.reject(new Error(error));
          else request.resolve(window.__TCLD_TEST_CREATE_LOAD_RESULT__!(path));
          pending.delete(path);
        },
      });
    });
    const finish = (path: string, error?: string) => page.evaluate((p, e) => {
      (window as unknown as { finishLoad: (path: string, error?: string) => void }).finishLoad(p, e);
    }, path, error);
    const open = () => page.click('[data-testid="topbar-open-another-file"]');
    await open();
    await page.waitForSelector('[role="status"]');
    await open();
    await finish("/tmp/file-2.tbc");
    await page.waitForSelector('[data-file-label="file-2.tbc"][data-active-file="true"]');
    await finish("/tmp/file-1.tbc");
    assert.equal(await page.$$eval('[data-file-id]', (tabs) => tabs.length), 1);
    await open();
    assert.ok(await page.$('[data-panel-id="instructions"]'), "workspace remains mounted while loading");
    await finish("/tmp/file-3.tbc", "Cannot read file-3.tbc");
    await page.waitForSelector('[role="alert"]');
    assert.match(await page.$eval('[role="alert"]', (el) => el.textContent!), /Cannot read/);
    await page.click('[aria-label="Dismiss file error"]');
    await open();
    await page.click('[aria-label="Close file-2.tbc"]');
    await finish("/tmp/file-4.tbc");
    await page.waitForSelector('[data-testid="welcome-open-file"]');
    assert.equal(await page.$$eval('[data-file-id]', (tabs) => tabs.length), 0);
    // Reopening an identical path must actually invoke the backend again.
    await page.evaluate(() => { window.__TCLD_TEST_TAURI__!.openFileDialog = async () => "/tmp/file-2.tbc"; });
    await open();
    await page.waitForSelector('[role="status"]');
    await finish("/tmp/file-2.tbc");
    await page.waitForSelector('[data-file-label="file-2.tbc"][data-active-file="true"]');
    assert.deepEqual(handle.pageErrors, []);
  } finally { await closeProductionPage(handle); }
});

test("saved-mark and navigation-history popup clicks execute before dismissing", async () => {
  const handle = await withProductionPage("/?parity=instructions");
  const { page } = handle;
  try {
    await page.waitForSelector('[data-panel-id="instructions"] [data-idx="0"]');
    await page.click('[data-panel-id="instructions"] [data-idx="0"]');
    await page.keyboard.press("m");
    await page.keyboard.press("a");
    await page.waitForSelector('[aria-label="1 saved marks"]');
    await page.click('[data-panel-id="instructions"] [data-idx="3"]');
    await page.click('[aria-label="1 saved marks"]');
    await page.waitForSelector('[aria-label="Saved marks"] [role="menuitem"]');
    await page.click('[aria-label="Saved marks"] [role="menuitem"]');
    await page.waitForFunction(() => document.querySelector('[data-status-selected-instruction]')?.getAttribute('data-status-selected-instruction') === "0");
    await page.click('[aria-label="History"]');
    await page.waitForSelector('[aria-label="Navigation history"] [role="menuitem"]');
    await page.click('[aria-label="Navigation history"] [role="menuitem"]');
    await page.waitForFunction(() => document.querySelector('[data-status-selected-instruction]')?.getAttribute('data-status-selected-instruction') === "3");
    assert.deepEqual(handle.pageErrors, []);
  } finally { await closeProductionPage(handle); }
});
