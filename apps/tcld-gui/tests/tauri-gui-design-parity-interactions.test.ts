import assert from "node:assert/strict";
import test from "node:test";
import {
  closeProductionPage,
  pressModShortcut,
  waitForVisibleText,
  withProductionPage,
} from "./browser-production-harness.ts";

type RectSnapshot = {
  left: number;
  right: number;
  top: number;
  bottom: number;
  width: number;
  height: number;
};

function overlaps(a: RectSnapshot, b: RectSnapshot): boolean {
  return a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
}

const PRODUCT_VIEW_IDS = [
  "instructions",
  "literals",
  "decompile",
  "comparison",
  "diagnostics",
  "provenance",
  "project",
  "detail",
  "exceptions",
  "graph",
  "summary",
  "variables",
  "stack",
  "console",
  "breakpoints",
  "watchpoints",
  "commandResolver",
];

async function activateWorkspaceView(
  page: Awaited<ReturnType<typeof withProductionPage>>["page"],
  viewId: string,
) {
  const inlineTab = await page.$(`[data-tab-view-id="${viewId}"]`);
  if (inlineTab) {
    await inlineTab.click();
  } else {
    const overflowTriggers = await page.$$('[data-tab-overflow-trigger="true"]');
    let selected = false;
    for (const trigger of overflowTriggers) {
      await trigger.click();
      const overflowItem = await page.$(`[data-overflow-tab-id="${viewId}"] [role="menuitem"]`);
      if (overflowItem) {
        await overflowItem.click();
        selected = true;
        break;
      }
      await page.keyboard.press("Escape");
    }
    assert.ok(selected, `${viewId} is reachable through an inline or overflow tab`);
  }
  await page.waitForSelector(`[data-panel-id="${viewId}"]`);
}

async function forcePanelWidthAndMeasure(
  page: Awaited<ReturnType<typeof withProductionPage>>["page"],
  panelId: string,
  width: number,
) {
  await page.evaluate((targetPanelId, targetWidth) => {
    const panel = document.querySelector<HTMLElement>(`[data-panel-id="${targetPanelId}"]`);
    if (!panel) throw new Error(`missing panel ${targetPanelId}`);
    const leaf = panel.closest<HTMLElement>("[data-leaf-id]");
    if (!leaf) throw new Error(`missing leaf for panel ${targetPanelId}`);
    for (const node of [leaf, panel]) {
      node.style.width = `${targetWidth}px`;
      node.style.minWidth = `${targetWidth}px`;
      node.style.maxWidth = `${targetWidth}px`;
      node.style.flexBasis = `${targetWidth}px`;
    }
  }, panelId, width);
  await page.evaluate(() => new Promise<void>((resolve) => {
    requestAnimationFrame(() => requestAnimationFrame(() => resolve()));
  }));
  return page.evaluate((targetPanelId) => {
    const toRect = (rect: DOMRect): RectSnapshot => ({
      left: rect.left,
      right: rect.right,
      top: rect.top,
      bottom: rect.bottom,
      width: rect.width,
      height: rect.height,
    });
    const panel = document.querySelector<HTMLElement>(`[data-panel-id="${targetPanelId}"]`);
    const header = panel?.querySelector<HTMLElement>('[data-design-surface="panel-header"]');
    const tabBar = header?.querySelector<HTMLElement>('[data-design-surface="adaptive-tab-bar"]');
    const actions = header?.querySelector<HTMLElement>('[data-design-surface="panel-header-actions"]');
    if (!panel || !header || !tabBar || !actions) {
      throw new Error(`missing adaptive header parts for ${targetPanelId}`);
    }
    return {
      panel: toRect(panel.getBoundingClientRect()),
      header: toRect(header.getBoundingClientRect()),
      tabBar: toRect(tabBar.getBoundingClientRect()),
      actions: toRect(actions.getBoundingClientRect()),
      tabs: [...tabBar.querySelectorAll<HTMLElement>('[data-design-surface="singleton-tab"]')].map((tab) => ({
        viewId: tab.getAttribute("data-tab-view-id"),
        mode: tab.getAttribute("data-tab-mode"),
        text: tab.textContent ?? "",
      })),
      // "+N" spill button: tabs that no longer fit inline move into the
      // overflow menu instead of degrading in place.
      overflowText: header.querySelector<HTMLElement>('[data-tab-overflow-trigger="true"]')?.textContent?.trim() ?? null,
    };
  }, panelId);
}

