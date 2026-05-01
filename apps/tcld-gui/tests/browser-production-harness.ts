import { spawn } from "node:child_process";
import { createServer, type Server } from "node:http";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer, { type Browser, type KeyInput, type Page } from "puppeteer";
import type { SerializedAppearanceSettings } from "@/lib/settings";

const here = path.dirname(fileURLToPath(import.meta.url));
const guiRoot = path.resolve(here, "..");
const productionExportDir = path.join(guiRoot, "dist");
const buildLockDir = path.join(guiRoot, ".browser-test-build.lock");

/**
 * Multiplier applied to every browser wait budget. CI sets
 * TCLD_TEST_WAIT_SCALE (> 1) so slow 2-CPU runners get proportionally
 * longer ceilings; local runs leave it unset and keep the fast defaults.
 * Values below 1 (or garbage) fall back to 1 so the scale can only ever
 * lengthen budgets, never silently shorten them.
 */
const WAIT_SCALE = (() => {
  const parsed = Number(process.env.TCLD_TEST_WAIT_SCALE ?? "");
  return Number.isFinite(parsed) && parsed >= 1 ? parsed : 1;
})();

/** Scale a base wait budget by TCLD_TEST_WAIT_SCALE (see WAIT_SCALE). */
export function scaledWaitMs(baseMs: number): number {
  return Math.round(baseMs * WAIT_SCALE);
}

/** Shape of every `invoke` call recorded by the browser test harness. */
export interface TestInvokeRecord {
  cmd: string;
  args?: Record<string, unknown>;
}

/** Shape of every Tauri event emitted through the browser test harness. */
export interface TestEventRecord {
  event: string;
  payload: unknown;
}

declare global {
  interface Window {
    __TCLD_TEST_APPEARANCE__?: SerializedAppearanceSettings | null;
    __TCLD_TEST_CLIPBOARD__?: string;
    __TCLD_TEST_CREATE_LOAD_RESULT__?: (pathname: string) => unknown;
    __TCLD_TEST_EVENTS__?: TestEventRecord[];
    __TCLD_TEST_INVOKES__?: TestInvokeRecord[];
  }
}

export interface ProductionPageHandle {
  browser: Browser;
  page: Page;
  baseUrl: string;
  server: Server;
  consoleMessages: BrowserConsoleMessage[];
  pageErrors: string[];
}

export interface BrowserConsoleMessage {
  type: string;
  text: string;
}

export interface BrowserFixtureOptions {
  dialogPath?: string | null;
  loadPath?: string;
  filename?: string;
  clipboardText?: string;
  debugPath?: string;
  /**
   * Backend capability handshake fixture (`get_app_capabilities`). Defaults
   * to true (debugger feature compiled in). Set false to simulate a Rust
   * backend built without the `debugger` feature so tests can assert the
   * explicit degradation path (disabled run controls, unavailable panel
   * states, refused launches).
   */
  debuggerAvailable?: boolean;
}

export async function withProductionPage(
  route: string,
  options: BrowserFixtureOptions = {},
): Promise<ProductionPageHandle> {
  await ensureProductionExport();
  const server = await serveProductionExport();
  const browser = await puppeteer.launch({
    headless: true,
    args: ["--no-sandbox", "--disable-setuid-sandbox"],
  });
  const page = await browser.newPage();
  const consoleMessages: BrowserConsoleMessage[] = [];
  const pageErrors: string[] = [];
  page.on("console", (message) => {
    consoleMessages.push({ type: message.type(), text: message.text() });
  });
  page.on("pageerror", (error) => {
    pageErrors.push(error instanceof Error ? error.message : String(error));
  });
  // Every waitForSelector/waitForFunction/goto without an explicit timeout
  // inherits this ceiling, so TCLD_TEST_WAIT_SCALE lengthens all of them
  // at once on slow runners (the puppeteer default is a flat 30s).
  page.setDefaultTimeout(scaledWaitMs(30_000));
  // Keep the workspace comfortably wider than WORKSPACE_COLLAPSE_AT_PX
  // (1280): at exactly 1280 the workspace enters the narrow flyout mode and
  // a drawer overlays the right-hand leaves, swallowing tab clicks.
  await page.setViewport({ width: 1600, height: 900, deviceScaleFactor: 1 });
  await installBrowserTauriHarness(page, options);
  await page.goto(`${server.baseUrl}${route}`, { waitUntil: "networkidle0" });
  return { browser, page, baseUrl: server.baseUrl, server: server.server, consoleMessages, pageErrors };
}

