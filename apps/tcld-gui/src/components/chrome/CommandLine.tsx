"use client";

import clsx from "clsx";
import { Bookmark, Circle, Hash, MessageSquare, Search } from "lucide-react";
import { useState, memo } from "react";

interface CommandLineResult {
  id: string;
  label: string;
  detail?: string;
  icon?: string;
}

interface CommandLineProps {
  visible: boolean;
  value: string;
  results: CommandLineResult[];
  onChange: (value: string) => void;
  onClose: () => void;
  onSubmit: (value: string) => void;
  onPickResult: (resultId: string) => void;
}

function ResultIcon({ result }: { result: CommandLineResult }) {
  const common = { size: 11, "aria-hidden": true } as const;

  if (result.icon === "bookmark") return <Bookmark {...common} />;
  if (result.icon === "#") return <Hash {...common} />;
  if (result.icon === ";") return <MessageSquare {...common} />;
  if (result.icon === "/") return <Search {...common} />;
  if (result.icon) return <span className="font-mono text-label leading-none">{result.icon}</span>;

  return <Circle size={7} fill="currentColor" aria-hidden />;
}

export const CommandLine = memo(function CommandLine({
  visible,
  value,
  results,
  onChange,
  onClose,
  onSubmit,
  onPickResult,
}: CommandLineProps) {
  const [selectedIdx, setSelectedIdx] = useState(0);

  const clampedSelected = Math.max(0, Math.min(selectedIdx, Math.max(results.length - 1, 0)));

  if (!visible) return null;

  // Stable id for the active option so the input's aria-activedescendant can
  // point screen readers at the highlighted result without us having to move
  // DOM focus into the listbox.
  const listboxId = "command-line-results";
  const activeOptionId = results.length > 0 ? `${listboxId}-opt-${clampedSelected}` : undefined;

  return (
    <div className="tcld-command-line bg-tcld-bg-2 border-t border-tcld-border-dim shrink-0">
      {results.length > 0 && (
        <div
          id={listboxId}
          role="listbox"
          aria-label="Command line results"
          className="max-h-48 overflow-y-auto border-b border-tcld-border-dim/60"
        >
          {results.map((result, idx) => {
            const optionId = `${listboxId}-opt-${idx}`;
            const isSelected = idx === clampedSelected;
            return (
              <button
                key={result.id}
                id={optionId}
                type="button"
                role="option"
                aria-selected={isSelected}
                onClick={() => onPickResult(result.id)}
                onMouseEnter={() => setSelectedIdx(idx)}
                className={clsx(
                  "w-full text-left px-4 py-1.5 font-mono text-meta flex items-center gap-2 border-l-2",
                  isSelected
                    ? "bg-tcld-bg-highlight border-l-tcld-border-focus text-tcld-fg"
                    : "bg-transparent border-l-transparent text-tcld-fg-muted hover:bg-tcld-bg-highlight/40",
                )}
              >
                <span className="w-5 text-tcld-fg-dim text-center flex items-center justify-center">
                  <ResultIcon result={result} />
                </span>
                <span className="flex-1 truncate">{result.label}</span>
                {result.detail && (
                  <span className="text-label text-tcld-fg-dim truncate max-w-[40%]">{result.detail}</span>
                )}
              </button>
            );
          })}
        </div>
      )}

      <div className="flex items-center px-4 h-10">
        <span className="font-mono text-base font-bold text-tcld-fg-dim mr-1.5" aria-hidden>
          :
        </span>
        <input
          autoFocus
          value={value}
          // Combobox pattern: the input owns the listbox and reports the
          // currently highlighted option via aria-activedescendant. Screen
          // readers announce the option without us moving DOM focus away
          // from the input, so the user can keep typing while AT speaks
          // each highlighted result.
          role="combobox"
          aria-expanded={results.length > 0}
          aria-controls={results.length > 0 ? listboxId : undefined}
          aria-activedescendant={activeOptionId}
          aria-autocomplete="list"
          onChange={(e) => {
            setSelectedIdx(0);
            onChange(e.target.value);
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") onClose();
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setSelectedIdx((prev) => Math.min(prev + 1, Math.max(results.length - 1, 0)));
            }
            if (e.key === "ArrowUp") {
              e.preventDefault();
              setSelectedIdx((prev) => Math.max(prev - 1, 0));
            }
            if (e.key === "Enter") {
              e.preventDefault();
              if (results.length > 0) {
                onPickResult(results[clampedSelected].id);
              } else {
                onSubmit(value);
              }
            }
          }}
          placeholder="i123 · c042 · b17 · line:220 · pc:0x1a2b · @bookmark · /opcode · #42"
          className={clsx(
            "flex-1",
            "font-mono text-sm",
            "bg-transparent border-none outline-none",
            "text-tcld-fg",
            "placeholder-tcld-fg-dim/50",
            "caret-tcld-accent"
          )}
        />
      </div>
    </div>
  );
});