test("V3 Welcome open/drop/recents drive the rendered app store and workspace chrome", async () => {
  const open = await withProductionPage("/", { dialogPath: "/tmp/demo.tbc" });
  try {
    await open.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(open.page, '[data-design-surface="top-tab-strip"]', /demo\.tbc/);
    await waitForVisibleText(open.page, '[data-design-surface="status-bar"]', /loaded demo\.tbc/);
    await open.page.waitForSelector('.tcld-main-toolbar[data-design-surface="action-bar"]');
    await open.page.waitForSelector('[data-panel-id="instructions"]');
    await open.page.waitForSelector('[data-panel-id="decompile"]');
    await open.page.waitForSelector('[data-panel-id="detail"]');

    const openState = await open.page.evaluate(() => ({
      recents: JSON.parse(localStorage.getItem("tcld-recent-files") ?? "[]"),
      invokes: window.__TCLD_TEST_INVOKES__ ?? [],
      chrome: {
        tabs: document.querySelector('.tcld-file-header[data-design-surface="top-tab-strip"]')?.textContent ?? "",
        actions: document.querySelector('.tcld-main-toolbar[data-design-surface="action-bar"]')?.textContent ?? "",
        status: document.querySelector('.tcld-status-bar[data-design-surface="status-bar"]')?.textContent ?? "",
      },
    }));
    assert.equal(openState.recents[0].path, "/tmp/demo.tbc");
    assert.equal(openState.recents[0].instructionCount, 4);
    assert.equal(openState.recents[0].tclVersion, "8.6");
    assert.equal(openState.invokes.some((call) => call.cmd === "load_file" && call.args?.path === "/tmp/demo.tbc"), true);
    assert.match(openState.chrome.tabs, /demo\.tbc/);
    assert.match(openState.chrome.actions, /LAUNCH/);
    assert.match(openState.chrome.actions, /Run/);
    assert.match(openState.chrome.status, /loaded demo\.tbc/);
  } finally {
    await closeProductionPage(open);
  }

  const drop = await withProductionPage("/");
  try {
    await drop.page.evaluate(() => {
      const shell = document.querySelector('[data-design-surface="app-shell"]');
      if (!shell) throw new Error("missing app shell");
      const file = new File(["puts hi"], "dropped.tcl", { type: "text/x-tcl" }) as File & { path?: string };
      Object.defineProperty(file, "path", { value: "/tmp/dropped.tcl" });
      const dataTransfer = new DataTransfer();
      dataTransfer.items.add(file);
      shell.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer }));
    });
    await drop.page.waitForFunction(() =>
      document.querySelector('[data-parity-state="welcome"]')?.getAttribute("data-drop-active") === "true"
      && document.querySelector('[data-design-surface="app-shell"]')?.classList.contains("drop-zone-active")
    );
    await drop.page.evaluate(() => {
      const shell = document.querySelector('[data-design-surface="app-shell"]');
      if (!shell) throw new Error("missing app shell");
      const file = new File(["puts hi"], "dropped.tcl", { type: "text/x-tcl" }) as File & { path?: string };
      Object.defineProperty(file, "path", { value: "/tmp/dropped.tcl" });
      const dataTransfer = new DataTransfer();
      dataTransfer.items.add(file);
      shell.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer }));
    });
    await waitForVisibleText(drop.page, '[data-design-surface="top-tab-strip"]', /dropped\.tcl/);
    const dropState = await drop.page.evaluate(() => ({
      invokes: window.__TCLD_TEST_INVOKES__ ?? [],
      recents: JSON.parse(localStorage.getItem("tcld-recent-files") ?? "[]"),
    }));
    assert.equal(dropState.recents[0].path, "/tmp/dropped.tcl");
    assert.equal(dropState.recents[0].instructionCount, 4);
    assert.equal(dropState.recents[0].tclVersion, "8.6");
    const dropInvokes = dropState.invokes;
    assert.equal(dropInvokes.some((call) => call.cmd === "load_file" && call.args?.path === "/tmp/dropped.tcl"), true);
  } finally {
    await closeProductionPage(drop);
  }

  const badDrop = await withProductionPage("/");
  try {
    await badDrop.page.evaluate(() => {
      const shell = document.querySelector('[data-design-surface="app-shell"]');
      if (!shell) throw new Error("missing app shell");
      const missingPath = new File(["puts hi"], "host-only.tbc", { type: "application/octet-stream" });
      const missingPathTransfer = new DataTransfer();
      missingPathTransfer.items.add(missingPath);
      shell.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: missingPathTransfer }));
    });
    await waitForVisibleText(badDrop.page, '[data-design-surface="status-bar"]', /file path unavailable from host/);

    await badDrop.page.evaluate(() => {
      const shell = document.querySelector('[data-design-surface="app-shell"]');
      if (!shell) throw new Error("missing app shell");
      const unsupported = new File(["nope"], "notes.txt", { type: "text/plain" });
      const unsupportedTransfer = new DataTransfer();
      unsupportedTransfer.items.add(unsupported);
      shell.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: unsupportedTransfer }));
    });
    await waitForVisibleText(badDrop.page, '[data-design-surface="status-bar"]', /unsupported file type/);
  } finally {
    await closeProductionPage(badDrop);
  }

  const recent = await withProductionPage("/");
  try {
    await recent.page.evaluate(() => {
      localStorage.setItem("tcld-recent-files", JSON.stringify([
        { path: "/tmp/recent.tbc", filename: "recent.tbc", mode: "open", timestamp: Date.now() },
      ]));
      location.reload();
    });
    await recent.page.waitForSelector('[data-recent-file-path="/tmp/recent.tbc"]');
    await waitForVisibleText(recent.page, '[data-recent-file-path="/tmp/recent.tbc"]', /inst unknown/);
    await waitForVisibleText(recent.page, '[data-recent-file-path="/tmp/recent.tbc"]', /Tcl unknown/);
    await recent.page.click('[data-recent-file-path="/tmp/recent.tbc"]');
    await waitForVisibleText(recent.page, '[data-design-surface="top-tab-strip"]', /recent\.tbc/);

    await recent.page.goto(`${recent.baseUrl}/`, { waitUntil: "networkidle0" });
    await recent.page.evaluate(() => {
      localStorage.setItem("tcld-recent-files", JSON.stringify([
        { path: "/tmp/clear-me.tbc", filename: "clear-me.tbc", mode: "open", timestamp: Date.now() },
      ]));
      location.reload();
    });
    await recent.page.waitForSelector('[data-testid="recent-clear"]');
    await recent.page.click('[data-testid="recent-clear"]');
    const afterClear = await recent.page.evaluate(() => ({
      storage: localStorage.getItem("tcld-recent-files"),
      hasRecent: Boolean(document.querySelector('[data-recent-file-path]')),
    }));
    assert.equal(afterClear.storage, null);
    assert.equal(afterClear.hasRecent, false);

    // Browse examples is now wired to the production backend: the button is
    // live, opens the bundled-examples dialog, and picking an example loads
    // it through the real `load_example` IPC boundary into the workspace.
    await recent.page.goto(`${recent.baseUrl}/`, { waitUntil: "networkidle0" });
    const browseExamples = await recent.page.evaluate(() => {
      const button = document.querySelector<HTMLButtonElement>('[data-testid="welcome-browse-examples"]');
      return button
        ? {
          disabled: button.disabled,
          ariaDisabled: button.getAttribute("aria-disabled"),
        }
        : null;
    });
    assert.deepEqual(browseExamples, { disabled: false, ariaDisabled: null });

    await recent.page.click('[data-testid="welcome-browse-examples"]');
    await recent.page.waitForSelector('[role="dialog"][aria-label="Browse example Tcl programs"]');
    await recent.page.waitForSelector('[data-example-id="hello"]');
    await recent.page.click('[data-example-id="hello"]');
    await waitForVisibleText(recent.page, '[data-design-surface="top-tab-strip"]', /hello\.tcl/);
    const exampleInvokes = await recent.page.evaluate(() => window.__TCLD_TEST_INVOKES__ ?? []);
    const loadExampleCall = exampleInvokes.find((call) => call.cmd === "load_example");
    assert.ok(loadExampleCall, "picking an example must dispatch load_example through the Tauri boundary");
    assert.equal(loadExampleCall?.args?.id, "hello");
  } finally {
    await closeProductionPage(recent);
  }
});

