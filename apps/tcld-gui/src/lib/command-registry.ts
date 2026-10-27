"use client";

import type { ViewId, DebugSessionState } from "@/types";

export interface CommandContext {
  overlayOpen: boolean;
  focusedPanel: ViewId | null;
  debugState: DebugSessionState;
  debugBusy: boolean;
  isExceptionStop: boolean;
  hasFile: boolean;
  hasSelection: boolean;
  /**
   * Backend capability handshake — false when the Rust backend was built
   * without the `debugger` feature. Debug commands gate on this via
   * `when: "...,debuggerAvailable"` so keyboard dispatch and native menu
   * availability degrade together.
   */
  debuggerAvailable: boolean;
}

export interface CommandDef {
  id: string;
  title: string;
  category: string;
  shortcut?: string[];
  when?: string;
  allowInInput?: boolean;
  run: () => void;
}

export interface ChordHint {
  prefix: string;
  options: { key: string; title: string }[];
}

interface KeyStroke {
  key: string;
  ctrl: boolean;
  meta: boolean;
  alt: boolean;
  shift: boolean;
  mod: boolean;
}

interface ShortcutPattern {
  raw: string;
  strokes: KeyStroke[];
}

interface CompiledCommand extends CommandDef {
  _order: number;
  _panelScoped: boolean;
  _whenPredicate: (ctx: CommandContext) => boolean;
  _shortcuts: ShortcutPattern[];
  _defaultShortcut?: string[];
}

interface ChordState {
  prefix: KeyStroke;
  candidates: CompiledCommand[];
  timeout: ReturnType<typeof setTimeout>;
}

type Listener = () => void;

const CHORD_TIMEOUT_MS = 500;

function normalizeKey(raw: string): string {
  if (raw === " ") return "Space";
  if (raw === "Esc") return "Escape";
  if (raw === "Return") return "Enter";
  return raw;
}

function parseShortcutPart(part: string): KeyStroke | null {
  const tokens = part
    .split("+")
    .map((token) => token.trim())
    .filter(Boolean);

  if (tokens.length === 0) return null;

  let key = "";
  let ctrl = false;
  let meta = false;
  let alt = false;
  let shift = false;
  let mod = false;

  for (const token of tokens) {
    const lower = token.toLowerCase();
    if (lower === "ctrl" || lower === "control") {
      ctrl = true;
      continue;
    }
    if (lower === "meta" || lower === "cmd" || lower === "command") {
      meta = true;
      continue;
    }
    if (lower === "alt" || lower === "option") {
      alt = true;
      continue;
    }
    if (lower === "shift") {
      shift = true;
      continue;
    }
    if (lower === "mod") {
      mod = true;
      continue;
    }
    key = normalizeKey(token);
  }

  if (!key) return null;

  return {
    key,
    ctrl,
    meta,
    alt,
    shift,
    mod,
  };
}

const SHORTCUT_KEY_LABELS: Record<string, string> = {
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
  enter: "↵",
  escape: "Esc",
  backspace: "⌫",
  delete: "Del",
  tab: "Tab",
  space: "Space",
  " ": "Space",
};

/**
 * Display label for a registry shortcut string: "Mod+o" reads "⌘O" on macOS
 * and "Ctrl+O" elsewhere, chords keep their space ("g g"), and bare keys stay
 * as typed so single-letter bindings don't suggest Shift.
 */