export async function closeProductionPage(handle: ProductionPageHandle): Promise<void> {
  await handle.browser.close();
  await new Promise<void>((resolve) => handle.server.close(() => resolve()));
}

export async function waitForText(page: Page, selector: string, pattern: RegExp, timeout = scaledWaitMs(10_000)): Promise<void> {
  await page.waitForFunction(
    (sel, source, flags) => {
      const text = document.querySelector(sel)?.textContent ?? "";
      return new RegExp(source, flags).test(text);
    },
    { timeout },
    selector,
    pattern.source,
    pattern.flags,
  );
}

export async function waitForVisibleText(page: Page, selector: string, pattern: RegExp, timeout = scaledWaitMs(10_000)): Promise<void> {
  await page.waitForFunction(
    (sel, source, flags) => {
      const text = [...document.querySelectorAll(sel)]
        .filter((node) => {
          const rect = (node as HTMLElement).getBoundingClientRect();
          return rect.width > 0 && rect.height > 0;
        })
        .map((node) => node.textContent ?? "")
        .join("\n");
      return new RegExp(source, flags).test(text);
    },
    { timeout },
    selector,
    pattern.source,
    pattern.flags,
  );
}

/**
 * Dispatch a `Mod+key` shortcut the way production resolves `Mod`: the
 * command registry reads `navigator.platform` inside the page, so the
 * primary modifier is Meta on macOS Chrome and Ctrl everywhere else.
 * Tests that hardcoded `keyboard.down("Meta")` were silent no-ops on
 * Linux runners — the command never fired and the follow-up DOM wait ran
 * out its ceiling ("Waiting failed: 30000ms exceeded").
 */
export async function pressModShortcut(page: Page, key: KeyInput): Promise<void> {
  const isMac = await page.evaluate(() => /Mac/i.test(navigator.platform));
  const modifier: KeyInput = isMac ? "Meta" : "Control";
  await page.keyboard.down(modifier);
  await page.keyboard.press(key);
  await page.keyboard.up(modifier);
}