test("V3 top bar empty tab, plus opener, command search, and read-only tab state use production wiring", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/topbar-first.tbc" });
  try {
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /No file loaded/);
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /Open Tcl bytecode or source/);

    const emptyAffordance = await app.page.$eval('[data-testid="topbar-empty-open-file"]', (button) => ({
      tag: button.tagName,
      label: button.getAttribute("aria-label"),
      disabled: (button as HTMLButtonElement).disabled,
    }));
    assert.deepEqual(emptyAffordance, {
      tag: "BUTTON",
      label: "Open Tcl bytecode or source to begin",
      disabled: false,
    });

    await app.page.click('[data-testid="topbar-empty-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /topbar-first\.tbc/);

    const loadedTab = await app.page.$eval('[data-file-label="topbar-first.tbc"]', (tab) => ({
      readOnly: tab.getAttribute("data-readonly-document"),
      dirty: tab.getAttribute("data-document-dirty"),
      hasDirtyDot: Boolean(tab.querySelector(".tcld-tab-dirty-dot")),
      text: tab.textContent ?? "",
    }));
    assert.equal(loadedTab.readOnly, "true");
    assert.equal(loadedTab.dirty, "false");
    assert.equal(loadedTab.hasDirtyDot, false);
    assert.match(loadedTab.text, /Tcl 8\.6/);
    assert.match(loadedTab.text, /4 inst/);
    assert.match(loadedTab.text, /2 lit/);

    // "+" opens the file dialog again. Same-path reloads are served from the
    // tauri-query cache (no duplicate load_file round-trip), so point the
    // dialog at a second file and assert the multi-document open path.
    await app.page.waitForSelector('[data-testid="topbar-open-another-file"]');
    await app.page.evaluate(() => {
      if (window.__TCLD_TEST_TAURI__) {
        window.__TCLD_TEST_TAURI__.openFileDialog = async () => "/tmp/topbar-second.tbc";
      }
    });
    await app.page.click('[data-testid="topbar-open-another-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /topbar-second\.tbc/);
    await app.page.waitForFunction(() =>
      (window.__TCLD_TEST_INVOKES__ ?? []).some((call) => call.cmd === "load_file" && call.args?.path === "/tmp/topbar-second.tbc")
    );
    // Both documents stay open as read-only tabs.
    await app.page.waitForSelector('[data-file-label="topbar-first.tbc"]');
    await app.page.waitForSelector('[data-file-label="topbar-second.tbc"]');

    // The "Search & commands" pill became the labelled command-palette icon
    // in the titlebar trail; it still opens the palette's command input.
    await app.page.click(".tcld-command-icon");
    await app.page.waitForSelector('input[placeholder="Type a command..."]');
    const paletteAffordance = await app.page.$eval(".tcld-command-icon", (button) => ({
      title: button.getAttribute("title"),
      ariaLabel: button.getAttribute("aria-label"),
    }));
    assert.deepEqual(paletteAffordance, { title: "Command Palette (⌘P)", ariaLabel: "Command Palette" });

    await app.page.keyboard.press("Escape");
    await app.page.waitForFunction(() => !document.querySelector('input[placeholder="Type a command..."]'));

    const rightChrome = await app.page.evaluate(() => ({
      panelPicker: Boolean(document.querySelector('[data-testid="panel-picker-trigger"]')),
      commandIcon: Boolean(document.querySelector(".tcld-command-icon")),
      plusRect: (() => {
        const rect = document.querySelector<HTMLElement>('[data-testid="topbar-open-another-file"]')?.getBoundingClientRect();
        return rect ? { left: rect.left, right: rect.right } : null;
      })(),
      panelPickerRect: (() => {
        const rect = document.querySelector<HTMLElement>('[data-testid="panel-picker-trigger"]')?.getBoundingClientRect();
        return rect ? { left: rect.left, right: rect.right } : null;
      })(),
    }));
    assert.equal(rightChrome.panelPicker, true);
    assert.equal(rightChrome.commandIcon, true);
    assert.ok(rightChrome.plusRect);
    assert.ok(rightChrome.panelPickerRect);
    assert.ok(rightChrome.plusRect.right < rightChrome.panelPickerRect.left, "plus/open button stays inside the tab strip before right-side chrome");
  } finally {
    await closeProductionPage(app);
  }
});

test("V4 Mosaic add, swap, and split preserve singleton workspace rules", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/layout-actions.tbc" });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /layout-actions\.tbc/);

    // Split moved from an inline header button to the workspace keyboard
    // command (Mod+\ splits the focused panel to the right edge). Mod is
    // the platform-primary modifier — Meta on macOS, Ctrl elsewhere — so
    // dispatch through the harness helper instead of hardcoding Meta.
    const leavesBeforeSplit = await app.page.$$eval("[data-leaf-id]", (nodes) => nodes.length);
    await app.page.click('[data-panel-id="decompile"]');
    await pressModShortcut(app.page, "\\");
    await app.page.waitForFunction((before) => document.querySelectorAll("[data-leaf-id]").length > before, {}, leavesBeforeSplit);
    let viewCounts = await app.page.$$eval("[data-panel-id]", (nodes) =>
      nodes.reduce<Record<string, number>>((counts, node) => {
        const id = node.getAttribute("data-panel-id") ?? "";
        counts[id] = (counts[id] ?? 0) + 1;
        return counts;
      }, {}),
    );
    assert.equal(viewCounts.decompile, 1, "split preserves singleton decompile view");

    // Add view moved into the unified panel-header ⋯ menu (add-view submenu).
    await app.page.click('[data-panel-id="instructions"] [data-panel-header-menu-trigger="true"]');
    await app.page.waitForSelector('[data-panel-menu-item="add-view"]');
    await app.page.click('[data-panel-menu-item="add-view"]');
    await app.page.waitForSelector('[data-panel-submenu-popover][data-submenu-of="add-view"]', { visible: true });
    const addMenu = await app.page.evaluate(() => {
      const options = [...document.querySelectorAll<HTMLElement>("[data-panel-submenu-item]")]
        .map((option) => option.getAttribute("data-panel-submenu-item")?.replace(/^add-view-/, "") ?? "");
      const rect = document.querySelector<HTMLElement>('[data-panel-submenu-popover][data-submenu-of="add-view"]')?.getBoundingClientRect();
      return {
        options,
        rect: rect ? {
          left: rect.left,
          right: rect.right,
          top: rect.top,
          bottom: rect.bottom,
        } : null,
        viewport: { width: window.innerWidth, height: window.innerHeight },
      };
    });
    assert.ok(addMenu.rect, "add-view submenu renders in the browser");
    assert.ok(addMenu.rect.left >= 0 && addMenu.rect.right <= addMenu.viewport.width, "add menu stays inside viewport horizontally");
    assert.ok(addMenu.rect.top >= 0 && addMenu.rect.bottom <= addMenu.viewport.height, "add menu stays inside viewport vertically");
    for (const openView of ["instructions", "literals", "decompile", "comparison", "detail", "diagnostics"]) {
      assert.equal(addMenu.options.includes(openView), false, `add menu must exclude already open ${openView}`);
    }
    await app.page.keyboard.press("Escape");
    await app.page.waitForFunction(() => !document.querySelector('[data-panel-submenu-popover][data-submenu-of="add-view"]'));
    await app.page.keyboard.press("Escape");
    await app.page.waitForFunction(() => !document.querySelector('[data-panel-header-menu="true"]'));

    await app.page.click('[data-panel-id="instructions"] [data-panel-header-menu-trigger="true"]');
    await app.page.waitForSelector('[data-panel-menu-item="add-view"]');
    await app.page.click('[data-panel-menu-item="add-view"]');
    await app.page.waitForSelector('[data-panel-submenu-item="add-view-exceptions"]', { visible: true });
    await app.page.click('[data-panel-submenu-item="add-view-exceptions"]');
    await app.page.waitForSelector('[data-panel-id="exceptions"]');
    viewCounts = await app.page.$$eval("[data-panel-id]", (nodes) =>
      nodes.reduce<Record<string, number>>((counts, node) => {
        const id = node.getAttribute("data-panel-id") ?? "";
        counts[id] = (counts[id] ?? 0) + 1;
        return counts;
      }, {}),
    );
    assert.equal(viewCounts.exceptions, 1, "add view creates one singleton exceptions view");

    await app.page.click('[data-panel-id="decompile"] [data-panel-header-menu-trigger="true"]');
    await app.page.waitForSelector('[data-panel-menu-item="add-view"]');
    await app.page.click('[data-panel-menu-item="add-view"]');
    await app.page.waitForSelector('[data-panel-submenu-popover][data-submenu-of="add-view"]', { visible: true });
    const addAfterExceptions = await app.page.$$eval("[data-panel-submenu-item]", (options) =>
      options.map((option) => option.getAttribute("data-panel-submenu-item")?.replace(/^add-view-/, "") ?? ""),
    );
    assert.equal(addAfterExceptions.includes("exceptions"), false, "added view is removed from add availability elsewhere");
    // Click-outside closes the menu (no dedicated backdrop element anymore).
    // The status bar sits well clear of the floating menu/submenu popovers.
    await app.page.click(".tcld-status-bar");
    await app.page.waitForFunction(() => !document.querySelector('[data-panel-header-menu="true"]'));

    await app.page.click('[data-panel-id="exceptions"] [data-panel-header-menu-trigger="true"]');
    await app.page.waitForSelector('[data-panel-menu-item="swap-view"]');
    await app.page.click('[data-panel-menu-item="swap-view"]');
    await app.page.waitForSelector('[data-panel-submenu-popover][data-submenu-of="swap-view"]', { visible: true });
    const swapOptions = await app.page.$$eval("[data-panel-submenu-item]", (options) =>
      options.map((option) => option.getAttribute("data-panel-submenu-item")?.replace(/^swap-view-/, "") ?? ""),
    );
    for (const openView of ["instructions", "literals", "decompile", "comparison", "detail", "diagnostics", "exceptions"]) {
      assert.equal(swapOptions.includes(openView), false, `swap menu must exclude open ${openView}`);
    }
    assert.equal(swapOptions.includes("summary"), true, "swap menu keeps unopened singleton choices available");
    await app.page.click('[data-panel-submenu-item="swap-view-summary"]');
    await app.page.waitForSelector('[data-panel-id="summary"]');
    await app.page.waitForFunction(() => !document.querySelector('[data-panel-id="exceptions"]'));
    viewCounts = await app.page.$$eval("[data-panel-id]", (nodes) =>
      nodes.reduce<Record<string, number>>((counts, node) => {
        const id = node.getAttribute("data-panel-id") ?? "";
        counts[id] = (counts[id] ?? 0) + 1;
        return counts;
      }, {}),
    );
    assert.equal(viewCounts.decompile, 1, "split preserves singleton decompile view");
    assert.equal(viewCounts.summary, 1, "split does not duplicate the swapped summary view");
  } finally {
    await closeProductionPage(app);
  }
});

