import assert from "node:assert/strict";
import test from "node:test";
import {
  closeProductionPage,
  waitForVisibleText,
  withProductionPage,
} from "./browser-production-harness.ts";

/**
 * C1 — reserved v0.4+ debug event kinds (DTO forward visibility).
 *
 * The backend reserves the `debugger:disassembly_context`,
 * `debugger:profile_data`, `debugger:epoch_change`, and
 * `debugger:compilation_event` channels for the four gated runtime
 * features. Nothing emits them in v1, but a newer backend may. The debug
 * store must tolerate them arriving before their UI exists: each arrival
 * is recorded in the console output ("unhandled event kind: X") and the
 * session keeps working — no crash, no silent drop.
 */

test("C1 reserved debug event kinds surface as unhandled in the console without breaking the store", async () => {
  const app = await withProductionPage("/", { dialogPath: "/tmp/debug-open.tbc", debugPath: "/tmp/debug-script.tcl" });
  try {
    await app.page.click('[data-testid="welcome-open-file"]');
    await waitForVisibleText(app.page, '[data-design-surface="top-tab-strip"]', /debug-open\.tbc/);
    await app.page.click('[title="Run / Debug (F5)"]');
    await app.page.waitForSelector('.relative[data-design-surface="debug-paused-overlay"]');

    // Feed all four reserved kinds through the harness's event emit with
    // plan-shaped payloads (plus one intentionally empty payload — the
    // draft plans are still loose, so partial payloads must be tolerated).
    await app.page.evaluate(() => {
      const emit = window.__TCLD_TEST_TAURI__?.emit;
      if (!emit) throw new Error("test harness emit missing");
      void emit("debugger:disassembly_context", {
        procName: "::foo",
        sourceFile: "/tmp/debug-script.tcl",
        disassembly: "  (0) push1 0\n",
        pcToLine: [[0, 1]],
      });
      void emit("debugger:profile_data", {
        requestId: 1,
        mode: "line_counts",
        lineData: [{ file: "/tmp/debug-script.tcl", line: 2, count: 1000 }],
      });
      void emit("debugger:epoch_change", {
        command: "::foo",
        oldEpoch: 2,
        newEpoch: 3,
        trigger: "proc_redefine",
        compileEpoch: 47,
      });
      void emit("debugger:compilation_event", {});
    });

    // The debug layout shows the console as the sole view of its own
    // leaf, so it is already mounted once the debug overlay is up.
    await app.page.waitForSelector('[data-panel-id="console"]');
    await waitForVisibleText(app.page, '[data-panel-id="console"]', /unhandled event kind: disassembly_context/);
    await waitForVisibleText(app.page, '[data-panel-id="console"]', /unhandled event kind: profile_data/);
    await waitForVisibleText(app.page, '[data-panel-id="console"]', /unhandled event kind: epoch_change/);
    await waitForVisibleText(app.page, '[data-panel-id="console"]', /unhandled event kind: compilation_event/);

    // The store survived: the paused overlay is still mounted and debug
    // controls still dispatch through the Tauri boundary.
    assert.notEqual(
      await app.page.$('.relative[data-design-surface="debug-paused-overlay"]'),
      null,
      "paused overlay must survive reserved event kinds",
    );
    await app.page.click('[title="Step Into (F11)"]');
    await app.page.waitForFunction(() =>
      (window.__TCLD_TEST_INVOKES__ ?? []).some((entry) => entry.cmd === "debug_step_into"),
    );

    // Known event kinds still flow after the reserved ones were handled.
    await app.page.evaluate(() => {
      void window.__TCLD_TEST_TAURI__?.emit?.("debugger:output", {
        stream: "stdout",
        text: "still alive after reserved kinds",
      });
    });
    await waitForVisibleText(app.page, '[data-panel-id="console"]', /still alive after reserved kinds/);
  } finally {
    await closeProductionPage(app);
  }
});
