import assert from "node:assert/strict";
import test from "node:test";
import { mkdirSync } from "node:fs";
import { closeProductionPage, withProductionPage } from "./browser-production-harness.ts";

const screenshots = process.env.TCLD_USABILITY_SCREENSHOTS;
if (screenshots) mkdirSync(screenshots, { recursive: true });

test("panel discovery searches by purpose, supports keyboard focus, and preserves open panels", async () => {
  const app = await withProductionPage("/?parity=instructions");
  const { page } = app;
  const open = async () => {
    await page.click('[data-testid="panel-picker-trigger"]');
    await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "Find a panel");
  };
  try {
    await open();
    if (screenshots) await page.screenshot({ path: `${screenshots}/panel-picker.png` });
    await page.type('[aria-label="Find a panel"]', "runtime output");
    assert.equal(await page.$$eval('[data-panel-picker-option]', (items) => items.length), 1);
    await page.keyboard.press("ArrowDown");
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("data-panel-picker-option")), "console");
    await page.keyboard.press("Enter");
    await page.waitForSelector('[data-panel-id="console"]');
    await open();
    await page.type('[aria-label="Find a panel"]', "console");
    assert.match(await page.$eval('[data-panel-picker-option="console"]', (el) => el.textContent!), /Open$/);
    await page.keyboard.press("Enter");
    assert.equal(await page.$$eval('[data-panel-id="console"]', (items) => items.length), 1);
    await open();
    await page.type('[aria-label="Find a panel"]', "no-such-panel");
    await page.waitForSelector('.tcld-picker-empty');
    await page.keyboard.press("Escape");
    assert.equal(await page.evaluate(() => document.activeElement?.getAttribute("data-testid")), "panel-picker-trigger");
    assert.deepEqual(app.pageErrors, []);
  } finally { await closeProductionPage(app); }
});

test("back and forward navigate immediately and history supports keyboard selection", async () => {
  const app = await withProductionPage("/?parity=instructions");
  const { page } = app;
  try {
    await page.click('[data-panel-id="instructions"] [data-idx="0"]');
    await page.click('[data-panel-id="instructions"] [data-idx="3"]');
    await page.click('[aria-label="Back"]');
    await page.waitForFunction(() => document.querySelector('[data-status-selected-instruction]')?.getAttribute('data-status-selected-instruction') === "0");
    assert.equal(await page.$('[aria-label="Navigation history"]'), null);
    await page.click('[aria-label="Forward"]');
    await page.waitForFunction(() => document.querySelector('[data-status-selected-instruction]')?.getAttribute('data-status-selected-instruction') === "3");
    await page.click('[aria-label="History"]');
    await page.waitForFunction(() => document.activeElement?.getAttribute("role") === "menuitem");
    await page.keyboard.press("Enter");
    await page.waitForFunction(() => document.querySelector('[data-status-selected-instruction]')?.getAttribute('data-status-selected-instruction') === "0");
    if (screenshots) await page.screenshot({ path: `${screenshots}/workspace.png` });
    assert.deepEqual(app.pageErrors, []);
  } finally { await closeProductionPage(app); }
});

test("welcome offers entry points and palette ranks common actions with multiword search", async () => {
  const app = await withProductionPage("/");
  const { page } = app;
  try {
    await page.waitForSelector('[data-testid="welcome-debug"]');
    if (screenshots) await page.screenshot({ path: `${screenshots}/welcome.png` });
    await page.click('[aria-label="Command Palette"]');
    await page.waitForSelector('[role="combobox"]');
    assert.match(await page.$eval('[role="option"]', (el) => el.textContent!), /Open File/);
    await page.type('[role="combobox"]', "debug layout");
    assert.ok(await page.$$eval('[role="option"]', (items) => items.some((el) => el.textContent?.includes("Layout: Debug"))));
    await page.keyboard.press("Escape");
    await page.waitForSelector('[aria-label="Command palette"]', { hidden: true });
    await page.setViewport({ width: 600, height: 650 });
    assert.equal(await page.$eval('.tcld-no-file-tab', (el) =>
      Array.from(el.children).every((child) => child.getBoundingClientRect().height <= el.getBoundingClientRect().height)), true);
    if (screenshots) await page.screenshot({ path: `${screenshots}/welcome-compact.png` });
    await page.click('[data-testid="panel-picker-trigger"]');
    await page.waitForFunction(() => document.activeElement?.getAttribute("aria-label") === "Find a panel");
    const bounds = await page.$eval('[aria-label="Choose a panel"]', (el) => {
      const rect = el.getBoundingClientRect();
      const list = el.querySelector('.tcld-panel-picker-list')!;
      return { fits: rect.left >= 0 && rect.right <= innerWidth && rect.bottom <= innerHeight, scrolls: list.scrollHeight > list.clientHeight };
    });
    assert.deepEqual(bounds, { fits: true, scrolls: true });
    if (screenshots) await page.screenshot({ path: `${screenshots}/panel-picker-compact.png` });
    assert.deepEqual(app.pageErrors, []);
  } finally { await closeProductionPage(app); }
});

test("welcome hides debug entry when runtime does not support debugging", async () => {
  const app = await withProductionPage("/", { debuggerAvailable: false });
  try {
    await app.page.waitForSelector('[data-testid="welcome-open-file"]');
    assert.equal(await app.page.$('[data-testid="welcome-debug"]'), null);
  } finally { await closeProductionPage(app); }
});