test("V4 local panel view picker shows the all-views-used empty state and closes by backdrop or Escape", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/all-views.tbc" });
  try {
    await app.page.evaluate((viewIds) => {
      localStorage.setItem("tcld-layout-v3", JSON.stringify({
        tree: "leaf-all",
        stacks: {
          "leaf-all": {
            leafId: "leaf-all",
            views: viewIds,
            activeView: "instructions",
          },
        },
      }));
      location.reload();
    }, PRODUCT_VIEW_IDS);

    await app.page.waitForSelector('[data-testid="welcome-open-file"]');
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /all-views\.tbc/);
    await app.page.waitForSelector('[data-panel-id="instructions"]');

    // Add view lives in the ⋯ panel-header menu; with every view already
    // open the submenu renders its explicit empty state instead of options.
    await app.page.click('[data-panel-id="instructions"] [data-panel-header-menu-trigger="true"]');
    await app.page.waitForSelector('[data-panel-menu-item="add-view"]');
    await app.page.click('[data-panel-menu-item="add-view"]');
    await waitForVisibleText(app.page, '[data-panel-submenu-popover][data-submenu-of="add-view"]', /All views are already open/);
    const addOptions = await app.page.$$("[data-panel-submenu-item]");
    assert.equal(addOptions.length, 0, "all-open add menu should not render unavailable choices");

    // Click-outside closes both submenu and menu (the status bar sits well
    // clear of the floating popovers).
    await app.page.click(".tcld-status-bar");
    await app.page.waitForFunction(() => !document.querySelector('[data-panel-header-menu="true"]'));

    await app.page.click('[data-panel-id="instructions"] [data-panel-header-menu-trigger="true"]');
    await app.page.waitForSelector('[data-panel-menu-item="add-view"]');
    await app.page.click('[data-panel-menu-item="add-view"]');
    await waitForVisibleText(app.page, '[data-panel-submenu-popover][data-submenu-of="add-view"]', /All views are already open/);
    await app.page.keyboard.press("Escape");
    await app.page.waitForFunction(() => !document.querySelector('[data-panel-submenu-popover][data-submenu-of="add-view"]'));
    await app.page.keyboard.press("Escape");
    await app.page.waitForFunction(() => !document.querySelector('[data-panel-header-menu="true"]'));
  } finally {
    await closeProductionPage(app);
  }
});

test("V6 Inspector follows the selected instruction without summary sections", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/inspector.tbc", debugPath: "/tmp/inspector-debug.tcl" });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /inspector\.tbc/);
    await app.page.waitForSelector('[data-panel-id="detail"] [data-inspector-summary="true"]');
    await waitForVisibleText(app.page, '[data-panel-id="detail"] [data-inspector-identity="true"]', /push1/);

    // Variables, call stack, diagnostics, and literals live in their own
    // views; the Inspector carries no accordion sections or summary copies.
    const summaryMarkers = await app.page.$$(
      '[data-panel-id="detail"] :is([data-inspector-section], [data-inspector-literals-summary], [data-inspector-diagnostics-summary], [data-inspector-debug-summary])',
    );
    assert.equal(summaryMarkers.length, 0);

    await app.page.click('[data-panel-id="instructions"] [data-idx="2"]');
    await waitForVisibleText(app.page, '[data-panel-id="detail"] [data-inspector-identity="true"]', /invokeStk1/);
    await waitForVisibleText(app.page, '[data-panel-id="detail"] [data-inspector-field="control-flow"]', /Continues at 0x0003/);
    await waitForVisibleText(app.page, '[data-panel-id="detail"] [data-inspector-field="stack-depth"]', /2 → 1 \(-1\)/);
    await waitForVisibleText(app.page, '[data-panel-id="detail"] [data-inspector-field="pops"]', /"set", "x"/);

    await app.page.click('[data-panel-id="instructions"] [data-idx="3"]');
    await waitForVisibleText(app.page, '[data-panel-id="detail"] [data-inspector-field="control-flow"]', /no following instruction/);
    const idleText = await app.page.$eval('[data-panel-id="detail"]', (node) => node.textContent ?? "");
    assert.doesNotMatch(idleText, /argv|sample_proc/, "inspector must not fabricate debug rows while idle");

    // Issues stays one tab away in the Inspector's pane.
    await app.page.click('[data-tab-view-id="diagnostics"]');
    await waitForVisibleText(app.page, '[data-panel-id="diagnostics"]', /unsupportedDemo/);
  } finally {
    await closeProductionPage(app);
  }
});

