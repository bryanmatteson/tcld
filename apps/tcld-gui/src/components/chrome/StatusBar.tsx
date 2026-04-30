"use client";

import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import clsx from "clsx";
import { useTcldStore } from "@/lib/store";
import { useAnchoredFloatingPosition } from "@/lib/floating-position";
import type { ChordHint } from "@/lib/command-registry";

interface PinItem {
  label: string;
  detail?: string;
}

interface MarkItem {
  name: string;
  label: string;
  detail?: string;
}

interface StatusBarProps {
  selected: number;
  trace: boolean;
  statusMessage: string;
  panelCount: number;
  chordHint: ChordHint | null;
  pinA: PinItem | null;
  pinB: PinItem | null;
  markItems: MarkItem[];
  executionScopeMismatch: boolean;
  onJumpPinA: () => void;
  onJumpPinB: () => void;
  onJumpMark: (name: string) => void;
}

// ─── Precedence model ─────────────────────────────────────────
//
// Right-side status-bar slots compete for the same horizontal space.
// We rank them so the most actionable signal always wins, and overflow
// past `MAX_VISIBLE_CHIPS` collapses into a "+N more" chip.
//
//   chord  > error   > warning > pins > info
//
// (chord = transient keystroke continuation; error/warning = exec/diag
// state; pins = user-pinned positions; info = transient status text.)

type ChipTone = "chord" | "err" | "warn" | "info" | "pin-a" | "pin-b" | "mark" | "neutral";

interface StatusChip {
  id: string;
  tone: ChipTone;
  rank: number;        // lower = higher precedence
  label: string;
  detail?: string;
  onClick?: () => void;
  ariaLabel?: string;
}

const MAX_VISIBLE_CHIPS = 4;

const CHIP_TONE_CLASSES: Record<ChipTone, string> = {
  chord:    "border-tcld-accent/50 bg-tcld-accent/12 text-tcld-accent",
  err:      "border-tcld-error/40 bg-tcld-error/10 text-tcld-error",
  warn:     "border-tcld-warning/40 bg-tcld-warning/10 text-tcld-warning",
  info:     "border-tcld-border-dim bg-tcld-bg-2 text-tcld-text-inactive",
  "pin-a":  "border-tcld-warning/30 bg-tcld-warning/10 text-tcld-warning",
  "pin-b":  "border-tcld-number/30 bg-tcld-number/10 text-tcld-number",
  mark:     "border-tcld-border-dim bg-tcld-bg-2 text-tcld-text-inactive",
  neutral:  "border-tcld-border-dim bg-tcld-bg-2 text-tcld-text-inactive",
};

export function StatusBar({
  selected,
  trace,
  statusMessage,
  panelCount,
  chordHint,
  pinA,
  pinB,
  markItems,
  executionScopeMismatch,
  onJumpPinA,
  onJumpPinB,
  onJumpMark,
}: StatusBarProps) {
  const { data } = useTcldStore();
  const instructions = data?.instructions ?? [];
  const totalInstructions = instructions.length;

  // Build the chip set, then sort by precedence and clamp to MAX_VISIBLE_CHIPS.
  const chips: StatusChip[] = [];
  if (chordHint && chordHint.options.length > 0) {
    chips.push({
      id: "chord",
      tone: "chord",
      rank: 0,
      label: `${chordHint.prefix} · ${chordHint.options.map((opt) => `${opt.key} ${opt.title}`).join("  ")}`,
    });
  }
  if (executionScopeMismatch) {
    chips.push({
      id: "exec-scope-mismatch",
      tone: "warn",
      rank: 20,
      label: "exec scope mismatch",
    });
  }
  if (pinA) {
    chips.push({
      id: "pin-a",
      tone: "pin-a",
      rank: 30,
      label: `A ${pinA.label}`,
      detail: pinA.detail ?? "Jump to pin A",
      onClick: onJumpPinA,
      ariaLabel: `Jump to pin A — ${pinA.label}`,
    });
  }
  if (pinB) {
    chips.push({
      id: "pin-b",
      tone: "pin-b",
      rank: 31,
      label: `B ${pinB.label}`,
      detail: pinB.detail ?? "Jump to pin B",
      onClick: onJumpPinB,
      ariaLabel: `Jump to pin B — ${pinB.label}`,
    });
  }
  if (statusMessage) {
    chips.push({
      id: "status-message",
      tone: "info",
      rank: 40,
      label: statusMessage,
      detail: statusMessage,
    });
  }

  chips.sort((a, b) => a.rank - b.rank);

  const visible = chips.slice(0, MAX_VISIBLE_CHIPS);
  const overflow = chips.slice(MAX_VISIBLE_CHIPS);

  return (
    <div className="tcld-status-bar flex bg-tcld-bg-1 border-t border-tcld-border-dim shrink-0" data-design-surface="status-bar">
      {/*
        28px target floor (WCAG 2.5.8) + 4px breathing room.
        Position (iNNN / total) now renders as a subtitle on the
        InstructionPanel / DecompilePanel headers — closer to the cursor it
        describes than a status-bar counter, and it kept the "Position shown
        twice" deduplication called out in the UI review. We still emit the
        sr-only data attributes below so screen readers and external probes
        have a stable hook against StatusBar.
      */}
      <div className="flex w-full items-center px-3 h-8 gap-2 min-w-0">
        <span
          className="sr-only"
          data-status-selected-instruction={String(selected)}
          data-status-instruction-count={String(totalInstructions)}
        >
          {selected + 1}/{totalInstructions || 0}
        </span>

        <div className="flex-1 min-w-0" />

        {/* Right: ranked chips, with overflow */}
        <div className="flex items-center gap-2 min-w-0">
          {markItems.length > 0 && (
            <MarksChip markItems={markItems} onJumpMark={onJumpMark} />
          )}

          {visible.map((chip) => (
            <StatusBarChip key={chip.id} chip={chip} />
          ))}

          {overflow.length > 0 && (
            <OverflowChip overflow={overflow} />
          )}
        </div>

        <span
          className="sr-only"
          data-status-trace={trace ? "true" : "false"}
          data-status-panel-count={panelCount}
        >
          trace {trace ? "on" : "off"} · {panelCount} panel{panelCount !== 1 ? "s" : ""}
        </span>
      </div>
    </div>
  );
}

