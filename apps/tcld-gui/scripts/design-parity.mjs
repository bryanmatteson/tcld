import { spawn } from "node:child_process";
import { createServer } from "node:http";
import { existsSync, mkdirSync, readFileSync, readdirSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";

const here = path.dirname(fileURLToPath(import.meta.url));
const guiRoot = path.resolve(here, "..");
const productionExportDir = path.join(guiRoot, "dist");
const outDir = path.resolve(guiRoot, ".design-parity");
const nextEnvPath = path.join(guiRoot, "next-env.d.ts");
const originalNextEnv = existsSync(nextEnvPath) ? readFileSync(nextEnvPath, "utf8") : null;
const viewport = { width: 1280, height: 820, deviceScaleFactor: 1 };
// Same wait-budget multiplier the browser test harness honors: CI sets
// TCLD_TEST_WAIT_SCALE > 1 on slow runners, local runs keep fast defaults.
const waitScale = (() => {
  const parsed = Number(process.env.TCLD_TEST_WAIT_SCALE ?? "");
  return Number.isFinite(parsed) && parsed >= 1 ? parsed : 1;
})();

const states = [
  {
    id: "welcome",
    designSource: "tcld-welcome.jsx",
    artboard: "WelcomeOnly",
    designArtboardId: "welcome",
    productionRoute: "/",
    productionSelector: "[data-parity-state=\"welcome\"]",
    deviations: ["shell-launch-debug-secondary-cta", "welcome-browse-examples-disabled", "shell-native-window-controls"],
  },
  {
    id: "open-file-workspace",
    designSource: "tcld-workspace.jsx",
    artboard: "Workspace inspect",
    designArtboardId: "inspect",
    productionRoute: "/?parity=open-file",
    productionSelector: "[data-panel-id=\"instructions\"]",
    deviations: ["workspace-drag-resize", "workspace-layout-saved-menu", "top-tabs-readonly-dirty-state"],
  },
  {
    id: "debug-paused-workspace",
    designSource: "tcld-debug.jsx",
    artboard: "Workspace debug paused",
    designArtboardId: "debug",
    productionRoute: "/?parity=debug-paused",
    productionSelector: "[data-panel-id=\"variables\"]",
    deviations: ["debug-sparklines", "debug-breakpoint-controls"],
  },
  {
    id: "flow",
    designSource: "tcld-flow.jsx",
    artboard: "Flow control graph",
    designArtboardId: "flow",
    productionRoute: "/?parity=flow",
    productionSelector: "[data-panel-id=\"graph\"]",
    deviations: ["views-flow-id"],
  },
  {
    id: "trace",
    designSource: "tcld-trace.jsx",
    artboard: "Trace pass timeline",
    designArtboardId: "trace",
    productionRoute: "/?parity=trace",
    productionSelector: "[data-panel-id=\"provenance\"]",
    deviations: ["views-trace-id"],
  },
  {
    id: "console",
    designSource: "tcld-console.jsx",
    artboard: "Console channels",
    designArtboardId: "console",
    productionRoute: "/?parity=console",
    productionSelector: "[data-panel-id=\"console\"]",
    deviations: [],
  },
  {
    id: "symbols",
    designSource: "tcld-workspace.jsx",
    artboard: "Symbols deep dive",
    designArtboardId: "symbols",
    productionRoute: "/?parity=symbols",
    productionSelector: "[data-panel-id=\"literals\"][data-design-surface=\"view-symbols\"]",
    deviations: ["views-symbols-id"],
  },
  {
    id: "preferences-appearance",
    designSource: "tcld-prefs.jsx",
    artboard: "Preferences Appearance",
    designArtboardId: "prefs-appearance",
    productionRoute: "/?window=preferences&section=Appearance",
    productionSelector: "[data-parity-state=\"preferences-appearance\"]",
    deviations: ["preferences-editor-sections"],
  },
  {
    id: "preferences-decompiler",
    designSource: "tcld-prefs.jsx",
    artboard: "Preferences Decompiler",
    designArtboardId: "prefs-decomp",
    productionRoute: "/?window=preferences&section=Decompiler",
    productionSelector: "[data-parity-state=\"preferences-decompiler\"]",
    deviations: ["preferences-editor-sections"],
  },
  {
    id: "preferences-keybindings",
    designSource: "tcld-prefs.jsx",
    artboard: "Preferences Keybindings",
    designArtboardId: "prefs-keys",
    productionRoute: "/?window=preferences&section=Keybindings",
    productionSelector: "[data-parity-state=\"preferences-keybindings\"]",
    deviations: ["preferences-editor-sections"],
  },
  {
    id: "run-configuration",
    designSource: "tcld-prefs.jsx",
    artboard: "Run Configuration",
    designArtboardId: "runcfg",
    productionRoute: "/?window=run-configuration",
    productionSelector: "[data-parity-state=\"run-configuration\"]",
    deviations: ["run-config-model"],
  },
  {
    id: "variables-detail",
    designSource: "tcld-debug.jsx",
    artboard: "Variables detail",
    designArtboardId: "vars",
    productionRoute: "/?parity=variables",
    productionSelector: "[data-panel-id=\"variables\"]",
    deviations: ["debug-sparklines"],
  },
  {
    id: "call-stack-detail",
    designSource: "tcld-debug.jsx",
    artboard: "Call Stack detail",
    designArtboardId: "stack",
    productionRoute: "/?parity=stack",
    productionSelector: "[data-panel-id=\"stack\"]",
    deviations: [],
  },
  {
    id: "breakpoints-detail",
    designSource: "tcld-debug.jsx",
    artboard: "Breakpoints detail",
    designArtboardId: "bps",
    productionRoute: "/?parity=breakpoints",
    productionSelector: "[data-panel-id=\"breakpoints\"]",
    deviations: ["debug-breakpoint-controls"],
  },
  ...["graphite", "catppuccin", "dracula", "solarized", "tokyolight"].map((themeId) => ({
    id: `theme-${themeId}-inspect`,
    designSource: "tcld-themes.jsx",
    artboard: `Theme inspect ${themeId}`,
    designArtboardId: `th-${themeId}`,
    productionRoute: `/?parity=theme-${themeId}`,
    productionSelector: `[data-panel-id="instructions"]`,
    deviations: ["workspace-drag-resize"],
  })),
];

function mimeFor(file) {
  if (file.endsWith(".html")) return "text/html; charset=utf-8";
  if (file.endsWith(".jsx") || file.endsWith(".js")) return "text/javascript; charset=utf-8";
  if (file.endsWith(".css")) return "text/css; charset=utf-8";
  if (file.endsWith(".svg")) return "image/svg+xml";
  return "application/octet-stream";
}

function serveProductionExport() {
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
      resolve({ child: server, baseUrl: `http://127.0.0.1:${address.port}`, output: [], mode: "next build static export" });
    });
  });
}