test("V4 adaptive tab strip excludes action clusters at 260px and 220px", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/narrow-tabs.tbc" });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /narrow-tabs\.tbc/);

    // Rendered badge contract: a stacked tab renders the production badge
    // value (the loaded instruction count), never its shortcut key hint.
    await app.page.waitForSelector('[data-tab-view-id="instructions"] [data-tab-badge="true"]');
    const badgeEvidence = await app.page.evaluate(() => {
      const tab = document.querySelector('[data-tab-view-id="instructions"]');
      return {
        badgeText: tab?.querySelector('[data-tab-badge="true"]')?.textContent?.trim() ?? null,
        keyHint: tab?.getAttribute("data-tab-key-hint") ?? null,
      };
    });
    assert.equal(badgeEvidence.badgeText, "4", "instructions tab badge shows the loaded instruction count");
    assert.notEqual(badgeEvidence.badgeText, badgeEvidence.keyHint, "tab badge must not render the shortcut key hint");

    // Narrow-strip contract (overflow model): instead of demoting tabs to
    // icon-only in place, tabs that don't fit spill into the "+N" overflow
    // menu. Inline tabs stay readable, the actions cluster never overlaps or
    // clips, and every stacked view stays reachable (inline + overflow).
    const overflowCount = (text: string | null) => {
      const match = /^\+(\d+)$/.exec(text ?? "");
      return match ? Number(match[1]) : 0;
    };
    for (const panelId of ["decompile", "instructions"]) {
      const wide = await forcePanelWidthAndMeasure(app.page, panelId, 620);
      const totalViews = wide.tabs.length + overflowCount(wide.overflowText);
      assert.ok(totalViews >= 2, `${panelId} must be a stacked leaf with at least two views`);

      const at260 = await forcePanelWidthAndMeasure(app.page, panelId, 260);
      assert.equal(overlaps(at260.tabBar, at260.actions), false, `${panelId} tabs and actions overlap at 260px`);
      assert.ok(at260.actions.right <= at260.panel.right + 1, `${panelId} actions clip past panel at 260px`);
      assert.ok(at260.tabs.length >= 1, `${panelId} keeps at least one tab inline at 260px`);
      assert.ok(
        at260.tabs.some((tab) => tab.viewId === panelId),
        `${panelId} keeps its active tab inline at 260px`,
      );
      assert.equal(
        at260.tabs.length + overflowCount(at260.overflowText),
        totalViews,
        `${panelId} keeps every stacked view reachable at 260px (inline + overflow)`,
      );

      const at220 = await forcePanelWidthAndMeasure(app.page, panelId, 220);
      assert.equal(overlaps(at220.tabBar, at220.actions), false, `${panelId} tabs and actions overlap at 220px`);
      assert.ok(at220.actions.right <= at220.panel.right + 1, `${panelId} actions clip past panel at 220px`);
      assert.ok(at220.tabs.length >= 1, `${panelId} keeps at least the active tab inline at 220px`);
      assert.equal(
        at220.tabs.length + overflowCount(at220.overflowText),
        totalViews,
        `${panelId} keeps every stacked view reachable at the 220px minimum`,
      );
      assert.equal(Math.round(at220.header.height), Math.round(at260.header.height), `${panelId} header height changes between tab modes`);
    }
  } finally {
    await closeProductionPage(app);
  }
});