// ─── Chip primitives ─────────────────────────────────────────

function StatusBarChip({ chip }: { chip: StatusChip }) {
  const className = clsx(
    "inline-flex items-center px-2 h-6 rounded border font-mono text-meta leading-none whitespace-nowrap max-w-[40%] truncate transition-colors",
    CHIP_TONE_CLASSES[chip.tone],
    chip.onClick && "hover:brightness-125 cursor-pointer",
  );
  if (chip.onClick) {
    return (
      <button
        type="button"
        onClick={chip.onClick}
        title={chip.detail ?? chip.label}
        aria-label={chip.ariaLabel ?? chip.label}
        className={className}
      >
        {chip.label}
      </button>
    );
  }
  return (
    <span title={chip.detail ?? chip.label} className={className}>
      {chip.label}
    </span>
  );
}

function OverflowChip({ overflow }: { overflow: StatusChip[] }) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  const pos = useAnchoredFloatingPosition(open, triggerRef, popRef, { placement: "bottom-end" });

  useEffect(() => {
    if (!open) return;
    const onDoc = (event: MouseEvent) => {
      if (!wrapRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  return (
    <div className="relative" ref={wrapRef}>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((prev) => !prev)}
        className="inline-flex items-center px-2 h-6 rounded border border-tcld-border-dim bg-tcld-bg-2 text-tcld-text-inactive font-mono text-meta leading-none hover:text-tcld-fg hover:border-tcld-border"
        title={`+${overflow.length} more status item${overflow.length === 1 ? "" : "s"}`}
        aria-label={`Show ${overflow.length} more status items`}
        aria-expanded={open}
      >
        +{overflow.length} more
      </button>
      {open && typeof document !== "undefined" && createPortal(
        <div
          ref={popRef}
          role="menu"
          className="fixed z-[var(--z-context-menu)] min-w-64 max-w-96 overflow-auto rounded border border-tcld-border bg-tcld-bg-1 shadow-[var(--shadow-popover)]"
          style={{
            left: pos.left,
            top: pos.top,
            maxHeight: pos.maxHeight || 288,
            visibility: pos.ready ? "visible" : "hidden",
          }}
        >
          {overflow.map((chip) => (
            <button
              key={chip.id}
              type="button"
              onClick={() => {
                chip.onClick?.();
                setOpen(false);
              }}
              className="w-full text-left px-2 py-1.5 hover:bg-tcld-bg-highlight border-b border-tcld-border-dim last:border-b-0"
            >
              <div className="font-mono text-xs text-tcld-fg">{chip.label}</div>
              {chip.detail && chip.detail !== chip.label && (
                <div className="font-mono text-label text-tcld-text-inactive mt-0.5">{chip.detail}</div>
              )}
            </button>
          ))}
        </div>,
        document.body,
      )}
    </div>
  );
}

function MarksChip({ markItems, onJumpMark }: { markItems: MarkItem[]; onJumpMark: (name: string) => void }) {
  const [open, setOpen] = useState(false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popRef = useRef<HTMLDivElement>(null);
  const pos = useAnchoredFloatingPosition(open, triggerRef, popRef, { placement: "bottom-end" });

  useEffect(() => {
    if (!open) return;
    const onDoc = (event: MouseEvent) => {
      if (!wrapRef.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [open]);

  return (
    <div className="relative" ref={wrapRef}>
      <button
        ref={triggerRef}
        type="button"
        onClick={() => setOpen((prev) => !prev)}
        className="inline-flex items-center px-2 h-6 rounded border border-tcld-border-dim bg-tcld-bg-2 text-tcld-text-inactive font-mono text-meta leading-none hover:text-tcld-fg hover:border-tcld-border"
        title={`${markItems.length} saved mark${markItems.length === 1 ? "" : "s"}`}
        aria-label={`${markItems.length} saved marks`}
        aria-expanded={open}
      >
        {markItems.length}m
      </button>
      {open && typeof document !== "undefined" && createPortal(
        <div
          ref={popRef}
          className="fixed z-[var(--z-context-menu)] min-w-64 max-w-96 overflow-auto bg-tcld-bg-1 border border-tcld-border rounded shadow-[var(--shadow-popover)]"
          style={{
            left: pos.left,
            top: pos.top,
            maxHeight: pos.maxHeight || 288,
            visibility: pos.ready ? "visible" : "hidden",
          }}
        >
          {markItems.map((item) => (
            <button
              key={item.name}
              type="button"
              onClick={() => {
                onJumpMark(item.name);
                setOpen(false);
              }}
              className="w-full text-left px-2 py-1.5 hover:bg-tcld-bg-highlight border-b border-tcld-border-dim last:border-b-0"
            >
              <div className="font-mono text-xs text-tcld-fg">
                {item.name} · {item.label}
              </div>
              {item.detail && (
                <div className="font-mono text-label text-tcld-text-inactive mt-0.5">{item.detail}</div>
              )}
            </button>
          ))}
        </div>,
        document.body,
      )}
    </div>
  );
}