export function createLoadResult(pathname: string, filename = path.basename(pathname)) {
  return {
    metadata: {
      path: pathname,
      filename,
      tclVersion: "8.6",
      instructionCount: 4,
      commandCount: 2,
      codeBytes: 64,
      maxStackDepth: 2,
      literalCount: 2,
      exceptionCount: 0,
      auxCount: 0,
      syntaxEngine: "tree-sitter",
      format: 6,
      build: null,
    },
    instructions: [
      instruction(0, 0, "push1", "literal 0", 0, true),
      instruction(1, 1, "push1", "literal 1", 0, false),
      instruction(2, 2, "invokeStk1", "argc 2", 0, false),
      instruction(3, 3, "done", "", 1, true),
    ],
    literals: [
      { index: 0, value: "set", refCount: 1, size: 3 },
      { index: 1, value: "x", refCount: 1, size: 1 },
    ],
    exceptions: [],
    blocks: createBlocks(),
    traces: [
      { instructionIndex: 0, pc: 0, opcode: "push1", argc: 0, stackDelta: 1, beforeDepth: 0, afterDepth: 1, popped: [], pushed: ["set"] },
      { instructionIndex: 1, pc: 1, opcode: "push1", argc: 0, stackDelta: 1, beforeDepth: 1, afterDepth: 2, popped: [], pushed: ["x"] },
      { instructionIndex: 2, pc: 2, opcode: "invokeStk1", argc: 2, stackDelta: -1, beforeDepth: 2, afterDepth: 1, popped: ["set", "x"], pushed: [""] },
      { instructionIndex: 3, pc: 3, opcode: "done", argc: 0, stackDelta: 0, beforeDepth: 1, afterDepth: 1, popped: [], pushed: [] },
    ],
    decompileLines: [
      { lineIndex: 0, text: "set x 1", indent: 0, group: 0, sourceMapIndex: 0, sourceLineStart: 0, sourceLineEnd: 0 },
      { lineIndex: 1, text: "puts $x", indent: 0, group: 1, sourceMapIndex: 1, sourceLineStart: 1, sourceLineEnd: 1 },
    ],
    decompileTokens: [
      { capture: "keyword", startByte: 0, endByte: 3, startRow: 0, startColumn: 0, endRow: 0, endColumn: 3 },
      { capture: "function", startByte: 8, endByte: 12, startRow: 1, startColumn: 0, endRow: 1, endColumn: 4 },
    ],
    decompileText: "set x 1\nputs $x\n",
    // No multi-line blocks in the two-line fixture — the backend would
    // report no foldable regions for it. Tests that override decompileLines
    // with block structures must also override this field.
    decompileFoldingRanges: [],
    dumpText: "ByteCode 8.6\n",
    summaryText: "4 instructions, 2 literals",
    sourceMap: [
      {
        commandGroup: 0,
        commandPath: [0],
        parentCommandGroup: null,
        bytecodeRange: [0, 2],
        sourceLines: [0, 1],
        sourceChars: [0, 7],
        text: "set x 1",
        originalStatus: "available",
        originalSpan: [0, 7],
      },
      {
        commandGroup: 1,
        commandPath: [1],
        parentCommandGroup: null,
        bytecodeRange: [3, 3],
        sourceLines: [1, 2],
        sourceChars: [8, 15],
        text: "puts $x",
        originalStatus: "available",
        originalSpan: [8, 15],
      },
    ],
    diagnostics: createDiagnostics(),
    reconstruction: "Strict",
    provenance: { kind: "File", label: pathname, originalSourceAvailable: true },
    originalSourceText: "set x 1\nputs $x\n",
  };
}

