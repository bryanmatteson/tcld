import assert from "node:assert/strict";
import test from "node:test";
import { commandRegistry, type CommandContext, type CommandDef } from "@/lib/command-registry";

const baseContext: CommandContext = {
  overlayOpen: false,
  focusedPanel: null,
  debugState: "paused",
  debugBusy: false,
  isExceptionStop: false,
  hasFile: true,
  hasSelection: true,
  debuggerAvailable: true,
};

function keyEvent(
  key: string,
  opts: Partial<Pick<KeyboardEvent, "ctrlKey" | "metaKey" | "altKey" | "shiftKey">> = {},
): KeyboardEvent {
  return {
    key,
    ctrlKey: opts.ctrlKey ?? false,
    metaKey: opts.metaKey ?? false,
    altKey: opts.altKey ?? false,
    shiftKey: opts.shiftKey ?? false,
  } as KeyboardEvent;
}

function registerCommands(commands: CommandDef[]): () => void {
  const unregister = commands.map((command) => commandRegistry.register(command));
  return () => {
    for (const fn of unregister) fn();
  };
}

test("when predicates filter context-specific commands", () => {
  const cleanup = registerCommands([
    {
      id: "test.ctx.match",
      title: "Match",
      category: "Test",
      when: "focusedPanel:graph,!overlayOpen",
      run: () => {},
    },
    {
      id: "test.ctx.miss",
      title: "Miss",
      category: "Test",
      when: "focusedPanel:instructions",
      run: () => {},
    },
  ]);

  const graphCtx: CommandContext = { ...baseContext, focusedPanel: "graph" };
  const visible = commandRegistry.getForContext(graphCtx).map((command) => command.id);
  assert.deepEqual(visible, ["test.ctx.match"]);

  cleanup();
});

test("panel-scoped command wins over global command for same key", () => {
  const calls: string[] = [];
  const cleanup = registerCommands([
    {
      id: "test.global.f",
      title: "Global F",
      category: "Test",
      shortcut: ["f"],
      when: "!overlayOpen",
      run: () => calls.push("global"),
    },
    {
      id: "test.panel.f",
      title: "Panel F",
      category: "Test",
      shortcut: ["f"],
      when: "focusedPanel:graph,!overlayOpen",
      run: () => calls.push("panel"),
    },
  ]);

  const handled = commandRegistry.handleKeydown(keyEvent("f"), { ...baseContext, focusedPanel: "graph" });
  assert.equal(handled, true);
  assert.deepEqual(calls, ["panel"]);

  cleanup();
});

test("chord commands resolve on second keystroke", () => {
  const calls: string[] = [];
  const cleanup = registerCommands([
    {
      id: "test.chord.top",
      title: "Top",
      category: "Test",
      shortcut: ["g g"],
      when: "!overlayOpen",
      run: () => calls.push("top"),
    },
    {
      id: "test.chord.bookmarks",
      title: "Bookmarks",
      category: "Test",
      shortcut: ["g b"],
      when: "!overlayOpen",
      run: () => calls.push("bookmarks"),
    },
  ]);

  const started = commandRegistry.handleKeydown(keyEvent("g"), baseContext);
  assert.equal(started, true);
  assert.equal(commandRegistry.getChordHint()?.prefix, "g");

  const completed = commandRegistry.handleKeydown(keyEvent("b"), baseContext);
  assert.equal(completed, true);
  assert.deepEqual(calls, ["bookmarks"]);
  assert.equal(commandRegistry.getChordHint(), null);

  cleanup();
});
