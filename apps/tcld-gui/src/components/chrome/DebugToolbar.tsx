"use client";

import React from "react";
import clsx from "clsx";
import {
  AlertTriangle,
  ArrowDown,
  ArrowRight,
  ArrowUp,
  Circle,
  CircleOff,
  Copy,
  CornerDownRight,
  Pause,
  Play,
  RotateCcw,
  RotateCw,
  SkipForward,
  Square,
  Terminal,
  X,
} from "lucide-react";
import { useDebugStore } from "@/lib/debug-store";
import { T } from "@/lib/theme";
import { commandRegistry } from "@/lib/command-registry";
import { useLocationStore } from "@/lib/location-store";
import { useTcldStore } from "@/lib/store";
import { resolveBreakpointTarget } from "@/lib/workflow-contracts";

const BTN =
  "px-2 py-1 rounded text-xs font-mono transition-all duration-150 flex items-center gap-1 flex-shrink-0 select-none";

interface ToolbarButtonProps {
  label: string;
  icon: React.ReactNode;
  shortcut?: string;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
  iconStyle?: React.CSSProperties;
}

function ToolbarButton({ label, icon, shortcut, onClick, disabled, danger, iconStyle }: ToolbarButtonProps) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      className={clsx(
        BTN,
        disabled
          ? "text-tcld-fg-muted cursor-not-allowed opacity-40"
          : danger
            ? "text-tcld-error hover:bg-tcld-error/10"
            : "text-tcld-fg-dim hover:text-tcld-fg hover:bg-tcld-bg-highlight",
      )}
      title={shortcut ? `${label} (${shortcut})` : label}
    >
      <span className="flex h-3.5 w-3.5 items-center justify-center" style={iconStyle}>{icon}</span>
    </button>
  );
}