test("V4 workspace presets, singleton add/close, drag/drop, resize minimum, dirty state, and save layout use production Mosaic paths", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/workspace.tbc" });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /workspace\.tbc/);

    await app.page.click('[data-testid="layout-preset-trigger"]');
    await app.page.waitForSelector('[data-testid="layout-preset-menu"]');
    await waitForVisibleText(app.page, '[data-testid="layout-builtins-group"]', /Presets/);
    await app.page.waitForSelector('[data-layout-menu-kind="built-in"][data-layout-menu-id="cockpit"]');
    await app.page.waitForSelector('[data-layout-menu-kind="built-in"][data-layout-menu-id="runbook"]');
    await waitForVisibleText(app.page, '[data-testid="layout-preset-menu"]', /Cockpit/);
    await waitForVisibleText(app.page, '[data-testid="layout-preset-menu"]', /Runbook/);
    await app.page.click('[data-layout-menu-kind="built-in"][data-layout-menu-id="cockpit"]');
    await app.page.waitForSelector('[data-panel-id="instructions"]');
    await app.page.waitForSelector('[data-panel-id="decompile"]');
    await app.page.waitForSelector('[data-panel-id="detail"]');
    await app.page.waitForSelector('[data-tab-view-id="graph"]');
    await app.page.click('[data-testid="layout-preset-trigger"]');
    await app.page.waitForSelector('[data-testid="layout-preset-menu"]');
    await app.page.click('[data-layout-menu-kind="built-in"][data-layout-menu-id="runbook"]');
    await app.page.waitForSelector('[data-panel-id="decompile"]');
    await app.page.waitForSelector('[data-panel-id="instructions"]');
    await app.page.waitForSelector('[data-panel-id="variables"]');
    await app.page.waitForSelector('[data-panel-id="console"]');
    await activateWorkspaceView(app.page, "watchpoints");
    await app.page.click('[data-testid="layout-preset-trigger"]');
    await app.page.waitForSelector('[data-testid="layout-preset-menu"]');
    await app.page.click('[data-testid="layout-menu-backdrop"]');
    await app.page.waitForFunction(() => !document.querySelector('[data-testid="layout-preset-menu"]'));
    await app.page.click('[data-testid="layout-preset-trigger"]');
    await app.page.waitForSelector('[data-testid="layout-preset-menu"]');
    await app.page.keyboard.press("Escape");
    await app.page.waitForFunction(() => !document.querySelector('[data-testid="layout-preset-menu"]'));
    await app.page.focus('[data-testid="layout-preset-trigger"]');
    await app.page.keyboard.press("Enter");
    await app.page.waitForSelector('[data-testid="layout-preset-menu"]');
    await app.page.waitForFunction(() =>
      document.activeElement?.getAttribute("role") === "menuitem"
      && document.activeElement?.getAttribute("data-layout-menu-id") === "default"
    );
    await app.page.keyboard.press("ArrowDown");
    await app.page.waitForFunction(() =>
      document.activeElement?.getAttribute("role") === "menuitem"
      && document.activeElement?.getAttribute("data-layout-menu-id") === "flow"
    );
    await app.page.keyboard.press("Enter");
    await app.page.waitForSelector('[data-panel-id="graph"]');
    await app.page.waitForSelector('[data-panel-id="decompile"]');
    await app.page.waitForSelector('[data-panel-id="detail"]');
    await app.page.waitForSelector('[data-tab-view-id="diagnostics"]');

    await app.page.click('[data-testid="panel-picker-trigger"]');
    await app.page.click('[data-panel-picker-option="console"]');
    await app.page.waitForSelector('[data-panel-id="console"]');
    await app.page.waitForFunction(() => document.querySelector('[data-testid="layout-preset-trigger"]')?.getAttribute("data-layout-dirty") === "true");

    await app.page.click('[data-testid="panel-picker-trigger"]');
    await app.page.click('[data-panel-picker-option="console"]');
    const consoleCount = await app.page.$$eval('[data-panel-id="console"]', (nodes) => nodes.length);
    assert.equal(consoleCount, 1, "singleton console focuses instead of duplicating");

    // Save layout lives inside the Layout menu since the inline toolbar
    // button was retired; the trigger's dirty marker stays always-visible.
    await app.page.click('[data-testid="layout-preset-trigger"]');
    await app.page.waitForSelector('[data-testid="save-layout"][data-layout-dirty="true"]');
    await app.page.click('[data-testid="save-layout"]');
    await app.page.waitForFunction(() => document.querySelector('[data-testid="layout-preset-trigger"]')?.getAttribute("data-layout-dirty") === "false");
    const saved = await app.page.evaluate(() => JSON.parse(localStorage.getItem("tcld-named-layouts-v1") ?? "[]"));
    assert.equal(saved[0].id, "saved-design-parity-layout");
    assert.equal(saved[0].name, "Design parity layout");
    assert.ok(saved[0].layout.tree, "saved layout contains the production Mosaic tree");

    await app.page.click('[data-testid="layout-preset-trigger"]');
    await app.page.waitForSelector('[data-testid="layout-saved-group"] [data-layout-menu-kind="saved"][data-layout-menu-id="saved-design-parity-layout"]');
    await waitForVisibleText(app.page, '[data-testid="layout-preset-menu"]', /Saved/);
    await app.page.click('[data-layout-menu-kind="built-in"][data-layout-menu-id="symbols"]');
    await app.page.waitForSelector('[data-panel-id="literals"]');
    await app.page.waitForFunction(() => !document.querySelector('[data-panel-id="console"]'));
    await app.page.waitForFunction(() => document.querySelector('[data-testid="layout-preset-trigger"]')?.getAttribute("data-layout-dirty") === "false");

    const beforeSavedApply = await app.page.$$eval("[data-panel-id]", (nodes) => nodes.map((node) => node.getAttribute("data-panel-id")).sort());
    await app.page.click('[data-testid="layout-preset-trigger"]');
    await app.page.waitForSelector('[data-testid="layout-saved-group"] [data-layout-menu-kind="saved"][data-layout-menu-id="saved-design-parity-layout"]');
    await app.page.click('[data-layout-menu-kind="saved"][data-layout-menu-id="saved-design-parity-layout"]');
    await app.page.waitForSelector('[data-panel-id="console"]');
    const afterSavedApply = await app.page.$$eval("[data-panel-id]", (nodes) => nodes.map((node) => node.getAttribute("data-panel-id")).sort());
    const afterSavedLeafIds = await app.page.$$eval("[data-leaf-id]", (nodes) => nodes.map((node) => node.getAttribute("data-leaf-id")).sort());
    const savedLeafIds = await app.page.evaluate(() => {
      const savedLayouts = JSON.parse(localStorage.getItem("tcld-named-layouts-v1") ?? "[]");
      const savedLayout = savedLayouts.find((layout: { id?: string }) => layout.id === "saved-design-parity-layout");
      return Object.keys(savedLayout?.layout?.stacks ?? {}).sort();
    });
    assert.notDeepEqual(afterSavedApply, beforeSavedApply, "saved layout reapply changes workspace stacks");
    assert.ok(afterSavedApply.includes("graph"), "saved layout restores flow graph stack");
    assert.ok(afterSavedApply.includes("console"), "saved layout restores saved console stack");
    assert.deepEqual(afterSavedLeafIds, savedLeafIds, "saved layout applies the persisted production stack ids");
    await app.page.waitForFunction(() => document.querySelector('[data-testid="layout-preset-trigger"]')?.getAttribute("data-layout-dirty") === "false");

    const beforeDrag = await app.page.$$eval("[data-leaf-id]", (nodes) => nodes.map((node) => node.getAttribute("data-leaf-id")));
    const dragHandles = await app.page.$$('[data-design-surface="panel-header"]');
    assert.ok(dragHandles.length >= 2, "workspace exposes at least two real panel drag handles");
    const first = await dragHandles[0].boundingBox();
    const second = await dragHandles[1].boundingBox();
    assert.ok(first && second, "panel drag handles have browser boxes");
    await app.page.mouse.move(first.x + first.width / 2, first.y + first.height / 2);
    await app.page.mouse.down();
    await app.page.mouse.move(second.x + second.width / 2, second.y + second.height / 2, { steps: 12 });
    await app.page.mouse.up();
    await app.page.evaluate(() => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
    const afterDrag = await app.page.$$eval("[data-leaf-id]", (nodes) => nodes.map((node) => node.getAttribute("data-leaf-id")));
    assert.ok(beforeDrag.length >= afterDrag.length, "drag/drop keeps Mosaic leaf structure valid");

    const minWidthEvidence = await app.page.evaluate(() => {
      const mosaic = document.querySelector('[data-design-surface="workspace-mosaic"][data-mosaic-resize-minimum-pane-size-percentage]');
      const leafWidths = [...document.querySelectorAll<HTMLElement>("[data-leaf-id]")].map((node) => Math.round(node.getBoundingClientRect().width));
      return {
        minimumPaneSizePercentage: mosaic?.getAttribute("data-mosaic-resize-minimum-pane-size-percentage"),
        leafWidths,
      };
    });
    assert.equal(minWidthEvidence.minimumPaneSizePercentage, "20");
    assert.equal(minWidthEvidence.leafWidths.every((width: number) => width >= 220), true);
  } finally {
    await closeProductionPage(app);
  }
});

