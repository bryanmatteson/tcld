"use client";

import React from "react";
import clsx from "clsx";
import {
  AlertTriangle,
  ArrowDownToDot,
  ArrowUpFromDot,
  ChevronsRight,
  Circle,
  CircleOff,
  Copy,
  CornerDownRight,
  Crosshair,
  ListEnd,
  Pause,
  Play,
  RedoDot,
  RotateCcw,
  Square,
  StepForward,
  Terminal,
  X,
} from "lucide-react";
import { useDebugStore } from "@/lib/debug-store";
import { T } from "@/lib/theme";
import { commandRegistry } from "@/lib/command-registry";
import { useLocationStore } from "@/lib/location-store";
import { useShallow } from "zustand/react/shallow";
import { useTcldData } from "@/lib/store";
import { resolveBreakpointTarget } from "@/lib/workflow-contracts";

interface ToolbarButtonProps {
  label: string;
  icon: React.ReactNode;
  shortcut?: string;
  onClick: () => void;
  disabled?: boolean;
  danger?: boolean;
  /** Primary actions also show their label when the toolbar has room. */
  primary?: boolean;
  tone?: "go";
  iconStyle?: React.CSSProperties;
}

function ToolbarButton({ label, icon, shortcut, onClick, disabled, danger, primary, tone, iconStyle }: ToolbarButtonProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={clsx("tcld-debug-btn", danger && "is-danger", tone === "go" && "is-go")}
      title={shortcut ? `${label} (${shortcut})` : label}
      aria-label={label}
    >
      <span className="tcld-debug-btn-icon" style={iconStyle} aria-hidden>{icon}</span>
      {primary && <span className="tcld-debug-btn-label">{label}</span>}
    </button>
  );
}

const STOP_REASON_TEXT: Record<string, string> = {
  breakpoint: "at breakpoint",
  exception: "on exception",
  watchpoint: "on watchpoint",
  step: "after step",
  pause: "by request",
  entry: "at entry",
};