async function installBrowserTauriHarness(page: Page, options: BrowserFixtureOptions) {
  await page.evaluateOnNewDocument((fixtureOptions) => {
    const testWindow = window as unknown as Record<string, unknown>;
    const eventHandlers: Map<string, Array<(payload: unknown) => void>> = new Map();
    const emitted: Array<{ event: string; payload: unknown }> = [];
    const invokes: Array<{ cmd: string; args?: Record<string, unknown> }> = [];
    let clipboard = fixtureOptions.clipboardText ?? "";
    const debugPath = fixtureOptions.debugPath ?? fixtureOptions.dialogPath ?? fixtureOptions.loadPath ?? "/tmp/demo-debug.tcl";
    let debugState = {
      state: "idle",
      scriptPath: "",
      tclVersion: null as string | null,
      pid: null as number | null,
      breakpointCount: 0,
      chunkCount: 0,
    };
    let breakpoints: Array<{ file: string; line: number; condition?: string; enabled: boolean; hitCount: number }> = [
      { file: debugPath, line: 2, condition: "$argc > 0", enabled: true, hitCount: 3 },
    ];
    let watchpoints: Array<{ id: number; name: string; scope: string; level: number; condition?: string; active: boolean }> = [
      { id: 1, name: "watchVar", scope: "local", level: 1, condition: "$watchVar ne 0", active: true },
    ];
    const variables = [
      {
        name: "argv",
        value: "{demo.tbc --trace}",
        type: "list",
        scope: "local",
        children: [
          { name: "argv[0]", value: "demo.tbc", type: "string", scope: "local" },
          { name: "argv[1]", value: "--trace", type: "string", scope: "local" },
        ],
      },
      { name: "argc", value: "2", type: "int", scope: "local" },
      { name: "env(PATH)", value: "/usr/bin:/bin", type: "string", scope: "global" },
    ];
    const stackFrames = [
      { level: 0, type: "proc", procName: "sample_proc", file: debugPath, line: 2, cmd: "puts $argv" },
      { level: 1, type: "source", procName: null, file: "/tmp/main.tcl", line: 7, cmd: "sample_proc $argv" },
      { level: 2, type: "top", procName: null, file: "/tmp/main.tcl", line: 1, cmd: "source main.tcl" },
    ];
    const emit = async (event: string, payload: unknown) => {
      emitted.push({ event, payload });
      const handlers = eventHandlers.get(event) ?? [];
      for (const handler of handlers) handler(payload);
    };

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        readText: async () => clipboard,
        writeText: async (text: unknown) => {
          clipboard = String(text);
          testWindow.__TCLD_TEST_CLIPBOARD__ = clipboard;
        },
      },
    });

    testWindow.__TAURI_INTERNALS__ = {};
    testWindow.__TCLD_TEST_EVENTS__ = emitted;
    testWindow.__TCLD_TEST_INVOKES__ = invokes;
    testWindow.__TCLD_TEST_TAURI__ = {
      invoke: async (cmd: string, args?: Record<string, unknown>) => {
        invokes.push({ cmd, args });
        if (cmd === "load_file") {
          const loadPath = String(args?.path ?? fixtureOptions.loadPath ?? "/tmp/demo.tbc");
          return (testWindow.__TCLD_TEST_CREATE_LOAD_RESULT__ as (pathname: string) => unknown)(loadPath);
        }
        if (cmd === "load_text") {
          // Browse-examples path: production sends inline Tcl source plus a
          // display label; the fixture answers with the same load-result shape
          // keyed by that label so the workspace opens a document for it.
          const label = String(args?.label ?? fixtureOptions.filename ?? "example.tcl");
          return (testWindow.__TCLD_TEST_CREATE_LOAD_RESULT__ as (pathname: string) => unknown)(label);
        }
        if (cmd === "load_appearance_settings") return null;
        if (cmd === "get_app_capabilities") {
          return { debugger_available: fixtureOptions.debuggerAvailable !== false };
        }
        if (cmd === "debug_get_state") return debugState;
        if (cmd === "save_appearance_settings") {
          testWindow.__TCLD_TEST_APPEARANCE__ = args?.settings ?? null;
          return null;
        }
        if (cmd === "debug_launch") {
          const path = String(args?.path ?? debugPath);
          debugState = {
            state: "paused",
            scriptPath: path,
            tclVersion: "8.6",
            pid: 4242,
            breakpointCount: breakpoints.length,
            chunkCount: 1,
          };
          breakpoints = breakpoints.map((bp) => ({ ...bp, file: path }));
          setTimeout(() => {
            emit("debugger:state", debugState);
            emit("debugger:ready", {
              tclVersion: "8.6",
              pid: 4242,
              debugExtensionLoaded: true,
              debugExtensionInstalled: true,
              debugExtensionCapabilities: "instruction_step 1 bytecode_hooks 1",
            });
            emit("debugger:live_load", (testWindow.__TCLD_TEST_CREATE_LOAD_RESULT__ as (pathname: string) => unknown)("/tmp/live-debug.tbc"));
            emit("debugger:stopped", {
              reason: "breakpoint",
              file: path,
              line: 2,
              frameCount: stackFrames.length,
              pc: 2,
              instructionIndex: 1,
              blockId: 0,
            });
            emit("debugger:variables", { level: 1, vars: variables });
            emit("debugger:stack", { frames: stackFrames });
            emit("debugger:output", { stream: "stdout", text: "debug parity ready" });
            emit("debugger:watchpoint_hit", {
              watchId: 1,
              name: "watchVar",
              operation: "write",
              oldValue: "0",
              newValue: "1",
              file: path,
              line: 2,
              frameCount: stackFrames.length,
            });
          }, 0);
          return null;
        }
        if (cmd === "debug_disassemble") return "Command 0:\n  push1 0\n  invokeStk1 2\n";
        if (cmd === "debug_get_variables") {
          setTimeout(() => emit("debugger:variables", { level: args?.level ?? 1, vars: variables }), 0);
          return null;
        }
        if (cmd === "debug_get_stack") {
          setTimeout(() => emit("debugger:stack", { frames: stackFrames }), 0);
          return null;
        }
        if (cmd === "debug_list_breakpoints") return breakpoints;
        if (cmd === "debug_set_breakpoint") {
          const file = String(args?.file ?? debugPath);
          const line = Number(args?.line ?? 1);
          breakpoints = breakpoints.filter((bp) => bp.file !== file || bp.line !== line);
          breakpoints.push({ file, line, condition: args?.condition as string | undefined, enabled: true, hitCount: 0 });
          return null;
        }
        if (cmd === "debug_set_breakpoint_enabled") {
          breakpoints = breakpoints.map((bp) =>
            bp.file === args?.file && bp.line === args?.line ? { ...bp, enabled: Boolean(args?.enabled) } : bp,
          );
          return null;
        }
        if (cmd === "debug_clear_breakpoint") {
          breakpoints = breakpoints.filter((bp) => bp.file !== args?.file || bp.line !== args?.line);
          return null;
        }
        if (cmd === "debug_clear_all_breakpoints") {
          breakpoints = [];
          return null;
        }
        if (cmd === "debug_list_watchpoints") return watchpoints;
        if (cmd === "debug_set_watchpoint") {
          watchpoints = [
            ...watchpoints.filter((watchpoint) => watchpoint.name !== args?.name),
            { id: watchpoints.length + 1, name: String(args?.name), scope: String(args?.scope ?? "local"), level: Number(args?.level ?? 1), condition: args?.condition as string | undefined, active: true },
          ];
          return null;
        }
        if (cmd === "debug_clear_watchpoint") {
          watchpoints = watchpoints.filter((watchpoint) => watchpoint.name !== args?.name);
          return null;
        }
        if (cmd === "debug_clear_all_watchpoints") {
          watchpoints = [];
          return null;
        }
        if (cmd === "debug_continue") {
          debugState = { ...debugState, state: "running" };
          setTimeout(() => {
            emit("debugger:state", debugState);
            emit("debugger:resumed", { controlMode: "continue" });
            emit("debugger:output", { stream: "stdout", text: "continued" });
          }, 0);
          return null;
        }
        if (cmd === "debug_pause") {
          debugState = { ...debugState, state: "paused" };
          setTimeout(() => {
            emit("debugger:state", debugState);
            emit("debugger:stopped", {
              reason: "pause",
              file: debugState.scriptPath || debugPath,
              line: 2,
              frameCount: stackFrames.length,
              pc: 2,
              instructionIndex: 1,
              blockId: 0,
            });
            emit("debugger:variables", { level: 1, vars: variables });
            emit("debugger:stack", { frames: stackFrames });
          }, 0);
          return null;
        }
        if (cmd === "debug_step_into" || cmd === "debug_step_instruction" || cmd === "debug_step_over" || cmd === "debug_step_out") {
          setTimeout(() => emit("debugger:output", { stream: "stdout", text: `${cmd} completed` }), 0);
          return null;
        }
        if (cmd === "debug_stop") {
          debugState = { ...debugState, state: "terminated" };
          setTimeout(() => {
            emit("debugger:state", debugState);
            emit("debugger:exited", { code: 0, reason: "stopped" });
          }, 0);
          return null;
        }
        if (cmd === "debug_eval") {
          setTimeout(() => emit("debugger:eval_result", { result: `eval:${args?.script}`, error: false }), 0);
          return null;
        }
        return null;
      },
      openFileDialog: async () => fixtureOptions.dialogPath ?? fixtureOptions.loadPath ?? "/tmp/demo.tbc",
      openScriptDialog: async () => fixtureOptions.debugPath ?? fixtureOptions.dialogPath ?? fixtureOptions.loadPath ?? "/tmp/demo.tcl",
      saveFileDialog: async (defaultPath?: string) => defaultPath ?? "/tmp/output.tcl",
      emit,
      listen: async (event: string, handler: (payload: unknown) => void) => {
        const handlers = eventHandlers.get(event) ?? [];
        handlers.push(handler);
        eventHandlers.set(event, handlers);
        return () => {
          eventHandlers.set(event, (eventHandlers.get(event) ?? []).filter((candidate: (payload: unknown) => void) => candidate !== handler));
        };
      },
    };
    testWindow.__TCLD_TEST_CREATE_LOAD_RESULT__ = (pathname: string) => {
      const filename = String(pathname).split("/").pop() || fixtureOptions.filename || "demo.tbc";
      return {
        metadata: {
          path: pathname,
          filename,
          tclVersion: "8.6",
          instructionCount: 4,
          commandCount: 2,
          codeBytes: 64,
          maxStackDepth: 2,
          literalCount: 2,
          exceptionCount: 0,
          auxCount: 0,
          syntaxEngine: "tree-sitter",
          format: 6,
          build: null,
        },
        instructions: [
          { idx: 0, pc: 0, op: "push1", args: "literal 0", operands: [0], argc: 0, group: 0, commandPcRange: [0, 2], groupStart: true, comment: "set", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 1, primaryNextLocation: 1, secondaryNextLocation: null, nextLocations: [1] },
          { idx: 1, pc: 1, op: "push1", args: "literal 1", operands: [1], argc: 0, group: 0, commandPcRange: [0, 2], groupStart: false, comment: "x", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 2, primaryNextLocation: 2, secondaryNextLocation: null, nextLocations: [2] },
          { idx: 2, pc: 2, op: "invokeStk1", args: "argc 2", operands: [2], argc: 2, group: 0, commandPcRange: [0, 2], groupStart: false, comment: "invoke", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 3, primaryNextLocation: 3, secondaryNextLocation: null, nextLocations: [3] },
          { idx: 3, pc: 3, op: "done", args: "", operands: [], argc: 0, group: 1, commandPcRange: [3, 3], groupStart: true, comment: "done", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: null, primaryNextLocation: null, secondaryNextLocation: null, nextLocations: [] },
        ],
        literals: [
          { index: 0, value: "set", refCount: 1, size: 3 },
          { index: 1, value: "x", refCount: 1, size: 1 },
        ],
        exceptions: [],
        blocks: [
          { id: 0, start: 0, end: 2, startPc: 0, endPc: 2, group: "c000", edges: [1], instructions: [{ pc: 0, name: "push1", operands: [0] }, { pc: 1, name: "push1", operands: [1] }, { pc: 2, name: "invokeStk1", operands: [2] }] },
          { id: 1, start: 3, end: 3, startPc: 3, endPc: 3, group: "c001", edges: [], instructions: [{ pc: 3, name: "done", operands: [] }] },
        ],
        traces: [
          { instructionIndex: 0, pc: 0, opcode: "push1", argc: 0, stackDelta: 1, beforeDepth: 0, afterDepth: 1, popped: [], pushed: ["set"] },
          { instructionIndex: 1, pc: 1, opcode: "push1", argc: 0, stackDelta: 1, beforeDepth: 1, afterDepth: 2, popped: [], pushed: ["x"] },
          { instructionIndex: 2, pc: 2, opcode: "invokeStk1", argc: 2, stackDelta: -1, beforeDepth: 2, afterDepth: 1, popped: ["set", "x"], pushed: [""] },
          { instructionIndex: 3, pc: 3, opcode: "done", argc: 0, stackDelta: 0, beforeDepth: 1, afterDepth: 1, popped: [], pushed: [] },
        ],
        decompileLines: [
          { lineIndex: 0, text: "set x 1", indent: 0, group: 0, sourceMapIndex: 0, sourceLineStart: 0, sourceLineEnd: 0 },
          { lineIndex: 1, text: "puts $x", indent: 0, group: 1, sourceMapIndex: 1, sourceLineStart: 1, sourceLineEnd: 1 },
        ],
        decompileTokens: [
          { capture: "keyword", startByte: 0, endByte: 3, startRow: 0, startColumn: 0, endRow: 0, endColumn: 3 },
          { capture: "function", startByte: 8, endByte: 12, startRow: 1, startColumn: 0, endRow: 1, endColumn: 4 },
        ],
        decompileText: "set x 1\nputs $x\n",
        decompileFoldingRanges: [],
        dumpText: "ByteCode 8.6\n",
        summaryText: "4 instructions, 2 literals",
        sourceMap: [
          { commandGroup: 0, commandPath: [0], parentCommandGroup: null, bytecodeRange: [0, 2], sourceLines: [0, 1], sourceChars: [0, 7], text: "set x 1", originalStatus: "available", originalSpan: [0, 7] },
          { commandGroup: 1, commandPath: [1], parentCommandGroup: null, bytecodeRange: [3, 3], sourceLines: [1, 2], sourceChars: [8, 15], text: "puts $x", originalStatus: "available", originalSpan: [8, 15] },
        ],
        diagnostics: {
          reconstruction: "Strict",
          recognizedConstructs: ["set-command"],
          unreducedOpcodes: ["unknownOpcode"],
          approximations: [{ kind: "UnresolvedInstruction", bytecodeRange: [0, 2], reason: "fixture approximation" }],
          suppressedFastPathRanges: [[1, 2]],
          passDecisions: [{ name: "reduce", enabled: true, iteration: 1, changed: true }],
          unsupportedInstructions: ["unsupportedDemo"],
          suggestedNextWork: ["inspect source map"],
        },
        reconstruction: "Strict",
        provenance: { kind: "File", label: pathname, originalSourceAvailable: true },
        originalSourceText: "set x 1\nputs $x\n",
      };
    };
  }, options);
}

