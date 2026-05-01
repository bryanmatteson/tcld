"use client";

import React, { useRef, useEffect, useState, useCallback, useMemo } from "react";
import clsx from "clsx";
import { Panel } from "./Panel";
import { useDebugStore, type OutputLine } from "@/lib/debug-store";
import { useTcldStore } from "@/lib/store";
import { T } from "@/lib/theme";
import { ArrowDown, CornerDownLeft, Search } from "lucide-react";
import { createOutputSearchState, formatOutputForCopy, searchOutput } from "@/lib/output-search";
import { ViewIcon } from "@/components/ViewIcon";

// ─── Stream filter config ────────────────────────────────────────

const ALL_STREAMS = ["stdout", "stderr", "eval", "error", "system", "diag"] as const;

const STREAM_LABELS: Record<string, string> = {
  stdout: "out",
  stderr: "err",
  eval: "eval",
  error: "error",
  system: "sys",
  diag: "diag",
};

const CONSOLE_PREVIEW_TIMESTAMP = Date.UTC(2026, 0, 1, 14, 2, 11);

// ─── Output line component ──────────────────────────────────────
// Workbench-mock `.console-row` shape: 56px time · 18px level glyph · text.
// Stream identity collapses into the level glyph + tone class, matching the
// mock's vocabulary (info/ok/warn/err). The full stream name is preserved
// in `data-stream` for parity probes and the stream-filter chips above.

type ConsoleLevel = "ok" | "warn" | "err" | "info";

interface StreamPresentation {
  glyph: string;
  level: ConsoleLevel;
}

const STREAM_PRESENTATION: Record<string, StreamPresentation> = {
  stdout: { glyph: "i", level: "info" },
  stderr: { glyph: "⚠", level: "warn" },
  eval:   { glyph: "▶", level: "ok" },
  error:  { glyph: "✕", level: "err" },
  system: { glyph: "i", level: "info" },
  diag:   { glyph: "⚠", level: "warn" },
};

