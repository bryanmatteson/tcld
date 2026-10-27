"use client";

import { useEffect, useState, useRef } from "react";
import clsx from "clsx";
import { ArrowRight, Bug, FileUp, FolderOpen, X } from "lucide-react";
import { useTcldStore } from "@/lib/store";
import { loadRecentFiles, clearRecentFiles, type RecentFile } from "@/lib/recent-files";
import { EXAMPLES, type TcldExample } from "@/lib/examples";
import { formatInstructionCount, formatTclVersion } from "@/lib/format";
import { getOpcodeCategory, opcodeCategoryColor } from "@/lib/opcode-categories";

import { useModalFocus } from "@/hooks/use-modal-focus";

interface WelcomeScreenProps {
  onOpenFile: () => void;
  onLaunchDebug?: () => void;
  onRecentFile: (entry: RecentFile) => void;
  isDropActive?: boolean;
}

function relativeTime(ts: number): string {
  const diff = Date.now() - ts;
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  if (days < 7) return `${days}d ago`;
  return new Date(ts).toLocaleDateString();
}

function truncatePath(path: string, max: number = 50): string {
  if (path.length <= max) return path;
  const parts = path.split("/");
  // Keep first dir + ... + last 2 dirs + filename
  if (parts.length > 4) {
    return parts[0] + "/.../" + parts.slice(-3).join("/");
  }
  return "..." + path.slice(-(max - 3));
}

// A tiny proc and its bytecode, used only as the welcome illustration.
const PREVIEW_SOURCE: { text: string; group: number | null }[] = [
  { text: "proc greet {name} {", group: null },
  { text: "    set msg \"hello, $name\"", group: 1 },
  { text: "    puts $msg", group: 2 },
  { text: "}", group: null },
];

const PREVIEW_BYTECODE: { pc: string; op: string; arg: string; group: number }[] = [
  { pc: "0000", op: "push1", arg: "\"hello, \"", group: 1 },
  { pc: "0002", op: "loadScalar1", arg: "%name", group: 1 },
  { pc: "0004", op: "strcat", arg: "2", group: 1 },
  { pc: "0006", op: "storeScalar1", arg: "%msg", group: 1 },
  { pc: "0008", op: "push1", arg: "\"puts\"", group: 2 },
  { pc: "000a", op: "loadScalar1", arg: "%msg", group: 2 },
  { pc: "000c", op: "invokeStk1", arg: "2 words", group: 2 },
];

const PREVIEW_ACTIVE_GROUP = 1;

/** Decorative linked view: one command highlighted in source and bytecode at
 *  once, which is what the workbench does. Shown only when the hero is wide. */
