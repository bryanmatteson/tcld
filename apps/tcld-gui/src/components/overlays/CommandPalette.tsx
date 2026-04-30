"use client";

import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import clsx from "clsx";
import { motion, AnimatePresence } from "framer-motion";
import { commandRegistry, type CommandDef } from "@/lib/command-registry";

interface CommandPaletteProps {
  visible: boolean;
  onSelect: (cmd: string) => void;
  onClose: () => void;
}

export function CommandPalette({
  visible,
  onSelect,
  onClose,
}: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [selectedIdx, setSelectedIdx] = useState(0);
  const [commands, setCommands] = useState<CommandDef[]>(() => commandRegistry.getAll());
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    return commandRegistry.subscribe(() => {
      setCommands(commandRegistry.getAll());
    });
  }, []);

  const filtered = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return commands;
    return commands.filter((command) => (
      command.title.toLowerCase().includes(normalized)
      || command.id.toLowerCase().includes(normalized)
      || command.category.toLowerCase().includes(normalized)
    ));
  }, [commands, query]);

  useEffect(() => {
    if (!visible) return;
    const timer = window.setTimeout(() => {
      setQuery("");
      setSelectedIdx(0);
      inputRef.current?.focus();
    }, 0);
    return () => window.clearTimeout(timer);
  }, [visible]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          setSelectedIdx((i) => Math.max(0, Math.min(i + 1, filtered.length - 1)));
          break;
        case "ArrowUp":
          e.preventDefault();
          setSelectedIdx((i) => Math.max(i - 1, 0));
          break;
        case "Enter":
          e.preventDefault();
          if (filtered[selectedIdx]) {
            onSelect(filtered[selectedIdx].id);
          }
          break;
        case "Escape":
          e.preventDefault();
          onClose();
          break;
      }
    },
    [filtered, selectedIdx, onSelect, onClose]
  );

  return (
    <AnimatePresence>
      {visible && (
        <motion.div
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.12 }}
          className="absolute inset-0 z-50 flex justify-center pt-16 tcld-overlay-scrim"
          onClick={onClose}
        >
          <motion.div
            initial={{ opacity: 0, y: -2, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: -2, scale: 0.98 }}
            transition={{ duration: 0.15, ease: [0.2, 0, 0, 1] }}
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-label="Command palette"
            className={clsx(
              "w-[460px] self-start overflow-hidden",
              "tcld-overlay-surface",
            )}
          >
            <div className="px-4 py-3 border-b border-tcld-border flex items-center gap-2.5">
              <span className="font-mono text-base text-tcld-fg-dim font-semibold">
                ⌘
              </span>
              <input
                ref={inputRef}
                value={query}
                onChange={(e) => {
                  setQuery(e.target.value);
                  setSelectedIdx(0);
                }}
                onKeyDown={handleKeyDown}
                placeholder="Type a command..."
                className={clsx(
                  "flex-1",
                  "font-mono text-sm",
                  "bg-transparent border-none outline-none",
                  "text-tcld-fg placeholder-tcld-fg-dim/50",
                  "caret-tcld-accent"
                )}
              />
              <span className="font-mono text-xs text-tcld-fg-dim px-1.5 py-0.5 rounded bg-tcld-bg-2">
                esc
              </span>
            </div>

            <div className="max-h-96 overflow-auto py-1">
              {filtered.map((item, i) => (
                <div
                  key={item.id}
                  onClick={() => onSelect(item.id)}
                  onMouseEnter={() => setSelectedIdx(i)}
                  className={clsx(
                    "flex items-center px-4 py-1.75 cursor-pointer",
                    "border-l-2 transition-all duration-75",
                    i === selectedIdx
                      ? "bg-tcld-bg-highlight border-l-tcld-border-focus"
                      : "bg-transparent border-l-transparent"
                  )}
                >
                  <span
                    className={clsx(
                      "flex-1",
                      "font-mono text-sm",
                      i === selectedIdx
                        ? "text-tcld-fg font-medium"
                        : "text-tcld-fg-muted font-normal"
                    )}
                  >
                    {highlightMatch(item.title, query)}
                  </span>
                  {item.shortcut?.[0] && (
                    <span className="font-mono text-xs text-tcld-fg-dim bg-tcld-bg-2 px-1.75 py-0.5 rounded border border-tcld-border-dim">
                      {item.shortcut[0]}
                    </span>
                  )}
                </div>
              ))}
              {filtered.length === 0 && (
                <div className="px-4 py-4 font-mono text-sm text-tcld-fg-dim text-center">
                  No matching commands
                </div>
              )}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

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