function createDiagnostics() {
  return {
    reconstruction: "Strict",
    recognizedConstructs: ["set-command"],
    unreducedOpcodes: ["unknownOpcode"],
    approximations: [{ kind: "UnresolvedInstruction", bytecodeRange: [0, 2], reason: "fixture approximation" }],
    suppressedFastPathRanges: [[1, 2]],
    passDecisions: [{ name: "reduce", enabled: true, iteration: 1, changed: true }],
    unsupportedInstructions: ["unsupportedDemo"],
    suggestedNextWork: ["inspect source map"],
  };
}

function createBlocks() {
  return [
    {
      id: 0,
      start: 0,
      end: 2,
      startPc: 0,
      endPc: 2,
      group: "c000",
      edges: [1],
      instructions: [
        { pc: 0, name: "push1", operands: [0] },
        { pc: 1, name: "push1", operands: [1] },
        { pc: 2, name: "invokeStk1", operands: [2] },
      ],
    },
    {
      id: 1,
      start: 3,
      end: 3,
      startPc: 3,
      endPc: 3,
      group: "c001",
      edges: [],
      instructions: [{ pc: 3, name: "done", operands: [] }],
    },
  ];
}

function instruction(idx: number, pc: number, op: string, args: string, group: number, groupStart: boolean) {
  return {
    idx,
    pc,
    op,
    args,
    operands: args ? [idx] : [],
    argc: 0,
    group,
    commandPcRange: group === 0 ? [0, 2] : [3, 3],
    groupStart,
    comment: args,
    size: 1,
    isJump: false,
    isJumpConditional: false,
    isJumpUnconditional: false,
    jumpTarget: null,
    fallthrough: idx < 3 ? idx + 1 : null,
    primaryNextLocation: idx < 3 ? idx + 1 : null,
    secondaryNextLocation: null,
    nextLocations: idx < 3 ? [idx + 1] : [],
  };
}