export function DebugToolbar() {
  const debug = useDebugStore();
  const { cursor, setCursor } = useLocationStore(useShallow((s) => ({ cursor: s.cursor, setCursor: s.setCursor })));
  const { data } = useTcldData();
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
  let locationText: string | null = null;
  if (isTerminated) {
    statusText = "Ended";
    statusTone = "neutral";
  } else if (isStopping) {
    statusText = "Stopping…";
    statusTone = "warn";
  } else if (isPausing) {
    statusText = "Pausing…";
    statusTone = "warn";
  } else if (isLaunching) {
    statusText = "Launching…";
    statusTone = "warn";
  } else if (isPaused) {
    statusText = isExceptionStop ? "Paused on exception" : "Paused";
    statusTone = isExceptionStop ? "warn" : "accent";
    if (stoppedAt) {
      const file = stoppedAt.file.split("/").pop() || stoppedAt.file;
      const reason = !isExceptionStop && stoppedAt.reason ? STOP_REASON_TEXT[stoppedAt.reason] : null;
      locationText = `${reason ? `${reason} · ` : ""}${file}:${stoppedAt.line}`;
    }
  } else if (isRunning) {
    statusText = "Running";
    statusTone = "success";
  } else {
    statusText = sessionState;
    statusTone = "neutral";
  }

  const steppingDisabled = !isPaused || isTerminated || isBusy || isExceptionStop || isPausing || isStopping;
  const scriptName = scriptPath ? scriptPath.split("/").pop() : null;
  const stopReasonPanel = stoppedAt?.reason === "breakpoint"
    ? "panel:breakpoints"
    : stoppedAt?.reason === "exception"
      ? "panel:console"
      : stoppedAt?.reason === "watchpoint"
        ? "panel:variables"
        : null;

  return (
    <div
      className={clsx(
        "tcld-debug-toolbar relative flex items-center gap-3 px-3 border-b border-tcld-border-dim bg-tcld-bg-1",
        // Slide-down enter animation so the toolbar appearance signals a
        // session-state transition rather than just inserting a row.
        "animate-[slide-up_0.16s_ease-out]",
      )}
      data-design-surface="debug-paused-overlay"
    >
      {/* Left: session state and where execution stopped */}
      <div className="tcld-debug-status" data-tone={statusTone}>
        <span className="tcld-debug-state">
          <span className="tcld-debug-state-dot" aria-hidden />
          {statusText}{isBusy ? " · syncing…" : ""}
        </span>
        {locationText && (
          <button
            type="button"
            className="tcld-debug-location"
            disabled={!stopReasonPanel}
            onClick={() => {
              if (stopReasonPanel) commandRegistry.execute(stopReasonPanel);
            }}
            title={stoppedAt?.reason ? `Open ${stoppedAt.reason} context panel` : undefined}
          >
            {locationText}
          </button>
        )}
      </div>

      {/* Center: execution controls, grouped run → step → session → breakpoints */}
      <div className="tcld-debug-controls" role="toolbar" aria-label="Debugger controls">
        <div className="tcld-debug-group">
          {isRunning || isPausing ? (
            <ToolbarButton
              label="Pause"
              icon={<Pause size={15} />}
              shortcut="F6"
              onClick={() => runDebugAction("debug:pause", pause)}
              disabled={isTerminated || isBusy || isPausing || isStopping}
              primary
            />
          ) : (
            <ToolbarButton
              label="Continue"
              icon={<Play size={15} />}
              shortcut="F5"
              onClick={() => runDebugAction("debug:continue", resume)}
              disabled={steppingDisabled}
              primary
              tone="go"
            />
          )}
        </div>
        <span className="tcld-debug-sep" aria-hidden />
        <div className="tcld-debug-group" role="group" aria-label="Step">
          <ToolbarButton
            label="Step Over"
            icon={<RedoDot size={15} />}
            shortcut="F10"
            onClick={() => runDebugAction("debug:step_over", stepOver)}
            disabled={steppingDisabled}
            primary
          />
          <ToolbarButton
            label="Step Into"
            icon={<ArrowDownToDot size={15} />}
            shortcut="F11"
            onClick={() => runDebugAction("debug:step_into", stepInto)}
            disabled={steppingDisabled}
            primary
          />
          <ToolbarButton
            label="Step Out"
            icon={<ArrowUpFromDot size={15} />}
            shortcut="⇧F11"
            onClick={() => runDebugAction("debug:step_out", stepOut)}
            disabled={steppingDisabled}
            primary
          />
          <ToolbarButton
            label="Next Group"
            icon={<ListEnd size={15} />}
            shortcut="⇧]"
            onClick={() => runDebugAction("debug:step_next_group", stepToNextGroup)}
            disabled={steppingDisabled}
          />
        </div>
        {canStepInstruction && (
          <>
            <span className="tcld-debug-sep" aria-hidden />
            <div className="tcld-debug-group" role="group" aria-label="Instruction stepping">
              <ToolbarButton
                label="Step Instruction"
                icon={<StepForward size={15} />}
                shortcut="⌥F11"
                onClick={() => runDebugAction("debug:step_instruction", stepInstruction)}
                disabled={steppingDisabled}
              />
              <ToolbarButton
                label="Step Over Instruction"
                icon={<ChevronsRight size={15} />}
                onClick={() => runDebugAction("debug:step_over_instruction", stepOverInstruction)}
                disabled={steppingDisabled}
              />
              <ToolbarButton
                label="Step Out Instruction"
                icon={<CornerDownRight size={15} />}
                onClick={() => runDebugAction("debug:step_out_instruction", stepOutInstruction)}
                disabled={steppingDisabled}
              />
              <ToolbarButton
                label="Run To PC"
                icon={<Crosshair size={15} />}
                onClick={() => {
                  if (cursor.pc != null) {
                    runDebugAction("debug:run_to_pc", () => runToPc(cursor.pc!));
                  }
                }}
                disabled={steppingDisabled || cursor.pc == null}
              />
            </div>
          </>
        )}
        <span className="tcld-debug-sep" aria-hidden />
        <div className="tcld-debug-group" role="group" aria-label="Session">
          <ToolbarButton
            label="Restart"
            icon={<RotateCcw size={15} />}
            shortcut="⌘⇧F5"
            onClick={() => runDebugAction("debug:restart", restart)}
            disabled={isLaunching || isBusy}
          />
          <ToolbarButton
            label="Stop"
            icon={<Square size={14} />}
            shortcut="⇧F5"
            onClick={() => runDebugAction("debug:stop", stop)}
            disabled={isTerminated || isStopping}
            danger
          />
          {isTerminated && (
            <ToolbarButton
              label="Close"
              icon={<X size={15} />}
              onClick={resetSession}
              disabled={false}
            />
          )}
        </div>
        <span className="tcld-debug-sep" aria-hidden />
        <div className="tcld-debug-group" role="group" aria-label="Breakpoints">
          <ToolbarButton
            label="Toggle Breakpoint"
            icon={<Circle size={13} fill="currentColor" />}
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
            label="Clear All Breakpoints"
            icon={<CircleOff size={15} />}
            onClick={() => runDebugAction("debug:clear_breakpoints", clearAllBreakpoints)}
            disabled={breakpoints.length === 0 || isBusy}
          />
        </div>
      </div>

      {/* Right: script identity + breakpoint count */}
      <div className="tcld-debug-meta">
        {breakpoints.length > 0 && (
          <button
            type="button"
            className="tcld-debug-bp-count"
            onClick={() => commandRegistry.execute("panel:breakpoints")}
            title="Show breakpoints"
          >
            <Circle size={8} fill="currentColor" aria-hidden />
            {breakpoints.length} {breakpoints.length === 1 ? "breakpoint" : "breakpoints"}
          </button>
        )}
        {tclVersion && <span>Tcl {tclVersion}</span>}
        {scriptName && (
          <span className="truncate font-mono text-meta" title={scriptPath ?? undefined}>
            {scriptName}
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