export function formatShortcutLabel(
  shortcut: string,
  isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform),
): string {
  return shortcut
    .trim()
    .split(" ")
    .filter(Boolean)
    .map((part) => {
      const stroke = parseShortcutPart(part);
      if (!stroke) return part;
      const named = SHORTCUT_KEY_LABELS[stroke.key.toLowerCase()];
      const hasModifier = stroke.mod || stroke.ctrl || stroke.meta || stroke.alt || stroke.shift;
      const key = named ?? (hasModifier && stroke.key.length === 1 ? stroke.key.toUpperCase() : stroke.key);
      const modifiers = isMac
        ? [
          stroke.ctrl && "⌃",
          stroke.alt && "⌥",
          stroke.shift && "⇧",
          (stroke.mod || stroke.meta) && "⌘",
        ]
        : [
          (stroke.mod || stroke.ctrl) && "Ctrl",
          stroke.meta && "Meta",
          stroke.alt && "Alt",
          stroke.shift && "Shift",
        ];
      const active = modifiers.filter(Boolean) as string[];
      return isMac ? `${active.join("")}${key}` : [...active, key].join("+");
    })
    .join(" ");
}

function parseShortcut(shortcut: string): ShortcutPattern | null {
  const parts = shortcut
    .trim()
    .split(" ")
    .map((part) => part.trim())
    .filter(Boolean);

  if (parts.length === 0) return null;

  const strokes: KeyStroke[] = [];
  for (const part of parts) {
    const parsed = parseShortcutPart(part);
    if (!parsed) return null;
    strokes.push(parsed);
  }

  return {
    raw: shortcut,
    strokes,
  };
}

function compileWhen(when: string | undefined): (ctx: CommandContext) => boolean {
  if (!when) return () => true;

  const checks = when
    .split(",")
    .map((clause) => clause.trim())
    .filter(Boolean)
    .map((clause) => {
      const negated = clause.startsWith("!");
      const body = negated ? clause.slice(1) : clause;

      if (body.includes(":")) {
        const [prop, value] = body.split(":", 2);
        return (ctx: CommandContext) => {
          const actual = String(ctx[prop as keyof CommandContext]);
          return negated ? actual !== value : actual === value;
        };
      }

      return (ctx: CommandContext) => {
        const actual = Boolean(ctx[body as keyof CommandContext]);
        return negated ? !actual : actual;
      };
    });

  return (ctx: CommandContext) => checks.every((check) => check(ctx));
}

function eventMatchesStroke(e: KeyboardEvent, stroke: KeyStroke, isMac: boolean): boolean {
  const key = normalizeKey(e.key);
  if (key !== stroke.key) return false;

  if (stroke.mod) {
    const expectedMod = isMac ? e.metaKey : e.ctrlKey;
    if (!expectedMod) return false;
    if (isMac ? e.ctrlKey : e.metaKey) return false;
  }

  if (stroke.ctrl && !e.ctrlKey) return false;
  if (stroke.meta && !e.metaKey) return false;
  if (stroke.alt && !e.altKey) return false;
  if (stroke.shift && !e.shiftKey) return false;

  if (!stroke.ctrl && !stroke.mod && e.ctrlKey) return false;
  if (!stroke.meta && !stroke.mod && e.metaKey) return false;
  if (!stroke.alt && e.altKey) return false;

  const namedKey = stroke.key.length > 1;
  if (!stroke.shift && namedKey && e.shiftKey) return false;

  return true;
}

function isTextInput(el: Element | null): boolean {
  if (!el) return false;
  const tagName = el.tagName;
  if (tagName === "INPUT" || tagName === "TEXTAREA" || tagName === "SELECT") {
    return true;
  }
  return (el as HTMLElement).isContentEditable;
}

function isPanelScoped(when: string | undefined): boolean {
  if (!when) return false;
  return when
    .split(",")
    .some((clause) => clause.trim().startsWith("focusedPanel:"));
}

function formatStroke(stroke: KeyStroke): string {
  const parts: string[] = [];
  if (stroke.mod) parts.push("Mod");
  if (stroke.ctrl) parts.push("Ctrl");
  if (stroke.meta) parts.push("Meta");
  if (stroke.alt) parts.push("Alt");
  if (stroke.shift && stroke.key.length > 1) parts.push("Shift");
  parts.push(stroke.key);
  return parts.join("+");
}