async function waitForUrl(url, timeoutMs = 60_000) {
  const start = Date.now();
  let lastError = null;
  while (Date.now() - start < timeoutMs) {
    try {
      const response = await fetch(url);
      if (response.status < 500) return;
      lastError = new Error(`HTTP ${response.status}`);
    } catch (error) {
      lastError = error;
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`Timed out waiting for ${url}: ${lastError?.message ?? "no response"}`);
}

function runNextCommand(args, label) {
  const nextBin = path.join(guiRoot, "node_modules/next/dist/bin/next");
  const child = spawn(process.execPath, [nextBin, ...args], {
    cwd: guiRoot,
    env: { ...process.env, NEXT_TELEMETRY_DISABLED: "1", TCLD_DESIGN_PARITY: "1" },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const output = [];
  child.stdout.on("data", (chunk) => output.push(chunk.toString()));
  child.stderr.on("data", (chunk) => output.push(chunk.toString()));
  return { child, output, done: new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("exit", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${label} exited with ${code ?? "unknown"}\n${output.join("")}`));
    });
  }) };
}

async function buildProductionApp() {
  const build = runNextCommand(["build"], "next build");
  await build.done;
}

async function startProductionServer() {
  await buildProductionApp();
  const started = await serveProductionExport();
  await waitForUrl(started.baseUrl);
  return started;
}

async function freezePage(page) {
  await page.emulateMediaFeatures([{ name: "prefers-reduced-motion", value: "reduce" }]);
  await page.addStyleTag({
    content: `
      *, *::before, *::after {
        animation-duration: 0s !important;
        animation-delay: 0s !important;
        transition-duration: 0s !important;
        caret-color: transparent !important;
      }
    `,
  });
}

async function installProductionHarness(page) {
  await page.evaluateOnNewDocument(() => {
    const eventHandlers = new Map();
    const emitted = [];
    const invokes = [];
    const debugFile = "/tmp/tcld-design-parity-debug.tcl";
    let debugState = {
      state: "idle",
      scriptPath: "",
      tclVersion: null,
      pid: null,
      breakpointCount: 0,
      chunkCount: 0,
    };
    let breakpoints = [
      {
        file: debugFile,
        line: 2,
        enabled: true,
        condition: "$argc > 0",
        hitCount: 3,
      },
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
    const frames = [
      { level: 0, type: "proc", proc: "sample_proc", file: debugFile, line: 2, cmd: "puts $argv" },
      { level: 1, type: "source", proc: null, file: "/tmp/main.tcl", line: 7, cmd: "sample_proc $argv" },
      { level: 2, type: "top", proc: null, file: "/tmp/main.tcl", line: 1, cmd: "source main.tcl" },
    ];

    try {
      localStorage.removeItem("tcld-layout-v3");
      localStorage.removeItem("tcld-layout-v2");
      localStorage.removeItem("tcld-named-layouts-v1");
    } catch {
      // Ignore browser storage setup failures; the app will still render defaults.
    }

    function emit(event, payload) {
      emitted.push({ event, payload });
      const handlers = eventHandlers.get(event) ?? [];
      for (const handler of handlers) handler(payload);
    }

    function createLoadResult(pathname) {
      const filename = String(pathname).split("/").pop() || "demo.tbc";
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
          { idx: 1, pc: 1, op: "push1", args: "literal 1", operands: [1], argc: 0, group: 0, commandPcRange: [0, 2], groupStart: false, comment: "argv", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 2, primaryNextLocation: 2, secondaryNextLocation: null, nextLocations: [2] },
          { idx: 2, pc: 2, op: "invokeStk1", args: "argc 2", operands: [2], argc: 2, group: 0, commandPcRange: [0, 2], groupStart: false, comment: "invoke", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: 3, primaryNextLocation: 3, secondaryNextLocation: null, nextLocations: [3] },
          { idx: 3, pc: 3, op: "done", args: "", operands: [], argc: 0, group: 1, commandPcRange: [3, 3], groupStart: true, comment: "done", size: 1, isJump: false, isJumpConditional: false, isJumpUnconditional: false, jumpTarget: null, fallthrough: null, primaryNextLocation: null, secondaryNextLocation: null, nextLocations: [] },
        ],
        literals: [
          { index: 0, value: "set", refCount: 1, size: 3 },
          { index: 1, value: "argv", refCount: 1, size: 4 },
        ],
        exceptions: [],
        blocks: [
          { id: 0, start: 0, end: 2, startPc: 0, endPc: 2, group: "c000", edges: [1], instructions: [{ pc: 0, name: "push1", operands: [0] }, { pc: 1, name: "push1", operands: [1] }, { pc: 2, name: "invokeStk1", operands: [2] }] },
          { id: 1, start: 3, end: 3, startPc: 3, endPc: 3, group: "c001", edges: [], instructions: [{ pc: 3, name: "done", operands: [] }] },
        ],
        traces: [
          { instructionIndex: 0, pc: 0, opcode: "push1", argc: 0, stackDelta: 1, beforeDepth: 0, afterDepth: 1, popped: [], pushed: ["set"] },
          { instructionIndex: 1, pc: 1, opcode: "push1", argc: 0, stackDelta: 1, beforeDepth: 1, afterDepth: 2, popped: [], pushed: ["argv"] },
          { instructionIndex: 2, pc: 2, opcode: "invokeStk1", argc: 2, stackDelta: -1, beforeDepth: 2, afterDepth: 1, popped: ["set", "argv"], pushed: [""] },
          { instructionIndex: 3, pc: 3, opcode: "done", argc: 0, stackDelta: 0, beforeDepth: 1, afterDepth: 1, popped: [], pushed: [] },
        ],
        decompileLines: [
          { lineIndex: 0, text: "set argv {demo.tbc --trace}", indent: 0, group: 0, sourceMapIndex: 0, sourceLineStart: 0, sourceLineEnd: 0 },
          { lineIndex: 1, text: "puts $argv", indent: 0, group: 1, sourceMapIndex: 1, sourceLineStart: 1, sourceLineEnd: 1 },
        ],
        decompileTokens: [
          { capture: "keyword", startByte: 0, endByte: 3, startRow: 0, startColumn: 0, endRow: 0, endColumn: 3 },
          { capture: "function", startByte: 29, endByte: 33, startRow: 1, startColumn: 0, endRow: 1, endColumn: 4 },
        ],
        decompileText: "set argv {demo.tbc --trace}\nputs $argv\n",
        dumpText: "ByteCode 8.6\n",
        summaryText: "4 instructions, 2 literals",
        sourceMap: [
          { commandGroup: 0, commandPath: [0], parentCommandGroup: null, bytecodeRange: [0, 2], sourceLines: [0, 1], sourceChars: [0, 28], text: "set argv {demo.tbc --trace}", originalStatus: "available", originalSpan: [0, 28] },
          { commandGroup: 1, commandPath: [1], parentCommandGroup: null, bytecodeRange: [3, 3], sourceLines: [1, 2], sourceChars: [29, 39], text: "puts $argv", originalStatus: "available", originalSpan: [29, 39] },
        ],
        diagnostics: {
          reconstruction: "Strict",
          recognizedConstructs: ["set-command", "puts-command"],
          unreducedOpcodes: [],
          approximations: [],
          suppressedFastPathRanges: [],
          passDecisions: [{ name: "reduce", enabled: true, iteration: 1, changed: true }],
          unsupportedInstructions: [],
          suggestedNextWork: [],
        },
        provenance: { kind: "File", label: pathname, originalSourceAvailable: true },
        originalSourceText: "set argv {demo.tbc --trace}\nputs $argv\n",
      };
    }

    window.__TAURI_INTERNALS__ = {};
    window.__TCLD_TEST_EVENTS__ = emitted;
    window.__TCLD_TEST_INVOKES__ = invokes;
    window.__TCLD_TEST_TAURI__ = {
      invoke: async (cmd, args = {}) => {
        invokes.push({ cmd, args });
        if (cmd === "load_file") return createLoadResult(String(args.path ?? "/tmp/tcld-design-parity-sample.tbc"));
        if (cmd === "load_appearance_settings") return null;
        if (cmd === "save_appearance_settings") return null;
        if (cmd === "debug_get_state") return debugState;
        if (cmd === "debug_launch") {
          const path = String(args.path ?? debugFile);
          debugState = {
            state: "paused",
            scriptPath: path,
            tclVersion: "8.6",
            pid: 4242,
            breakpointCount: breakpoints.length,
            chunkCount: 1,
          };
          setTimeout(() => {
            emit("debugger:state", debugState);
            emit("debugger:ready", {
              tclVersion: "8.6",
              pid: 4242,
              debugExtensionLoaded: true,
              debugExtensionInstalled: true,
              debugExtensionCapabilities: "instruction_step 1 bytecode_hooks 1",
            });
            emit("debugger:live_load", createLoadResult("/tmp/tcld-design-parity-sample.tbc"));
            emit("debugger:stopped", {
              reason: "breakpoint",
              file: path,
              line: 2,
              frameCount: frames.length,
              pc: 2,
              instructionIndex: 1,
              blockId: 0,
            });
            emit("debugger:variables", { level: 1, vars: variables });
            emit("debugger:stack", { frames });
            emit("debugger:output", { stream: "stdout", text: "debug parity ready" });
          }, 0);
          return null;
        }
        if (cmd === "debug_disassemble") return "Command 0:\n  push1 0\n  invokeStk1 2\n";
        if (cmd === "debug_get_variables") {
          setTimeout(() => emit("debugger:variables", { level: args.level ?? 1, vars: variables }), 0);
          return null;
        }
        if (cmd === "debug_get_stack") {
          setTimeout(() => emit("debugger:stack", { frames }), 0);
          return null;
        }
        if (cmd === "debug_list_breakpoints") return breakpoints;
        if (cmd === "debug_set_breakpoint") {
          const file = String(args.file ?? debugFile);
          const line = Number(args.line ?? 1);
          breakpoints = breakpoints.filter((bp) => bp.file !== file || bp.line !== line);
          breakpoints.push({ file, line, condition: args.condition, enabled: true, hitCount: 3 });
          return null;
        }
        if (cmd === "debug_set_breakpoint_enabled") {
          breakpoints = breakpoints.map((bp) =>
            bp.file === args.file && bp.line === args.line ? { ...bp, enabled: Boolean(args.enabled) } : bp,
          );
          return null;
        }
        if (cmd === "debug_clear_breakpoint") {
          breakpoints = breakpoints.filter((bp) => bp.file !== args.file || bp.line !== args.line);
          return null;
        }
        if (cmd === "debug_clear_all_breakpoints") {
          breakpoints = [];
          return null;
        }
        if (cmd === "debug_continue") {
          debugState = { ...debugState, state: "running" };
          setTimeout(() => emit("debugger:resumed", { controlMode: "continue" }), 0);
          return null;
        }
        if (cmd === "debug_pause") {
          debugState = { ...debugState, state: "paused" };
          return null;
        }
        if (cmd === "debug_stop") {
          debugState = { ...debugState, state: "terminated" };
          return null;
        }
        if (cmd === "debug_eval") return null;
        return null;
      },
      openFileDialog: async () => "/tmp/tcld-design-parity-sample.tbc",
      openScriptDialog: async () => debugFile,
      saveFileDialog: async (defaultPath) => defaultPath ?? "/tmp/output.tcl",
      emit: async (event, payload) => emit(event, payload),
      listen: async (event, handler) => {
        const handlers = eventHandlers.get(event) ?? [];
        handlers.push(handler);
        eventHandlers.set(event, handlers);
        return () => {
          eventHandlers.set(event, (eventHandlers.get(event) ?? []).filter((candidate) => candidate !== handler));
        };
      },
    };
  });
}

async function captureProduction(page, baseUrl, state) {
  const url = new URL(state.productionRoute, baseUrl).toString();
  await page.goto(url, { waitUntil: "networkidle0" });
  await freezePage(page);
  try {
    await page.waitForSelector(state.productionSelector, { timeout: 20_000 * waitScale, visible: true });
  } catch (error) {
    const debug = await page.evaluate(() => ({
      url: location.href,
      parity: document.querySelector("[data-parity-state]")?.getAttribute("data-parity-state"),
      panels: [...document.querySelectorAll("[data-panel-id]")].map((node) => node.getAttribute("data-panel-id")),
      text: document.body.textContent?.slice(0, 500),
      invokes: window.__TCLD_TEST_INVOKES__ ?? [],
    }));
    console.error(`[production debug] ${JSON.stringify(debug, null, 2)}`);
    throw error;
  }
  const artifact = `${state.id}.production.png`;
  const fullPath = path.join(outDir, artifact);
  await page.screenshot({ path: fullPath, fullPage: false });
  return { url, artifact: path.join(".design-parity", artifact) };
}

// NOTE: the design/ reference sources were removed from the repo
// (2026-05, "Remove design inventory/parity docs"); this harness now captures
// production-side screenshots only. The in-repo source of truth for parity is
// DESIGN_PARITY_STATES + the production selectors below.

function assertArtifact(relativePath) {
  const fullPath = path.join(guiRoot, relativePath);
  if (!existsSync(fullPath) || statSync(fullPath).size === 0) {
    throw new Error(`Missing or empty screenshot artifact: ${relativePath}`);
  }
}

function stopServer(child) {
  if (typeof child.close === "function") {
    child.close();
  } else if (!child.killed) {
    child.kill("SIGTERM");
  }
}

mkdirSync(outDir, { recursive: true });
for (const file of readdirSync(outDir)) {
  if (/\.(png|svg)$/.test(file) || file === "manifest.json") {
    unlinkSync(path.join(outDir, file));
  }
}

let production;
let browser;
try {
  production = await startProductionServer();
  browser = await puppeteer.launch({
    headless: true,
    defaultViewport: viewport,
    args: ["--no-sandbox", "--disable-setuid-sandbox", "--font-render-hinting=none"],
  });

  const page = await browser.newPage();
  await installProductionHarness(page);
  const manifestStates = [];
  for (const state of states) {
    const productionCapture = await captureProduction(page, production.baseUrl, state);
    assertArtifact(productionCapture.artifact);
    manifestStates.push({
      ...state,
      artifacts: {
        production: productionCapture.artifact,
      },
      capture: {
        renderer: "puppeteer",
        viewport,
        productionUrl: productionCapture.url,
        productionSelector: state.productionSelector,
      },
    });
  }

  const manifest = {
    generatedAt: "deterministic",
    productionBaseUrl: production.baseUrl,
    productionRenderer: production.mode,
    states: manifestStates,
  };
  writeFileSync(path.join(outDir, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`captured ${manifest.states.length} rendered design parity states to ${path.relative(process.cwd(), outDir)}`);
} finally {
  if (browser) await browser.close();
  if (production) stopServer(production.child);
  if (originalNextEnv != null) writeFileSync(nextEnvPath, originalNextEnv);
}