/**
 * Newest mtime of every build input that shapes the production export:
 * everything under src/ (recursive), plus the top-level config files that
 * change the compiled output. Used for staleness detection below.
 */
function newestBuildInputMtimeMs(): number {
  const roots = [
    path.join(guiRoot, "src"),
    path.join(guiRoot, "package.json"),
    path.join(guiRoot, "next.config.ts"),
    path.join(guiRoot, "next.config.mjs"),
    path.join(guiRoot, "postcss.config.mjs"),
    path.join(guiRoot, "tsconfig.json"),
  ];
  let newest = 0;
  const visit = (candidate: string) => {
    const stats = statSync(candidate, { throwIfNoEntry: false });
    if (!stats) return;
    if (stats.isDirectory()) {
      for (const entry of readdirSync(candidate)) visit(path.join(candidate, entry));
      return;
    }
    if (stats.mtimeMs > newest) newest = stats.mtimeMs;
  };
  for (const root of roots) visit(root);
  return newest;
}

/**
 * A dist/ export is reusable only when it exists AND is at least as new as
 * every build input. Reusing an export older than the sources produced the
 * 35s-timeout class of failures: tests asserted current selectors against
 * markup built from a previous commit.
 */
function productionExportIsFresh(): boolean {
  const indexStats = statSync(path.join(productionExportDir, "index.html"), { throwIfNoEntry: false });
  if (!indexStats) return false;
  return indexStats.mtimeMs >= newestBuildInputMtimeMs();
}