function OutputLineRow({ line }: { line: OutputLine }) {
  const presentation = STREAM_PRESENTATION[line.stream] ?? { glyph: "·", level: "info" as ConsoleLevel };
  const time = new Date(line.timestamp).toLocaleTimeString("en-US", {
    hour12: false,
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
  const isError = line.stream === "stderr" || line.stream === "error";

  return (
    <div
      className={clsx("console-row", isError && "is-error")}
      data-stream={line.stream}
      data-console-level={presentation.level}
    >
      <span className="t">{time}</span>
      <span className={clsx("lvl", presentation.level)} aria-hidden>
        {presentation.glyph}
      </span>
      <span className="msg">{line.text}</span>
    </div>
  );
}

// ─── Panel ──────────────────────────────────────────────────────

interface ConsolePanelProps {
  focused: boolean;
  id: string;
  onClose: () => void;
  onFocus: () => void;
}

export const ConsolePanel = React.memo(function ConsolePanel({
  focused,
  id,
  onClose,
  onFocus,
}: ConsolePanelProps) {
  const debug = useDebugStore();
  const data = useTcldStore().data;
  const { output, evalHistory, sessionState, evalScript, clearOutput } = debug;

  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const [inputValue, setInputValue] = useState("");
  const [historyIndex, setHistoryIndex] = useState(-1);
  const isAtBottomRef = useRef(true);
  const [showJumpToBottom, setShowJumpToBottom] = useState(false);
  const prevOutputLenRef = useRef(0);
  const [visibleStreams, setVisibleStreams] = useState<Set<string>>(
    new Set(ALL_STREAMS),
  );
  const [searchQuery, setSearchQuery] = useState("");
  const [filterMatches, setFilterMatches] = useState(false);

  const isActive = sessionState === "launching" || sessionState === "connected" || sessionState === "paused" || sessionState === "running";
  const isPaused = sessionState === "paused";

  const previewOutput = useMemo<OutputLine[]>(() => {
    if (output.length > 0 || !data) return output;
    const meta = data.metadata;
    const diagnostics = data.diagnostics;
    const now = CONSOLE_PREVIEW_TIMESTAMP;
    const line = (stream: OutputLine["stream"], text: string, offset = 0): OutputLine => ({
      stream,
      text,
      timestamp: now + offset,
    });
    return [
      line("system", `loaded ${meta.filename} · ${meta.instructionCount} inst · ${meta.commandCount} cmds`),
      line("system", `tcl ${meta.tclVersion} · parser ${meta.syntaxEngine}`, 1),
      line("eval", "package require tcld::analysis", 2),
      line("stdout", `reconstruction=${data.reconstruction} blocks=${data.blocks.length} literals=${data.literals.length}`, 3),
      ...(diagnostics?.passDecisions ?? []).slice(0, 6).map((decision, index) =>
        line("diag", `pass ${decision.name} iter=${decision.iteration} enabled=${decision.enabled} changed=${decision.changed}`, 4 + index),
      ),
      ...(diagnostics?.unsupportedInstructions ?? []).slice(0, 3).map((instruction, index) =>
        line("stderr", `unsupported instruction: ${instruction}`, 20 + index),
      ),
    ];
  }, [data, output]);

  const streamFilteredOutput = previewOutput.filter((l) => visibleStreams.has(l.stream));
  const outputSearch = searchOutput(streamFilteredOutput, createOutputSearchState(searchQuery, { filterToMatches: filterMatches }));
  const filteredOutput = outputSearch.visibleLines;

  const toggleStream = (stream: string) => {
    setVisibleStreams((prev) => {
      const next = new Set(prev);
      if (next.has(stream)) next.delete(stream);
      else next.add(stream);
      return next;
    });
  };

  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const atBottom = el.scrollTop + el.clientHeight >= el.scrollHeight - 20;
    isAtBottomRef.current = atBottom;
    setShowJumpToBottom(!atBottom && previewOutput.length > 0);
  }, [previewOutput.length]);

  // Auto-scroll to bottom on new output (only if user is at bottom)
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    let timer: number | null = null;
    if (prevOutputLenRef.current === 0 && previewOutput.length > 0) {
      isAtBottomRef.current = true;
      timer = window.setTimeout(() => setShowJumpToBottom(false), 0);
    }
    if (isAtBottomRef.current) {
      el.scrollTop = el.scrollHeight;
    }
    prevOutputLenRef.current = previewOutput.length;
    return () => {
      if (timer != null) window.clearTimeout(timer);
    };
  }, [previewOutput.length]);

  const handleSubmit = useCallback(() => {
    const script = inputValue.trim();
    if (!script || !isPaused) return;
    evalScript(script);
    setInputValue("");
    setHistoryIndex(-1);
  }, [inputValue, isPaused, evalScript]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      // Prevent global shortcuts (F5, F10, etc.) from firing while typing
      e.stopPropagation();
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSubmit();
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        if (evalHistory.length === 0) return;
        const newIdx = historyIndex < evalHistory.length - 1 ? historyIndex + 1 : historyIndex;
        setHistoryIndex(newIdx);
        setInputValue(evalHistory[evalHistory.length - 1 - newIdx] || "");
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        if (historyIndex <= 0) {
          setHistoryIndex(-1);
          setInputValue("");
        } else {
          const newIdx = historyIndex - 1;
          setHistoryIndex(newIdx);
          setInputValue(evalHistory[evalHistory.length - 1 - newIdx] || "");
        }
      }
    },
    [handleSubmit, evalHistory, historyIndex],
  );

  return (
    <Panel
      designSurface="view-console"
      id={id}
      title="Console"
      icon={<ViewIcon viewId="console" />}
      focused={focused}
      onClose={onClose}
      onFocus={onFocus}
    >
      <div className="tcld-console-view">
        {/* Stream filter bar */}
        <div className="tcld-console-toolbar">
          {ALL_STREAMS.map((stream) => {
            const active = visibleStreams.has(stream);
            return (
              <button
                key={stream}
                onClick={() => toggleStream(stream)}
                data-stream={stream}
                data-stream-active={active ? "true" : "false"}
                className={active ? "tcld-console-chip tcld-console-chip-active" : "tcld-console-chip"}
              >
                <span />
                {STREAM_LABELS[stream]}
              </button>
            );
          })}

          <div className="tcld-console-filter">
            <Search size={10} aria-hidden />
            <input
              value={searchQuery}
              placeholder="Filter..."
              onChange={(e) => setSearchQuery(e.target.value)}
            />
            {searchQuery && <span>{outputSearch.matches.length} matches</span>}
          </div>
          <span className="tcld-console-toolbar-spacer" />
          <button
            className={filterMatches ? "tcld-console-action tcld-console-action-active" : "tcld-console-action"}
            onClick={() => setFilterMatches((value) => !value)}
          >
            matches
          </button>
          <button
            className="tcld-console-action"
            onClick={() => void navigator.clipboard.writeText(window.getSelection()?.toString() || "")}
          >
            copy selected
          </button>
          <button
            className="tcld-console-action"
            onClick={() => void navigator.clipboard.writeText(formatOutputForCopy(filteredOutput))}
          >
            copy visible
          </button>
          <button
            className="tcld-console-action"
            onClick={clearOutput}
          >
            clear
          </button>
        </div>

        {/* Output area */}
        <div ref={scrollRef} onScroll={handleScroll} className="tcld-console-output">
          {filteredOutput.length > 0 ? (
            filteredOutput.map((line, i) => <OutputLineRow key={i} line={line} />)
          ) : previewOutput.length > 0 ? (
            <div className="tcld-console-empty" data-console-empty="filter">No entries match your filter.</div>
          ) : (
            <div className="tcld-console-empty" data-console-empty={isActive ? "active-empty" : sessionState === "terminated" ? "terminated" : "inactive"}>
              {isActive ? "Waiting for output..." : sessionState === "terminated" ? "Session ended" : "No debug session active"}
            </div>
          )}
        </div>

        {showJumpToBottom && (
          <button
            onClick={() => {
              scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" });
              isAtBottomRef.current = true;
              setShowJumpToBottom(false);
            }}
            className="tcld-console-latest"
          >
            <ArrowDown size={11} aria-hidden /> latest
          </button>
        )}

        {/* Input area */}
        <div className={isPaused ? "tcld-console-input tcld-console-input-paused" : "tcld-console-input"}>
          <span
            style={{ color: isPaused ? T.accent : T.fgMuted }}
          >
            {isPaused ? "(paused)>" : ">"}
          </span>
          <input
            ref={inputRef}
            type="text"
            value={inputValue}
            onChange={(e) => {
              setInputValue(e.target.value);
              setHistoryIndex(-1);
            }}
            onKeyDown={handleKeyDown}
            disabled={!isPaused}
            placeholder={isPaused ? "eval in paused interpreter..." : sessionState === "terminated" ? "Session ended" : "type a command..."}
          />
          <span className="tcld-console-enter">
            <CornerDownLeft size={11} aria-hidden />
            eval
          </span>
        </div>

        <div className="tcld-console-footer">
          <span>{filteredOutput.length} entries</span>
          <span>·</span>
          <span>{visibleStreams.size}/{ALL_STREAMS.length} channels</span>
          {searchQuery && <span>{`· filter: "${searchQuery}"`}</span>}
          <span className="tcld-console-toolbar-spacer" />
          <span>history: {evalHistory.length}</span>
        </div>
      </div>
    </Panel>
  );
});
