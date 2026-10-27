"use client";

import { useState, useEffect, useRef, useCallback, useMemo, useId, memo } from "react";
import clsx from "clsx";
import { commandRegistry, formatShortcutLabel, type CommandDef, type CommandContext } from "@/lib/command-registry";

import { useModalFocus } from "@/hooks/use-modal-focus";
import { usePresence } from "@/hooks/use-presence";

/** Matches the CSS exit animation on .tcld-overlay-anim / .tcld-overlay-dialog. */
const OVERLAY_EXIT_MS = 120;

interface CommandPaletteProps {
  commandContext: CommandContext;
  visible: boolean;
  onSelect: (cmd: string) => void;
  onClose: () => void;
}

export const CommandPalette = memo(function CommandPalette({
  visible,
  commandContext,
  onSelect,
  onClose,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [commands, setCommands] = useState<CommandDef[]>(() => commandRegistry.getAll());
  const inputRef = useRef<HTMLInputElement>(null);
  const dialogRef = useModalFocus(visible, onClose);
  const mounted = usePresence(visible, OVERLAY_EXIT_MS);
  const listId = useId();

  useEffect(() => {
    return commandRegistry.subscribe(() => {
      setCommands(commandRegistry.getAll());
    });
  }, []);

  const filtered = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    const availableIds = new Set(commandRegistry.getForContext(commandContext).map((command) => command.id));
    const available = commands.filter((command) => availableIds.has(command.id));
    const preferred = ["open", "debug:launch", "command:goto", "layout:default", "layout:debug", "appearance:open", "help"];
    const rank = (command: CommandDef) => {
      const index = preferred.indexOf(command.id);
      return index === -1 ? preferred.length : index;
    };
    if (!normalized) return available.sort((a, b) => rank(a) - rank(b));
    const words = normalized.split(/\s+/);
    return available.filter((command) => {
      const searchable = `${command.title} ${command.id.replace(/[:_-]/g, " ")} ${command.category}`.toLowerCase();
      return words.every((word) => searchable.includes(word));
    }).sort((a, b) => {
      // Exact titles and title prefixes precede incidental category matches.
      const score = (command: CommandDef) => command.title.toLowerCase() === normalized ? 0
        : command.title.toLowerCase().startsWith(normalized) ? 1 : 2;
      return score(a) - score(b) || rank(a) - rank(b);
    });
  }, [commands, query, commandContext]);
  const activeIndex = Math.min(selectedIdx, Math.max(0, filtered.length - 1));

  useEffect(() => {
    if (!visible) return;
    const timer = window.setTimeout(() => {
      setQuery("");
      setSelectedIdx(0);
      inputRef.current?.focus();
    }, 0);
    return () => window.clearTimeout(timer);
  }, [visible]);

  useEffect(() => {
    if (!visible) return;
    dialogRef.current?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: "nearest" });
  }, [activeIndex, filtered, visible, dialogRef]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.nativeEvent.isComposing) return;
      e.stopPropagation();
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          setSelectedIdx((i) => Math.max(0, Math.min(i + 1, filtered.length - 1)));
          break;
        case "ArrowUp":
          e.preventDefault();
          setSelectedIdx((i) => Math.max(i - 1, 0));
          break;
        case "Home":
          e.preventDefault();
          setSelectedIdx(0);
          break;
        case "End":
          e.preventDefault();
          setSelectedIdx(Math.max(0, filtered.length - 1));
          break;
        case "Enter":
          e.preventDefault();
          if (filtered[activeIndex]) {
            onSelect(filtered[activeIndex].id);
          }
          break;
        case "Escape":
          e.preventDefault();
          onClose();
          break;
      }
    },
    [filtered, activeIndex, onSelect, onClose]
  );

  return (
    mounted && (
      <div
        data-state={visible ? "open" : "closed"}
        className="absolute inset-0 z-50 flex justify-center pt-16 tcld-overlay-scrim tcld-overlay-anim"
        onClick={onClose}
      >
        <div
          data-state={visible ? "open" : "closed"}
          onClick={(e) => e.stopPropagation()}
          ref={dialogRef}
          tabIndex={-1}
          aria-modal="true"
          role="dialog"
          aria-label="Command palette"
          className={clsx(
            "tcld-command-palette w-[540px] max-w-[calc(100vw-32px)] max-h-[calc(100dvh-96px)] self-start overflow-hidden flex flex-col",
            "tcld-overlay-surface tcld-overlay-dialog tcld-overlay-dialog--drop",
          )}
        >
          <div className="px-4 py-3 border-b border-tcld-border flex items-center gap-2.5">
            <span className="font-mono text-base text-tcld-fg-dim font-semibold">
              ⌘
            </span>
            <input
              ref={inputRef}
              role="combobox"
              aria-label="Search commands"
              aria-autocomplete="list"
              aria-expanded="true"
              aria-controls={listId}
              aria-activedescendant={filtered[activeIndex] ? `${listId}-${activeIndex}` : undefined}
              value={query}
              onChange={(e) => {
                setQuery(e.target.value);
                setSelectedIdx(0);
              }}
              onKeyDown={handleKeyDown}
              placeholder="Type a command..."
              className={clsx(
                "flex-1 min-w-0",
                "font-ui text-ui",
                "bg-transparent border-none outline-none",
                "text-tcld-fg placeholder-tcld-fg-dim/50",
                "caret-tcld-accent"
              )}
            />
            <span className="font-mono text-xs text-tcld-fg-dim px-1.5 py-0.5 rounded bg-tcld-bg-2">
              esc
            </span>
          </div>

          <div className="tcld-palette-heading">{query.trim() ? "Matching commands" : "Start here · or type to find any command"}</div>
          <div id={listId} role="listbox" aria-label="Commands" className="min-h-0 max-h-96 overflow-auto py-1">
            {filtered.map((item, i) => (
              <div
                key={item.id}
                id={`${listId}-${i}`}
                role="option"
                aria-selected={i === activeIndex}
                onClick={() => onSelect(item.id)}
                onMouseEnter={() => setSelectedIdx(i)}
                className={clsx(
                  "flex items-center px-4 py-1.75 cursor-pointer",
                  "border-l-2 transition-all duration-75",
                  i === activeIndex
                    ? "bg-tcld-bg-highlight border-l-tcld-border-focus"
                    : "bg-transparent border-l-transparent"
                )}
              >
                <span
                  className={clsx(
                    "flex-1 min-w-0",
                    "font-ui text-ui",
                    i === activeIndex
                      ? "text-tcld-fg font-medium"
                      : "text-tcld-fg-muted font-normal"
                  )}
                >
                  {highlightMatch(item.title, query.trim())}
                  <span className="block text-label text-tcld-text-inactive font-normal mt-0.5">{item.category}</span>
                </span>
                {item.shortcut?.[0] && (
                  <span className="font-mono text-xs text-tcld-fg-dim bg-tcld-bg-2 px-1.75 py-0.5 rounded border border-tcld-border-dim">
                    {formatShortcutLabel(item.shortcut[0])}
                  </span>
                )}
              </div>
            ))}
            {filtered.length === 0 && (
              <div className="px-4 py-4 font-ui text-ui text-tcld-fg-dim text-center">
                No matching commands
              </div>
            )}
          </div>
          <div className="tcld-palette-footer"><span>{filtered.length} commands</span><span>↑ ↓ navigate · Enter run · Esc close</span></div>
        </div>
      </div>
    )
  );
});

function highlightMatch(text: string, query: string): React.ReactNode {
  if (!query) return text;
  const lower = text.toLowerCase();
  const qLower = query.toLowerCase();
  const idx = lower.indexOf(qLower);
  if (idx === -1) return text;

  return (
    <>
      {text.slice(0, idx)}
      <span className="text-tcld-fg font-semibold">
        {text.slice(idx, idx + query.length)}
      </span>
      {text.slice(idx + query.length)}
    </>
  );
}