async function ensureProductionExport() {
  if (process.env.TCLD_BROWSER_TEST_REUSE_EXPORT === "1" && existsSync(path.join(productionExportDir, "index.html"))) {
    return;
  }
  if (productionExportIsFresh() && !existsSync(buildLockDir)) {
    return;
  }
  if (tryAcquireBuildLock()) {
    try {
      // Re-check under the lock: another process may have finished a fresh
      // build between our staleness probe and lock acquisition.
      if (!productionExportIsFresh()) {
        await runNextBuild();
      }
    } finally {
      rmSync(buildLockDir, { recursive: true, force: true });
    }
    return;
  }

  const start = Date.now();
  while (Date.now() - start < 240_000) {
    if (!existsSync(buildLockDir) && productionExportIsFresh()) {
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error("Timed out waiting for production browser test build");
}

function tryAcquireBuildLock(): boolean {
  try {
    mkdirSync(buildLockDir);
    return true;
  } catch {
    return false;
  }
}

function runNextBuild(): Promise<void> {
  const nextBin = path.join(guiRoot, "node_modules/next/dist/bin/next");
  const child = spawn(process.execPath, [nextBin, "build"], {
    cwd: guiRoot,
    env: { ...process.env, NEXT_TELEMETRY_DISABLED: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const output: string[] = [];
  child.stdout.on("data", (chunk) => output.push(chunk.toString()));
  child.stderr.on("data", (chunk) => output.push(chunk.toString()));
  return new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`next build exited with ${code ?? "unknown"}\n${output.join("")}`));
    });
  });
}

