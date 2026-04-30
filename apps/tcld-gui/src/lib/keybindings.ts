export interface KeybindingAction {
  id: string;
  defaultShortcuts: string[];
}

export interface KeybindingDiagnostic {
  actionId: string;
  message: string;
}

export interface KeybindingImportResult {
  ok: boolean;
  diagnostics: KeybindingDiagnostic[];
}

export interface KeybindingConflict {
  shortcut: string;
  actionIds: string[];
}

export interface StorageLike {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

const STORAGE_KEY = "tcld-keybindings";

export function createKeybindingStore(args: { storage: StorageLike; actions: KeybindingAction[] }) {
  let actions = new Map(args.actions.map((action) => [action.id, action]));
  let custom = loadCustomBindings(args.storage, actions);

  function persist() {
    args.storage.setItem(STORAGE_KEY, JSON.stringify({ bindings: orderedBindings(custom) }));
  }

  return {
    updateActions(nextActions: KeybindingAction[]) {
      actions = new Map(nextActions.map((action) => [action.id, action]));
      custom = new Map([...custom.entries()].filter(([id]) => actions.has(id)));
      persist();
    },
    setBinding(actionId: string, shortcuts: string[]) {
      validateAction(actionId, actions);
      validateShortcuts(actionId, shortcuts);
      custom.set(actionId, [...shortcuts]);
      persist();
    },
    getEffectiveShortcuts(actionId: string): string[] {
      const action = actions.get(actionId);
      if (!action) return [];
      return custom.get(actionId) ?? action.defaultShortcuts;
    },
    exportJson(): string {
      return `${JSON.stringify({ version: 1, bindings: orderedBindings(custom) }, null, 2)}\n`;
    },
    importJson(text: string): KeybindingImportResult {
      let parsed: unknown;
      try {
        parsed = JSON.parse(text);
      } catch {
        return { ok: false, diagnostics: [{ actionId: "*", message: "Malformed keybinding JSON." }] };
      }
      const bindings = parseBindingObject(parsed);
      const diagnostics: KeybindingDiagnostic[] = [];
      for (const [actionId, shortcuts] of Object.entries(bindings)) {
        if (!actions.has(actionId)) {
          diagnostics.push({ actionId, message: `Unknown action id '${actionId}'.` });
          continue;
        }
        const invalid = shortcuts.find((shortcut) => !isValidShortcut(shortcut));
        if (invalid) {
          diagnostics.push({ actionId, message: `Malformed shortcut '${invalid}'.` });
          continue;
        }
      }
      if (diagnostics.length > 0) return { ok: false, diagnostics };
      custom = new Map([...custom.entries(), ...Object.entries(bindings).map(([id, shortcuts]) => [id, shortcuts] as [string, string[]])]);
      persist();
      return { ok: true, diagnostics: [] };
    },
    resetToDefaults() {
      custom = new Map();
      args.storage.removeItem(STORAGE_KEY);
    },
    shortcutOverrides(): Record<string, string[]> {
      return orderedBindings(custom);
    },
    conflicts(): KeybindingConflict[] {
      const buckets = new Map<string, string[]>();
      for (const id of actions.keys()) {
        for (const shortcut of custom.get(id) ?? actions.get(id)!.defaultShortcuts) {
          const bucket = buckets.get(shortcut) ?? [];
          bucket.push(id);
          buckets.set(shortcut, bucket);
        }
      }
      return [...buckets.entries()]
        .filter(([, actionIds]) => actionIds.length > 1)
        .map(([shortcut, actionIds]) => ({ shortcut, actionIds }));
    },
  };
}

function loadCustomBindings(storage: StorageLike, actions: Map<string, KeybindingAction>): Map<string, string[]> {
  const raw = storage.getItem(STORAGE_KEY);
  if (!raw) return new Map();
  try {
    const bindings = parseBindingObject(JSON.parse(raw));
    return new Map(Object.entries(bindings).filter(([id]) => actions.has(id)));
  } catch {
    return new Map();
  }
}

function parseBindingObject(parsed: unknown): Record<string, string[]> {
  if (!parsed || typeof parsed !== "object" || !("bindings" in parsed)) return {};
  const bindings = (parsed as { bindings: unknown }).bindings;
  if (!bindings || typeof bindings !== "object" || Array.isArray(bindings)) return {};
  const out: Record<string, string[]> = {};
  for (const [key, value] of Object.entries(bindings)) {
    out[key] = Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
  }
  return out;
}

function validateAction(actionId: string, actions: Map<string, KeybindingAction>) {
  if (!actions.has(actionId)) throw new Error(`unknown action id ${actionId}`);
}

function validateShortcuts(actionId: string, shortcuts: string[]) {
  const invalid = shortcuts.find((shortcut) => !isValidShortcut(shortcut));
  if (invalid) throw new Error(`malformed shortcut ${invalid} for ${actionId}`);
}

function isValidShortcut(shortcut: string): boolean {
  return shortcut
    .trim()
    .split(" ")
    .every((part) => {
      const pieces = part.split("+").filter(Boolean);
      if (pieces.length === 0 || pieces[pieces.length - 1] == null) return false;
      const key = pieces[pieces.length - 1];
      if (["Mod", "Ctrl", "Control", "Meta", "Cmd", "Command", "Alt", "Option", "Shift"].includes(key)) return false;
      return pieces.length === part.split("+").length;
    });
}

function orderedBindings(custom: Map<string, string[]>): Record<string, string[]> {
  return Object.fromEntries([...custom.entries()].sort(([a], [b]) => a.localeCompare(b)));
}