function WelcomePreview() {
  return (
    <div className="tcld-welcome-preview" aria-hidden>
      <div className="tcld-welcome-preview-pane">
        <div className="tcld-welcome-preview-title">greet.tcl</div>
        {PREVIEW_SOURCE.map((line, idx) => (
          <div key={idx} className="tcld-welcome-preview-line" data-active={line.group === PREVIEW_ACTIVE_GROUP || undefined}>
            <span className="tcld-welcome-preview-gutter">{idx + 1}</span>
            <span>{line.text}</span>
          </div>
        ))}
      </div>
      <div className="tcld-welcome-preview-pane">
        <div className="tcld-welcome-preview-title">Disassembly</div>
        {PREVIEW_BYTECODE.map((row) => (
          <div key={row.pc} className="tcld-welcome-preview-line" data-active={row.group === PREVIEW_ACTIVE_GROUP || undefined}>
            <span className="tcld-welcome-preview-gutter">{row.pc}</span>
            <span style={{ color: opcodeCategoryColor(getOpcodeCategory(row.op)) }}>{row.op}</span>
            <span className="tcld-welcome-preview-arg">{row.arg}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

export function WelcomeScreen({ onOpenFile, onLaunchDebug, onRecentFile, isDropActive = false }: WelcomeScreenProps) {
  const { error, loading, loadExample } = useTcldStore();
  // Start empty so the hydration render matches the server (which has no
  // localStorage); load the persisted list after mount to avoid a mismatch.
  const pickingRef = useRef(false);
  const [recentFiles, setRecentFiles] = useState<RecentFile[]>([]);
  const [examplesOpen, setExamplesOpen] = useState(false);
  const [examplesError, setExamplesError] = useState<string | null>(null);
  const [loadingExampleId, setLoadingExampleId] = useState<string | null>(null);

  useEffect(() => {
    setRecentFiles(loadRecentFiles());
  }, []);

  const handleClearRecent = () => {
    clearRecentFiles();
    setRecentFiles([]);
  };

  const handlePickExample = async (example: TcldExample) => {
    if (pickingRef.current) return;
    pickingRef.current = true;
    setExamplesError(null);
    setLoadingExampleId(example.id);
    try {
      const data = await loadExample(example.id);
      if (data) {
        setExamplesOpen(false);
      } else {
        setExamplesError("The desktop runtime could not load this example.");
      }
    } catch (err) {
      setExamplesError(err instanceof Error ? err.message : String(err));
    } finally {
      pickingRef.current = false;
      setLoadingExampleId(null);
    }
  };

  return (
    <div
      className={clsx("tcld-welcome flex-1", isDropActive && "tcld-welcome-drop-active")}
      data-design-surface="welcome-screen"
      data-parity-state="welcome"
      data-drop-active={isDropActive ? "true" : undefined}
    >
      <section className="tcld-welcome-hero" aria-label="Open a Tcld workspace">
        <div className="tcld-welcome-intro">
        <div className="tcld-welcome-kicker">
          <span className="tcld-product-mark" aria-hidden>t</span>
          <span>Tcl Bytecode Workbench</span>
        </div>

        <h1 className="tcld-welcome-title">Understand your<br />Tcl code.</h1>

        <p className="tcld-welcome-copy">
          Inspect Tcl bytecode, read the reconstructed source, and follow
          execution in a synchronized workspace.
        </p>

        <div className="tcld-welcome-actions">
          <button
            data-testid="welcome-open-file"
            onClick={onOpenFile}
            disabled={loading}
            className={clsx("tcld-hero-action", "tcld-hero-action-primary")}
          >
            <FileUp size={16} aria-hidden />
            <span><strong>Open file</strong><small>Inspect bytecode or reconstruct Tcl source</small></span>
            <ArrowRight size={16} aria-hidden />
          </button>
          <button
            data-testid="welcome-browse-examples"
            type="button"
            className={clsx("tcld-hero-action", "tcld-hero-action-secondary")}
            onClick={() => {
              setExamplesError(null);
              setExamplesOpen(true);
            }}
          >
            <FolderOpen size={15} aria-hidden />
            <span><strong>Explore an example</strong><small>Start with a small, bundled Tcl program</small></span>
            <ArrowRight size={16} aria-hidden />
          </button>
          {onLaunchDebug && (
            <button type="button" data-testid="welcome-debug" className="tcld-hero-action tcld-hero-action-secondary" onClick={onLaunchDebug} disabled={loading}>
              <Bug size={16} aria-hidden />
              <span><strong>Debug a script</strong><small>Run Tcl with breakpoints and live variables</small></span>
              <ArrowRight size={16} aria-hidden />
            </button>
          )}
        </div>

        <p className="tcld-drop-affordance" data-drop-affordance="welcome">
          Drop a <strong>.tbc</strong> or <strong>.tcl</strong> file here
        </p>

        {error && (
          <div role="alert" className="tcld-welcome-error font-mono text-xs text-tcld-error mt-5">
            {error}
          </div>
        )}
        </div>
        <WelcomePreview />
      </section>

      <aside className="tcld-recent-column" aria-label="Recent files">
        <div className="tcld-recent-header">
          <span>Recent</span>
          {recentFiles.length > 0 && (
              <button
                data-testid="recent-clear"
                onClick={handleClearRecent}
              className="tcld-recent-clear"
              >
                Clear
              </button>
          )}
        </div>

        <div className="tcld-recent-list">
          {recentFiles.length > 0 ? (
            recentFiles.map((entry) => (
                <button
                  key={`${entry.path}-${entry.mode}`}
                  data-recent-file-path={entry.path}
                  title={entry.path}
                  onClick={() => onRecentFile(entry)}
                className="tcld-recent-row"
                >
                <span className="tcld-recent-dot" data-mode={entry.mode} />

                <span className="tcld-recent-info">
                  <span className="tcld-recent-name">
                      <strong>{entry.filename}</strong>
                    <span>{entry.mode === "debug" ? "Debug" : "Open"}</span>
                    </span>
                  <span className="tcld-recent-path">
                      {truncatePath(entry.path)}
                    </span>
                  </span>

                <span className="tcld-recent-meta">
                  <span>
                    {relativeTime(entry.timestamp)}
                  </span>
                  <span>{formatInstructionCount(entry.instructionCount)}</span>
                  <span>{formatTclVersion(entry.tclVersion)}</span>
                </span>
                </button>
            ))
          ) : (
            <div className="tcld-recent-empty"><FolderOpen size={24} aria-hidden /><strong>No recent files</strong><span>Files you open will appear here. Try a bundled example to explore the workbench.</span></div>
          )}
        </div>

        <div className="tcld-welcome-tips">
          {[
            { kbd: "⌘O", txt: "Open file" },
            { kbd: "⌘P", txt: "Find a command" },
            { kbd: "⌘D", txt: "Toggle debug layout" },
          ].map((tip) => (
            <div key={tip.kbd} className="tcld-welcome-tip">
              <span>{tip.kbd}</span>
              {tip.txt}
            </div>
          ))}
        </div>
      </aside>

      {examplesOpen && (
        <BrowseExamplesDialog
          error={examplesError ? (error ?? examplesError) : null}
          loadingId={loadingExampleId}
          onPick={handlePickExample}
          onClose={() => setExamplesOpen(false)}
        />
      )}
    </div>
  );
}

interface BrowseExamplesDialogProps {
  error: string | null;
  loadingId: string | null;
  onPick: (example: TcldExample) => void;
  onClose: () => void;
}

function BrowseExamplesDialog({ error, loadingId, onPick, onClose }: BrowseExamplesDialogProps) {
  const dialogRef = useModalFocus(true, onClose);
  return (
    <div
      ref={dialogRef}
      tabIndex={-1}
      role="dialog"
      aria-modal="true"
      aria-label="Browse example Tcl programs"
      className="absolute inset-0 z-50 flex items-center justify-center tcld-overlay-scrim"
      onClick={onClose}
    >
      <div
        className="w-[640px] max-w-[90vw] max-h-[80vh] overflow-hidden flex flex-col tcld-overlay-surface"
        onClick={(event) => event.stopPropagation()}
      >
        <header className="flex items-center justify-between px-5 py-3 border-b border-tcld-border">
          <div>
            <div className="text-sm font-mono font-semibold text-tcld-fg">Bundled examples</div>
            <div className="text-meta font-mono text-tcld-text-inactive mt-0.5">
              Tcl programs that exercise distinct compiler paths.
            </div>
          </div>
          <button
            type="button"
            aria-label="Close examples"
            onClick={onClose}
            className="inline-flex items-center justify-center w-7 h-7 rounded-md text-tcld-text-inactive hover:text-tcld-fg hover:bg-tcld-bg-highlight"
          >
            <X size={14} aria-hidden />
          </button>
        </header>

        <div className="flex-1 overflow-auto p-3 grid grid-cols-1 sm:grid-cols-2 gap-3">
          {EXAMPLES.map((example) => {
            const loading = loadingId === example.id;
            return (
              <button
                key={example.id}
                type="button"
                data-example-id={example.id}
                onClick={() => onPick(example)}
                disabled={loadingId !== null}
                className={clsx(
                  "text-left rounded-md border border-tcld-border-dim bg-tcld-bg-2 p-3",
                  "hover:border-tcld-accent hover:bg-tcld-bg-highlight transition-colors",
                  "disabled:opacity-60 disabled:cursor-wait",
                )}
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="font-mono text-xs font-semibold text-tcld-fg truncate">
                    {loading ? "Loading…" : example.title}
                  </span>
                  <span className="font-mono text-label text-tcld-text-inactive truncate">
                    {example.filename}
                  </span>
                </div>
                <p className="font-mono text-meta text-tcld-text-inactive mt-1 leading-relaxed">
                  {example.description}
                </p>
                <div className="flex flex-wrap gap-1 mt-2">
                  {example.tags.map((tag) => (
                    <span
                      key={tag}
                      className="inline-flex items-center px-1.5 h-4 rounded font-mono text-tag bg-tcld-bg-1 text-tcld-text-keyhint border border-tcld-border-dim"
                    >
                      {tag}
                    </span>
                  ))}
                </div>
              </button>
            );
          })}
        </div>

        {error && (
          <div className="px-5 py-3 border-t border-tcld-border bg-tcld-error/10 font-mono text-xs text-tcld-error">
            {error}
          </div>
        )}
      </div>
    </div>
  );
}