function mimeFor(file: string) {
  if (file.endsWith(".html")) return "text/html; charset=utf-8";
  if (file.endsWith(".js")) return "text/javascript; charset=utf-8";
  if (file.endsWith(".css")) return "text/css; charset=utf-8";
  if (file.endsWith(".svg")) return "image/svg+xml";
  if (file.endsWith(".woff2")) return "font/woff2";
  return "application/octet-stream";
}

function serveProductionExport(): Promise<{ server: Server; baseUrl: string }> {
  const server = createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    const relative = decodeURIComponent(url.pathname === "/" ? "/index.html" : url.pathname);
    let target = path.normalize(path.join(productionExportDir, relative));
    if (!target.startsWith(productionExportDir) || !existsSync(target) || statSync(target).isDirectory()) {
      target = path.join(productionExportDir, "index.html");
    }
    if (!existsSync(target) || statSync(target).isDirectory()) {
      res.writeHead(404);
      res.end("not found");
      return;
    }
    res.writeHead(200, { "content-type": mimeFor(target) });
    res.end(readFileSync(target));
  });
  return new Promise((resolve, reject) => {
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address && typeof address === "object") {
        resolve({ server, baseUrl: `http://127.0.0.1:${address.port}` });
      } else {
        reject(new Error("static server did not return a TCP address"));
      }
    });
  });
}