class CommandRegistry {
  private commands = new Map<string, CompiledCommand>();
  private listeners = new Set<Listener>();
  private order = 0;
  private chordState: ChordState | null = null;
  private shortcutOverrides = new Map<string, string[]>();

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  getChordHint(): ChordHint | null {
    if (!this.chordState) return null;

    const options = new Map<string, string>();
    for (const command of this.chordState.candidates) {
      for (const shortcut of command._shortcuts) {
        if (shortcut.strokes.length !== 2) continue;
        if (!this.sameStroke(shortcut.strokes[0], this.chordState.prefix)) continue;
        const key = formatStroke(shortcut.strokes[1]);
        if (!options.has(key)) {
          options.set(key, command.title);
        }
      }
    }

    return {
      prefix: formatStroke(this.chordState.prefix),
      options: [...options.entries()]
        .sort(([a], [b]) => a.localeCompare(b))
        .map(([key, title]) => ({ key, title })),
    };
  }

  private emitChange() {
    for (const listener of this.listeners) listener();
  }

  register(cmd: CommandDef): () => void {
    const effectiveShortcut = this.shortcutOverrides.get(cmd.id) ?? cmd.shortcut;
    const parsedShortcuts = (effectiveShortcut ?? [])
      .map((shortcut) => parseShortcut(shortcut))
      .filter((parsed): parsed is ShortcutPattern => parsed !== null);

    const compiled: CompiledCommand = {
      ...cmd,
      shortcut: effectiveShortcut,
      _order: ++this.order,
      _panelScoped: isPanelScoped(cmd.when),
      _whenPredicate: compileWhen(cmd.when),
      _shortcuts: parsedShortcuts,
      _defaultShortcut: cmd.shortcut,
    };

    if (process.env.NODE_ENV !== "production") {
      for (const other of this.commands.values()) {
        const overlaps = compiled._shortcuts.some((shortcut) =>
          other._shortcuts.some((otherShortcut) => shortcut.raw === otherShortcut.raw),
        );
        if (overlaps && compiled.when === other.when && compiled.id !== other.id) {
          console.warn(
            `[command-registry] shortcut conflict: '${compiled.id}' and '${other.id}' share shortcut(s) with when='${compiled.when ?? ""}'`,
          );
        }
      }
    }

    this.commands.set(cmd.id, compiled);
    this.emitChange();

    return () => {
      this.commands.delete(cmd.id);
      this.emitChange();
    };
  }

  execute(id: string): boolean {
    const cmd = this.commands.get(id);
    if (!cmd) return false;
    cmd.run();
    return true;
  }

  setShortcutOverrides(overrides: Record<string, string[]>): void {
    this.shortcutOverrides = new Map(Object.entries(overrides));
    for (const command of this.commands.values()) {
      const effectiveShortcut = this.shortcutOverrides.get(command.id) ?? command._defaultShortcut;
      command.shortcut = effectiveShortcut;
      command._shortcuts = (effectiveShortcut ?? [])
        .map((shortcut) => parseShortcut(shortcut))
        .filter((parsed): parsed is ShortcutPattern => parsed !== null);
    }
    this.emitChange();
  }

  getShortcutOverrides(): Record<string, string[]> {
    return Object.fromEntries([...this.shortcutOverrides.entries()].sort(([a], [b]) => a.localeCompare(b)));
  }

  getAll(): CommandDef[] {
    return [...this.commands.values()]
      .sort((a, b) => {
        if (a.category !== b.category) return a.category.localeCompare(b.category);
        if (a.title !== b.title) return a.title.localeCompare(b.title);
        return a._order - b._order;
      })
      .map(stripCompiledFields);
  }

  getForContext(ctx: CommandContext): CommandDef[] {
    return [...this.commands.values()]
      .filter((command) => command._whenPredicate(ctx))
      .sort((a, b) => {
        if (a.category !== b.category) return a.category.localeCompare(b.category);
        return a.title.localeCompare(b.title);
      })
      .map(stripCompiledFields);
  }

  handleKeydown(e: KeyboardEvent, ctx: CommandContext): boolean {
    const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);
    const inInput = typeof document !== "undefined" && isTextInput(document.activeElement);

