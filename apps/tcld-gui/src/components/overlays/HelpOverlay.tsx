"use client";

import clsx from "clsx";
import { useMemo, useEffect, useState, memo } from "react";
import { commandRegistry, formatShortcutLabel, type CommandDef, type CommandContext } from "@/lib/command-registry";

import { useModalFocus } from "@/hooks/use-modal-focus";
import { usePresence } from "@/hooks/use-presence";

/** Matches the CSS exit animation on .tcld-overlay-anim / .tcld-overlay-dialog. */
const OVERLAY_EXIT_MS = 120;

interface HelpOverlayProps {
  visible: boolean;
  commandContext: CommandContext;
  onClose: () => void;
}

export const HelpOverlay = memo(function HelpOverlay({ visible, commandContext, onClose }: HelpOverlayProps) {
  const dialogRef = useModalFocus(visible, onClose);
  const mounted = usePresence(visible, OVERLAY_EXIT_MS);
  const [commands, setCommands] = useState<CommandDef[]>(() => commandRegistry.getForContext(commandContext));
  useEffect(() => {
    return commandRegistry.subscribe(() => setCommands(commandRegistry.getForContext(commandContext)));
  }, [commandContext]);

  useEffect(() => {
    setCommands(commandRegistry.getForContext(commandContext));
  }, [commandContext]);

  const sections = useMemo(() => {
    const byCategory = new Map<string, CommandDef[]>();
    for (const command of commands) {
      if (!command.shortcut || command.shortcut.length === 0) continue;
      if (!byCategory.has(command.category)) byCategory.set(command.category, []);
      byCategory.get(command.category)!.push(command);
    }

    const preferredOrder = ["Navigation", "Panels", "Actions", "Debug", "General", "File", "Settings", "Debug Settings"];
    const sortedCategories = [...byCategory.keys()].sort((a, b) => {
      const ia = preferredOrder.indexOf(a);
      const ib = preferredOrder.indexOf(b);
      if (ia >= 0 && ib >= 0) return ia - ib;
      if (ia >= 0) return -1;
      if (ib >= 0) return 1;
      return a.localeCompare(b);
    });

    const keyboardSections = sortedCategories.map((category) => ({
      title: category,
      items: (byCategory.get(category) ?? [])
        .sort((a, b) => a.title.localeCompare(b.title))
        .map((command) => [formatShortcutLabel(command.shortcut?.[0] ?? ""), command.title] as [string, string]),
    }));

    // Mouse / drag interactions are not commandRegistry shortcuts — they're
    // direct DOM gestures handled inside MosaicWorkspace, FileHeader, and
    // the panel headers. Documenting them here is the only place the user
    // can discover the workspace's drag-to-split affordance without trying
    // it blind. Keep this list short and grouped so it doesn't drown the
    // keyboard table.
    const mouseSection = {
      title: "Mouse & Drag",
      items: [
        ["drag", "Reorder tabs in a panel"] as [string, string],
        ["drag→edge", "Drop on a panel edge to split"] as [string, string],
        ["drag→center", "Drop on a panel center to add as a tab"] as [string, string],
        ["right-click", "Context menu on rows / gutters"] as [string, string],
      ],
    };

    return [...keyboardSections, mouseSection];
  }, [commands]);

  return (
    mounted && (
      <div
        data-state={visible ? "open" : "closed"}
        className="absolute inset-0 z-50 flex items-center justify-center tcld-overlay-scrim tcld-overlay-anim"
        onClick={onClose}
      >
        <div
          data-state={visible ? "open" : "closed"}
          onClick={(e) => e.stopPropagation()}
          ref={dialogRef}
          tabIndex={-1}
          aria-modal="true"
          role="dialog"
          aria-label="Keyboard reference"
          className={clsx(
            "w-[720px] max-w-[calc(100vw-32px)] max-h-[calc(100dvh-32px)] overflow-hidden flex flex-col",
            "tcld-overlay-surface tcld-overlay-dialog",
          )}
        >
          <div className="px-5 py-3.5 border-b border-tcld-border flex items-center">
            <span className="font-ui text-base font-semibold text-tcld-fg tracking-tight">
              Keyboard Reference
            </span>
            <div className="flex-1" />
            <button type="button" onClick={onClose} aria-label="Close keyboard reference" className="font-mono text-xs text-tcld-fg-dim rounded px-2 py-1 hover:bg-tcld-bg-highlight">
              <KeyBadge label="Esc" /> Close
            </button>
          </div>

          <div className="grid grid-cols-1 sm:grid-cols-2 min-h-0 max-h-[60vh] overflow-y-auto">
            {sections.map((section, si) => (
              <div
                key={section.title}
                className={clsx(
                  "px-4.5 py-3.5 border-b border-tcld-border-dim",
                  si % 2 === 0 ? "border-r border-tcld-border-dim" : "",
                )}
              >
                <div className="font-ui text-xs font-semibold text-tcld-fg-muted mb-2.5 pb-1.5 border-b border-tcld-border-dim tracking-tight">
                  {section.title}
                </div>

                {section.items.map(([key, desc]) => (
                  <div
                    key={`${key}:${desc}`}
                    className="flex items-start min-h-6.5 py-1 gap-2"
                  >
                    <KeyBadge label={key} />
                    <span className="font-ui text-xs text-tcld-fg-muted">
                      {desc}
                    </span>
                  </div>
                ))}
              </div>
            ))}
          </div>
        </div>
      </div>
    )
  );
});

function KeyBadge({ label }: { label: string }) {
  return (
    <span
      className={clsx(
        "font-mono text-xs",
        "text-tcld-variable",
        "bg-tcld-bg-2 border border-tcld-border-dim",
        "px-1.75 py-0.5 rounded",
        "min-w-[28px] text-center",
        "inline-block flex-shrink-0"
      )}
    >
      {label}
    </span>
  );
}