test("V3 action bar launch cluster order and detached Configure wiring stay production-backed", async () => {
  // The action bar renders as titlebar chrome once a document is loaded, so
  // seed a run-configuration profile, load a file, then drive the launch
  // cluster (split Run button + profile dropdown with the Configure… item).
  const app = await withProductionPage("/", { dialogPath: "/tmp/action-bar.tbc" });
  try {
    await app.page.evaluate(() => {
      localStorage.setItem("tcld-launch-profiles", JSON.stringify({
        version: 1,
        selectedProfileId: "profile:smoke",
        profiles: [
          {
            id: "profile:smoke",
            name: "Smoke profile",
            path: "/tmp/smoke.tcl",
            args: "--trace",
            env: "",
            cwd: "/tmp",
            tclshPath: "",
          },
        ],
        adHoc: { args: "", env: "", cwd: "", tclshPath: "" },
      }));
      location.reload();
    });
    await app.page.waitForSelector('[data-testid="welcome-open-file"]');
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /action-bar\.tbc/);
    await app.page.waitForSelector('.tcld-main-toolbar[data-design-surface="action-bar"]');

    const launchCluster = await app.page.evaluate(() => {
      const toolbar = document.querySelector<HTMLElement>(".tcld-main-toolbar");
      const launch = toolbar?.querySelector<HTMLElement>(".tcld-actionbar-launch");
      const actionItems = [...(launch?.querySelectorAll<HTMLElement>("[data-actionbar-launch-item]") ?? [])]
        .map((node) => node.getAttribute("data-actionbar-launch-item"));
      const profileTrigger = launch?.querySelector<HTMLElement>('[data-launch-profile-trigger="true"]');
      return {
        actionItems,
        profileAriaLabel: profileTrigger?.getAttribute("aria-label") ?? null,
        profileName: launch?.querySelector(".tcld-launch-split-profile-name")?.textContent ?? "",
        text: launch?.textContent ?? "",
        hasInlineConfigPopover: /Run config · ad hoc|cmd|args|env|cwd/.test(launch?.textContent ?? ""),
        toolbarText: toolbar?.textContent ?? "",
      };
    });
    // The launch cluster is now a split button: LAUNCH label, Run, and the
    // profile dropdown trigger; Configure… lives inside the dropdown.
    assert.deepEqual(launchCluster.actionItems, ["label", "run", "profile"]);
    assert.equal(launchCluster.profileAriaLabel, "Run configuration");
    assert.equal(launchCluster.profileName, "Smoke profile");
    assert.match(launchCluster.toolbarText, /LAUNCH/);
    assert.equal(launchCluster.hasInlineConfigPopover, false);

    await app.page.click('[data-launch-profile-trigger="true"]');
    await app.page.waitForSelector('[data-launch-menu="true"]');
    const profileMenu = await app.page.evaluate(() => ({
      profiles: [...document.querySelectorAll('[data-launch-menu-item="profile"]')].map((node) => ({
        id: node.getAttribute("data-launch-menu-id"),
        checked: node.getAttribute("aria-checked"),
        label: node.querySelector(".tcld-launch-menu-label")?.textContent ?? "",
      })),
      configure: Boolean(document.querySelector('[data-testid="run-configuration-open-window"]')),
      text: document.querySelector('[data-launch-menu="true"]')?.textContent ?? "",
    }));
    assert.ok(profileMenu.profiles.some((profile) => profile.label === "Ad hoc"), "menu offers the ad hoc launch entry");
    assert.deepEqual(
      profileMenu.profiles.find((profile) => profile.id === "profile:smoke"),
      { id: "profile:smoke", checked: "true", label: "Smoke profile" },
      "seeded profile renders selected in the dropdown",
    );
    assert.ok(profileMenu.configure, "dropdown exposes the detached Configure entry");
    assert.match(profileMenu.text, /Configure/);

    await app.page.click('[data-testid="run-configuration-open-window"]');
    await app.page.waitForFunction(() =>
      (window.__TCLD_TEST_INVOKES__ ?? []).some((call) => call.cmd === "open_run_configuration_window")
      && (window.__TCLD_TEST_EVENTS__ ?? []).some((event) => event.event === "run-configuration:edit")
    );
  } finally {
    await closeProductionPage(app);
  }
});

test("V4 cursor/location selection synchronizes mapped workspace views", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/selection-sync.tbc" });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /selection-sync\.tbc/);

    await app.page.click('[data-panel-id="instructions"] [data-idx="3"]');
    await app.page.waitForFunction(() =>
      document.querySelector('[data-file-label="selection-sync.tbc"]')?.getAttribute("data-selected-inst") === "3",
    );
    // The Inspector identity card names the selected instruction (i003).
    await waitForVisibleText(app.page, '[data-panel-id="detail"] [data-inspector-identity="true"]', /i003/);

    await app.page.click('[data-testid="panel-picker-trigger"]');
    await app.page.click('[data-panel-picker-option="provenance"]');
    // Provenance rows moved to the trace-panel vocabulary (data-trace-*).
    await app.page.waitForSelector('[data-panel-id="provenance"] [data-trace-source-map-index="1"][data-trace-row-selected="true"]');

    await app.page.click('[data-testid="panel-picker-trigger"]');
    await app.page.click('[data-panel-picker-option="graph"]');
    await app.page.waitForSelector('[data-panel-id="graph"] [data-visual-selected="true"]');

    await app.page.click('[data-tab-view-id="comparison"]');
    await app.page.waitForSelector('[data-panel-id="comparison"] [data-source-map-index="1"][data-selected="true"]');
    const enabledCompareFilter = await app.page.evaluate(() =>
      [...document.querySelectorAll('[data-panel-id="comparison"] [data-compare-filter], [data-panel-id="comparison"] [data-comparison-filter]')]
        .some((node) => {
          const element = node as HTMLButtonElement | HTMLInputElement;
          return !element.disabled && element.getAttribute("aria-disabled") !== "true";
        }),
    );
    assert.equal(enabledCompareFilter, false, "Compare parity must not render an enabled filter control");

    await app.page.$eval('[data-panel-id="comparison"] [data-source-map-index="0"]', (row) => (row as HTMLElement).click());
    await app.page.waitForFunction(() =>
      document.querySelector('[data-file-label="selection-sync.tbc"]')?.getAttribute("data-selected-inst") === "0",
    );
    await app.page.waitForFunction(() =>
      document.querySelector('[data-file-label="selection-sync.tbc"]')?.getAttribute("data-active-line") === "0",
    );
    await app.page.click('[data-tab-view-id="detail"]');
    await waitForVisibleText(app.page, '[data-panel-id="detail"] [data-inspector-identity="true"]', /i000/);
    await app.page.click('[data-tab-view-id="comparison"]');
    await app.page.waitForSelector('[data-panel-id="comparison"] [data-source-map-index="0"][data-selected="true"]');

    await app.page.$eval('[data-panel-id="instructions"] [data-idx="0"]', (row) => (row as HTMLElement).click());
    await app.page.waitForFunction(() =>
      document.querySelector('[data-file-label="selection-sync.tbc"]')?.getAttribute("data-selected-inst") === "0",
    );
    await app.page.click('[data-tab-view-id="diagnostics"]');
    await app.page.waitForSelector('[data-panel-id="diagnostics"] [data-source-map-index="0"][data-selected="true"]');

    await app.page.waitForSelector('[data-panel-id="provenance"] [data-trace-source-map-index="0"]');
    await app.page.waitForSelector('[data-panel-id="provenance"] [data-trace-source-map-index="0"][data-trace-row-selected="true"]');
    await app.page.$eval('[data-panel-id="provenance"] [data-trace-source-map-index="1"]', (row) => (row as HTMLElement).click());
    await app.page.waitForFunction(() =>
      document.querySelector('[data-file-label="selection-sync.tbc"]')?.getAttribute("data-selected-inst") === "3",
    );
    await app.page.click('[data-tab-view-id="diagnostics"]');
    await app.page.waitForSelector('[data-panel-id="diagnostics"] [data-diagnostics-issue-row="approximation"][data-source-map-index="0"]');
    await app.page.$eval('[data-panel-id="diagnostics"] [data-diagnostics-issue-row="approximation"][data-source-map-index="0"]', (row) => (row as HTMLElement).click());
    await app.page.waitForFunction(() =>
      document.querySelector('[data-file-label="selection-sync.tbc"]')?.getAttribute("data-selected-inst") === "0",
    );
  } finally {
    await closeProductionPage(app);
  }
});