    if (this.chordState && normalizeKey(e.key) === "Escape") {
      this.clearChordState();
      return false;
    }

    if (this.chordState) {
      const match = this.pickBest(
        this.chordState.candidates.filter((command) => {
          if (inInput && !command.allowInInput) return false;
          if (!command._whenPredicate(ctx)) return false;
          return command._shortcuts.some((shortcut) =>
            shortcut.strokes.length === 2
            && this.sameStroke(shortcut.strokes[0], this.chordState?.prefix)
            && eventMatchesStroke(e, shortcut.strokes[1], isMac),
          );
        }),
      );

      this.clearChordState();

      if (match) {
        match.run();
        return true;
      }
    }

    const candidates = [...this.commands.values()].filter((command) => {
      if (inInput && !command.allowInInput) return false;
      if (!command._whenPredicate(ctx)) return false;
      return command._shortcuts.some((shortcut) => {
        if (shortcut.strokes.length !== 1) return false;
        return eventMatchesStroke(e, shortcut.strokes[0], isMac);
      });
    });

    const chordPrefixes = [...this.commands.values()].filter((command) => {
      if (inInput && !command.allowInInput) return false;
      if (!command._whenPredicate(ctx)) return false;
      return command._shortcuts.some((shortcut) => {
        if (shortcut.strokes.length < 2) return false;
        return eventMatchesStroke(e, shortcut.strokes[0], isMac);
      });
    });

    if (chordPrefixes.length > 0) {
      const prefix = this.findPrefixStroke(chordPrefixes, e, isMac);
      if (prefix) {
        this.startChordState(prefix, chordPrefixes);
        return true;
      }
    }

    const winner = this.pickBest(candidates);
    if (!winner) return false;

    winner.run();
    return true;
  }

  private findPrefixStroke(
    commands: CompiledCommand[],
    e: KeyboardEvent,
    isMac: boolean,
  ): KeyStroke | null {
    for (const command of commands) {
      for (const shortcut of command._shortcuts) {
        if (shortcut.strokes.length < 2) continue;
        if (eventMatchesStroke(e, shortcut.strokes[0], isMac)) {
          return shortcut.strokes[0];
        }
      }
    }
    return null;
  }

  private pickBest(candidates: CompiledCommand[]): CompiledCommand | null {
    if (candidates.length === 0) return null;
    candidates.sort((a, b) => {
      const panelDelta = Number(b._panelScoped) - Number(a._panelScoped);
      if (panelDelta !== 0) return panelDelta;
      const whenDelta = Number(Boolean(b.when)) - Number(Boolean(a.when));
      if (whenDelta !== 0) return whenDelta;
      return b._order - a._order;
    });
    return candidates[0] ?? null;
  }

  private sameStroke(a: KeyStroke, b: KeyStroke | null | undefined): boolean {
    if (!b) return false;
    return (
      a.key === b.key
      && a.ctrl === b.ctrl
      && a.meta === b.meta
      && a.alt === b.alt
      && a.shift === b.shift
      && a.mod === b.mod
    );
  }

  private startChordState(prefix: KeyStroke, candidates: CompiledCommand[]) {
    this.clearChordState();
    const timeout = setTimeout(() => {
      this.clearChordState();
    }, CHORD_TIMEOUT_MS);
    this.chordState = {
      prefix,
      candidates,
      timeout,
    };
    this.emitChange();
  }

  private clearChordState() {
    if (!this.chordState) return;
    clearTimeout(this.chordState.timeout);
    this.chordState = null;
    this.emitChange();
  }
}

function stripCompiledFields(command: CompiledCommand): CommandDef {
  return {
    id: command.id,
    title: command.title,
    category: command.category,
    shortcut: command.shortcut,
    when: command.when,
    allowInInput: command.allowInInput,
    run: command.run,
  };
}

export const commandRegistry = new CommandRegistry();