export function DebugToolbar() {
  const debug = useDebugStore();
  const { cursor, setCursor } = useLocationStore();
  const { data } = useTcldStore();
  const {
    sessionState,
    isBusy,
    stoppedAt,
    scriptPath,
    tclVersion,
    breakpoints,
    canStepInstruction,
    stepInto,
    stepInstruction,
    stepOverInstruction,
    stepOutInstruction,
    runToPc,
    stepOver,
    stepOut,
    stepToNextGroup,
    resume,
    pause,
    stop,
    restart,
    resetSession,
    toggleBreakpoint,
    clearAllBreakpoints,
  } = debug;

  // Don't render if no debug session
  if (sessionState === "idle") return null;

  const isPaused = sessionState === "paused";
  const isRunning = sessionState === "running";
  const isLaunching = sessionState === "launching";
  const isTerminated = sessionState === "terminated";
  const isPausing = sessionState === "pausing";
  const isStopping = sessionState === "stopping";
  const isExceptionStop = isPaused && stoppedAt?.reason === "exception";
  const runDebugAction = (id: string, fallback: () => void) => {
    if (!commandRegistry.execute(id)) fallback();
  };

  // Status text + tone — tones map to Tailwind utility classes from the
  // theme tokens (no inline `style={{ color: T.* }}` per the design-system
  // source-of-truth cleanup).
  type Tone = "neutral" | "warn" | "accent" | "success";
  let statusText: string;
  let statusTone: Tone;
  if (isTerminated) {
    statusText = "Ended";
    statusTone = "neutral";
  } else if (isStopping) {
    statusText = "Stopping...";
    statusTone = "warn";
  } else if (isPausing) {
    statusText = "Pausing...";
    statusTone = "warn";
  } else if (isLaunching) {
    statusText = "Launching...";
    statusTone = "warn";
  } else if (isPaused && stoppedAt) {
    const file = stoppedAt.file.split("/").pop() || stoppedAt.file;
    statusText = `${file}:${stoppedAt.line}`;
    if (stoppedAt.reason === "breakpoint") statusText += " (bp)";
    if (stoppedAt.reason === "exception") statusText += " (exc)";
    statusTone = "accent";
  } else if (isRunning) {
    statusText = "Running";
    statusTone = "success";
  } else {
    statusText = sessionState;
    statusTone = "neutral";
  }

  const toneTextClass: Record<Tone, string> = {
    neutral: "text-tcld-text-inactive",
    warn: "text-tcld-warning",
    accent: "text-tcld-accent",
    success: "text-tcld-success",
  };
  // Glow tones use the canonical --shadow-glow-* tokens. The neutral case
  // intentionally omits the glow so an idle/inactive dot stays quiet.
  const toneDotClass: Record<Tone, string> = {
    neutral: "bg-tcld-text-inactive",
    warn: "bg-tcld-warning shadow-[var(--shadow-glow-warn)]",
    accent: "bg-tcld-accent shadow-[var(--shadow-glow-accent)]",
    success: "bg-tcld-success shadow-[var(--shadow-glow-accent)]",
  };

  return (
    <div
      className={clsx(
        "tcld-debug-toolbar relative flex items-center justify-between px-3 py-1 border-b border-tcld-border-dim bg-tcld-bg-1",
        // Slide-down enter animation so the toolbar appearance signals a
        // session-state transition rather than just inserting a row.
        "animate-[slide-up_0.16s_ease-out]",
      )}
      data-design-surface="debug-paused-overlay"
      style={{ minHeight: 30 }}
    >
      {/* Left: status */}
      <div className="flex items-center gap-2 min-w-0">
        <span
          className={clsx("w-1.5 h-1.5 rounded-full flex-shrink-0", toneDotClass[statusTone])}
          aria-hidden
        />
        <button
          type="button"
          className={clsx(
            "text-xs font-mono truncate rounded px-1 py-0.5",
            toneTextClass[statusTone],
            stoppedAt?.reason ? "hover:bg-tcld-bg-highlight/60" : "pointer-events-none",
          )}
          onClick={() => {
            if (stoppedAt?.reason === "breakpoint") commandRegistry.execute("panel:breakpoints");
            if (stoppedAt?.reason === "exception") commandRegistry.execute("panel:console");
            if (stoppedAt?.reason === "watchpoint") commandRegistry.execute("panel:variables");
          }}
          title={stoppedAt?.reason ? `Open ${stoppedAt.reason} context panel` : undefined}
        >
          {statusText}{isBusy ? " · syncing…" : ""}
        </button>
      </div>

      {/* Center: step controls */}
      <div className="flex items-center gap-0.5">
        <ToolbarButton
          label="Continue"
          icon={<Play size={13} aria-hidden />}
          shortcut="F5"
          onClick={() => runDebugAction("debug:continue", resume)}
          disabled={!isPaused || isTerminated || isBusy || isExceptionStop || isPausing || isStopping}
        />
        <ToolbarButton
          label="Pause"
          icon={<Pause size={13} aria-hidden />}
          shortcut="F6"
          onClick={() => runDebugAction("debug:pause", pause)}
          disabled={!isRunning || isTerminated || isBusy || isPausing || isStopping}
        />
        <div className="w-px h-3 bg-tcld-border-dim mx-1" />
        <ToolbarButton
          label="Step Into"
          icon={<ArrowDown size={13} aria-hidden />}
          shortcut="F11"
          onClick={() => runDebugAction("debug:step_into", stepInto)}
          disabled={!isPaused || isTerminated || isBusy || isExceptionStop || isPausing || isStopping}
        />
        {canStepInstruction && (
          <>
            <ToolbarButton
              label="Step Instruction"
              icon={<ArrowRight size={13} aria-hidden />}
              shortcut="⌥F11"
              onClick={() => runDebugAction("debug:step_instruction", stepInstruction)}
              disabled={!isPaused || isTerminated || isBusy || isExceptionStop || isPausing || isStopping}
            />
            <ToolbarButton
              label="Step Over Instruction"
              icon={<CornerDownRight size={13} aria-hidden />}
              onClick={() => runDebugAction("debug:step_over_instruction", stepOverInstruction)}
              disabled={!isPaused || isTerminated || isBusy || isExceptionStop || isPausing || isStopping}
            />
            <ToolbarButton
              label="Step Out Instruction"
              icon={<ArrowUp size={13} aria-hidden />}
              onClick={() => runDebugAction("debug:step_out_instruction", stepOutInstruction)}
              disabled={!isPaused || isTerminated || isBusy || isExceptionStop || isPausing || isStopping}
            />
            <ToolbarButton
              label="Run To PC"
              icon={<SkipForward size={13} aria-hidden />}
              onClick={() => {
                if (cursor.pc != null) {
                  runDebugAction("debug:run_to_pc", () => runToPc(cursor.pc!));
                }
              }}
              disabled={
                !isPaused
                || isTerminated
                || isBusy
                || isExceptionStop
                || isPausing
                || isStopping
                || cursor.pc == null
              }
            />
          </>
        )}
        <ToolbarButton
          label="Step Over"
          icon={<RotateCw size={13} aria-hidden />}
          shortcut="F10"
          onClick={() => runDebugAction("debug:step_over", stepOver)}
          disabled={!isPaused || isTerminated || isBusy || isExceptionStop || isPausing || isStopping}
        />
        <ToolbarButton
          label="Next Group"
          icon={<SkipForward size={13} aria-hidden />}
          shortcut="⇧]"
          onClick={() => runDebugAction("debug:step_next_group", stepToNextGroup)}
          disabled={!isPaused || isTerminated || isBusy || isExceptionStop || isPausing || isStopping}
        />
        <ToolbarButton
          label="Step Out"
          icon={<ArrowUp size={13} aria-hidden />}
          shortcut="⇧F11"
          onClick={() => runDebugAction("debug:step_out", stepOut)}
          disabled={!isPaused || isTerminated || isBusy || isExceptionStop || isPausing || isStopping}
        />
        <div className="w-px h-3 bg-tcld-border-dim mx-1" />
        <ToolbarButton
          label="Stop"
          icon={<Square size={12} aria-hidden />}
          shortcut="⇧F5"
          onClick={() => runDebugAction("debug:stop", stop)}
          disabled={isTerminated || isStopping}
          danger
        />
        <div className="w-px h-3 bg-tcld-border-dim mx-1" />
        <ToolbarButton
          label="Restart"
          icon={<RotateCcw size={13} aria-hidden />}
          shortcut="⌘⇧F5"
          onClick={() => runDebugAction("debug:restart", restart)}
          disabled={isLaunching || isBusy}
        />
        {isTerminated && (
          <ToolbarButton
            label="Close"
            icon={<X size={13} aria-hidden />}
            onClick={resetSession}
            disabled={false}
          />
        )}
        <div className="w-px h-3 bg-tcld-border-dim mx-1" />
        <ToolbarButton
          label="Breakpoint"
          icon={<Circle size={12} fill="currentColor" aria-hidden />}
          shortcut="F9"
          onClick={() => runDebugAction("debug:toggle_breakpoint", () => {
            const target = resolveBreakpointTarget({
              data,
              selectedInstructionIndex: cursor.instIdx,
              stoppedAt,
              scriptPath,
            });
            if (target) {
              toggleBreakpoint(target.file, target.line);
            }
          })}
          disabled={!isPaused || !stoppedAt || isBusy}
          iconStyle={{ color: (!isPaused || !stoppedAt || isBusy) ? undefined : T.error }}
        />
        <ToolbarButton
          label="Clear All"
          icon={<CircleOff size={13} aria-hidden />}
          onClick={() => runDebugAction("debug:clear_breakpoints", clearAllBreakpoints)}
          disabled={breakpoints.length === 0 || isBusy}
        />
      </div>

      {/* Right: script info + breakpoint count */}
      <div className="flex items-center gap-2 text-xs font-mono text-tcld-fg-muted">
        {breakpoints.length > 0 && (
          <span
            className="px-1.5 py-0.5 rounded text-label font-semibold"
            style={{ color: T.error, background: `color-mix(in srgb, ${T.error} 12%, transparent)` }}
          >
            <Circle size={8} fill="currentColor" aria-hidden /> {breakpoints.length}
          </span>
        )}
        {tclVersion && <span>Tcl {tclVersion}</span>}
        {scriptPath && (
          <span className="truncate max-w-48">
            {scriptPath.split("/").pop()}
          </span>
        )}
      </div>

      {/*
       * Exception banner. Previously this rendered as a floating popup
       * (`absolute left-1/2 top-[34px]`) that overlaid the row below the
       * toolbar — on a narrow window it occluded the very controls the user
       * needed for stepping. Now it's a full-width sticky banner that sits
       * BELOW the toolbar in its own visual lane: own row, own background,
       * primary CTAs aligned right. The toolbar's controls stay reachable.
       */}
      {isExceptionStop && stoppedAt && (
        <div
          role="alert"
          aria-live="polite"
          className="absolute left-0 right-0 top-full z-20 flex items-center gap-2 border-y border-tcld-warning/40 bg-tcld-warning/8 px-3 py-1 animate-slide-up"
          data-debug-exception-banner="true"
          style={{
            // Use inline style for the soft 8% wash so we don't fight the
            // tailwind warning ramp; matches the inline-style rationale of
            // the rest of this toolbar.
            background: `color-mix(in srgb, ${T.warning} 8%, ${T.bg1})`,
          }}
        >
          <span className="flex items-center gap-1.5 text-meta font-mono" style={{ color: T.warning }}>
            <AlertTriangle size={12} aria-hidden /> exception
          </span>
          {stoppedAt.error && (
            <span className="truncate text-meta font-mono text-tcld-fg-dim flex-1 min-w-0">
              {stoppedAt.error}
            </span>
          )}
          <div className="flex items-center gap-1 shrink-0">
            <ToolbarButton
              label="Copy Error"
              icon={<Copy size={13} aria-hidden />}
              onClick={() => {
                const text = [
                  stoppedAt.error ?? "",
                  stoppedAt.errorInfo ?? "",
                ].filter(Boolean).join("\n\n");
                void navigator.clipboard.writeText(text.length > 0 ? text : "No error details available.");
              }}
            />
            <ToolbarButton
              label="Jump Throw Site"
              icon={<CornerDownRight size={13} aria-hidden />}
              onClick={() => {
                setCursor({
                  sourceFile: stoppedAt.file ?? null,
                  sourceLine: stoppedAt.line ?? null,
                });
              }}
            />
            <ToolbarButton
              label="Show Context"
              icon={<Terminal size={13} aria-hidden />}
              onClick={() => {
                commandRegistry.execute("panel:console");
                commandRegistry.execute("panel:variables");
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
}