test("V4 Issues diagnostics distinguish clean and unavailable browser states", async () => {
  const clean = await withProductionPage("/", { dialogPath: "/tmp/clean-diagnostics.tbc" });
  try {
    await clean.page.evaluate(() => {
      const testWindow = window as unknown as {
        __TCLD_TEST_CREATE_LOAD_RESULT__: (pathname: string) => Record<string, unknown>;
      };
      const createLoadResult = testWindow.__TCLD_TEST_CREATE_LOAD_RESULT__;
      testWindow.__TCLD_TEST_CREATE_LOAD_RESULT__ = (pathname: string) => {
        const result = createLoadResult(pathname);
        result.diagnostics = {
          reconstruction: "Strict",
          recognizedConstructs: ["set-command"],
          unreducedOpcodes: [],
          approximations: [],
          suppressedFastPathRanges: [],
          passDecisions: [{ name: "reduce", enabled: true, iteration: 1, changed: true }],
          unsupportedInstructions: [],
          suggestedNextWork: [],
        };
        return result;
      };
    });
    await clean.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(clean.page, '[data-design-surface="top-tab-strip"]', /clean-diagnostics\.tbc/);
    await clean.page.click('[data-tab-view-id="diagnostics"]');
    await clean.page.waitForSelector('[data-panel-id="diagnostics"] [data-diagnostics-state="clean"]');
    await clean.page.waitForSelector('[data-panel-id="diagnostics"] [data-diagnostics-summary="severity-counts"]');
    await waitForVisibleText(clean.page, '[data-panel-id="diagnostics"] [data-diagnostics-summary="severity-counts"]', /0 issues/);
    await waitForVisibleText(clean.page, '[data-panel-id="diagnostics"] [data-diagnostics-empty-state="clean"]', /No issues/);
  } finally {
    await closeProductionPage(clean);
  }

  const unavailable = await withProductionPage("/", { dialogPath: "/tmp/unavailable-diagnostics.tbc" });
  try {
    await unavailable.page.evaluate(() => {
      const testWindow = window as unknown as {
        __TCLD_TEST_CREATE_LOAD_RESULT__: (pathname: string) => Record<string, unknown>;
      };
      const createLoadResult = testWindow.__TCLD_TEST_CREATE_LOAD_RESULT__;
      testWindow.__TCLD_TEST_CREATE_LOAD_RESULT__ = (pathname: string) => {
        const result = createLoadResult(pathname);
        delete result.diagnostics;
        return result;
      };
    });
    await unavailable.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(unavailable.page, '[data-design-surface="top-tab-strip"]', /unavailable-diagnostics\.tbc/);
    await unavailable.page.click('[data-tab-view-id="diagnostics"]');
    await unavailable.page.waitForSelector('[data-panel-id="diagnostics"] [data-diagnostics-state="unavailable"]');
    await waitForVisibleText(unavailable.page, '[data-panel-id="diagnostics"] [data-diagnostics-empty-state="unavailable"]', /Diagnostics unavailable/);
    assert.equal(await unavailable.page.$('[data-panel-id="diagnostics"] [data-diagnostics-summary="severity-counts"]'), null);
  } finally {
    await closeProductionPage(unavailable);
  }
});

test("V3/V4 top workspace tab strip renders every open document and preserves document state", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/first.tbc" });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /first\.tbc/);

    await app.page.evaluate(() => {
      const shell = document.querySelector('[data-design-surface="app-shell"]');
      if (!shell) throw new Error("missing app shell");
      const file = new File(["puts second"], "second.tbc", { type: "application/octet-stream" }) as File & { path?: string };
      Object.defineProperty(file, "path", { value: "/tmp/second.tbc" });
      const dataTransfer = new DataTransfer();
      dataTransfer.items.add(file);
      shell.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer }));
      shell.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer }));
    });
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /second\.tbc/);

    const opened = await app.page.$$eval('[data-design-surface="top-tab-strip"] .tcld-open-file-tab', (tabs) =>
      tabs.map((tab) => ({
        label: tab.getAttribute("data-file-label"),
        active: tab.getAttribute("data-active-file"),
        selectedInst: tab.getAttribute("data-selected-inst"),
        debugState: tab.getAttribute("data-debug-state"),
        text: tab.textContent ?? "",
      })),
    );
    assert.deepEqual(opened.map((tab) => tab.label), ["first.tbc", "second.tbc"]);
    assert.equal(opened[1].active, "true");
    assert.equal(opened.every((tab) => tab.selectedInst === "0"), true);
    assert.equal(opened.every((tab) => tab.debugState === "idle"), true);

    await app.page.click('[data-file-label="first.tbc"]');
    await app.page.waitForFunction(() =>
      document.querySelector('[data-file-label="first.tbc"]')?.getAttribute("data-active-file") === "true",
    );
    await waitForVisibleText(app.page, '[data-design-surface="status-bar"]', /loaded first\.tbc/);
    await app.page.click('[data-panel-id="instructions"] [data-idx="3"]');
    await app.page.waitForFunction(() =>
      document.querySelector('[data-file-label="first.tbc"]')?.getAttribute("data-selected-inst") === "3",
    );
    await app.page.click('[data-file-label="second.tbc"]');
    await app.page.waitForFunction(() =>
      document.querySelector('[data-file-label="second.tbc"]')?.getAttribute("data-active-file") === "true"
      && document.querySelector('[data-file-label="second.tbc"]')?.getAttribute("data-selected-inst") === "0",
    );
    await app.page.click('[data-file-label="first.tbc"]');
    await app.page.waitForFunction(() =>
      document.querySelector('[data-file-label="first.tbc"]')?.getAttribute("data-active-file") === "true"
      && document.querySelector('[data-file-label="first.tbc"]')?.getAttribute("data-selected-inst") === "3",
    );

    await app.page.click('[data-file-label="first.tbc"] [data-close-file-tab]');
    await app.page.waitForFunction(() => !document.querySelector('[data-file-label="first.tbc"]'));
    const afterClose = await app.page.$$eval('[data-design-surface="top-tab-strip"] .tcld-open-file-tab', (tabs) =>
      tabs.map((tab) => ({
        label: tab.getAttribute("data-file-label"),
        active: tab.getAttribute("data-active-file"),
      })),
    );
    assert.deepEqual(afterClose, [{ label: "second.tbc", active: "true" }]);
  } finally {
    await closeProductionPage(app);
  }
});

test("V5/V7 Run enters debug mode and Variables search/scope controls filter real debug-store rows", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/debug-open.tbc", debugPath: "/tmp/debug-script.tcl" });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /debug-open\.tbc/);

    await app.page.click('[title="Run / Debug (F5)"]');
    await app.page.waitForSelector('[data-design-surface="debug-paused-overlay"]');
    await app.page.click('[title="Variables"]');
    await app.page.waitForSelector('[data-panel-id="variables"]');
    await waitForVisibleText(app.page, '[data-panel-id="variables"]', /argv/);
    await waitForVisibleText(app.page, '[data-panel-id="variables"]', /env\(PATH\)/);

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
    assert.doesNotMatch(afterSearch, /argv\{/);
    const globalToggle = await app.page.$$eval('[data-panel-id="variables"] button', (buttons) =>
      buttons.find((button) => button.textContent?.trim() === "global")?.getAttribute("data-scope-active"),
    );
    assert.equal(globalToggle, "false");
  } finally {
    await closeProductionPage(app);
  }
});
