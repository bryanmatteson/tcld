"use client";

import React, { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Panel } from "./Panel";
import { useTcldStore } from "@/lib/store";
import { useLocationStore } from "@/lib/location-store";
import { buildSourcePanelModel, sourceMapSelectionTarget } from "@/lib/panel-models";
import { buildSourceFoldModel } from "@/lib/folding-model";
import { tokenizeSourceLine } from "@/lib/syntax-rendering";
import { resolveCmdClickNavigation } from "@/lib/source-navigation";
import { commandRegistry } from "@/lib/command-registry";
import { subscribePanelAction } from "@/lib/panel-action-bus";
import { ChevronDown, ChevronRight, GitBranch, Link2, SplitSquareHorizontal } from "lucide-react";
import { ViewIcon } from "@/components/ViewIcon";

interface SourcePanelProps {
  focused: boolean;
  id: string;
  onClose: () => void;
  onFocus: () => void;
}

export const SourcePanel = React.memo(function SourcePanel({ focused, id, onClose, onFocus }: SourcePanelProps) {
  const { data } = useTcldStore();
  const { cursor, setCursor } = useLocationStore();
  const model = buildSourcePanelModel(data, cursor);
  const [foldedIds, setFoldedIds] = useState<Set<string>>(new Set());
  const foldModel = useMemo(
    () => data ? buildSourceFoldModel(data, { foldedIds, selectedSourceLine: cursor.sourceLine }) : null,
    [cursor.sourceLine, data, foldedIds],
  );

  // Preserve scroll position across fold-toggles. Without this, changing
  // foldedIds rebuilds the row keys and the scroll container resets to top
  // — clicking a fold triangle would yank the user back to line 0.
  //
  // The actual scroll container is .tcld-panel-body (Panel.tsx wraps every
  // panel's children in an overflow-y-auto div). bodyRef points at our
  // inner .tcld-source-body element; we walk up to the nearest scrollable
  // ancestor each time so layout changes inside the panel don't lose scroll.
  const bodyRef = useRef<HTMLDivElement | null>(null);
  const pendingScrollRef = useRef<number | null>(null);
  const findScroller = useCallback((): HTMLElement | null => {
    let node: HTMLElement | null = bodyRef.current;
    while (node) {
      const style = node.style.overflowY || getComputedStyle(node).overflowY;
      if (style === "auto" || style === "scroll" || node.classList.contains("tcld-panel-body")) {
        return node;
      }
      node = node.parentElement;
    }
    return bodyRef.current;
  }, []);
  const updateFoldedIds = useCallback((updater: (prev: Set<string>) => Set<string>) => {
    pendingScrollRef.current = findScroller()?.scrollTop ?? 0;
    setFoldedIds(updater);
  }, [findScroller]);
  useLayoutEffect(() => {
    if (pendingScrollRef.current == null) return;
    const scroller = findScroller();
    if (scroller) scroller.scrollTop = pendingScrollRef.current;
    pendingScrollRef.current = null;
  });

  const setAllFolds = useCallback((folded: boolean) => {
    if (!foldModel) return;
    updateFoldedIds(() => folded ? new Set(foldModel.foldRegions.map((region) => region.id)) : new Set());
  }, [foldModel, updateFoldedIds]);

  useEffect(() => {
    const unsubscribeFold = subscribePanelAction("source:fold-all", () => setAllFolds(true));
    const unsubscribeUnfold = subscribePanelAction("source:unfold-all", () => setAllFolds(false));
    return () => {
      unsubscribeFold();
      unsubscribeUnfold();
    };
  }, [setAllFolds]);

  const selectedMapRow = model.sourceMapRows.find((row) => row.selected) ?? model.sourceMapRows[0];
  const statusSourceLine = cursor.sourceLine != null
    ? cursor.sourceLine + 1
    : selectedMapRow?.entry.sourceLines[0] ?? null;
  const statusInstruction = cursor.instIdx ?? model.selection.instructionIndex;

  // Crumbs reflect live selection. Fallback chain (most-specific first):
  //   1. resolveSourceMapSelection (uses cursor.instIdx, .group, .pc, .sourceLine)
  //   2. source-map row containing cursor.sourceLine
  //   3. selectedMapRow (cursor-selected, else the first row)
  //   4. instruction.group / instruction.pc from cursor.instIdx
  //   5. en-dash placeholder
  const sourceMap = data?.sourceMap ?? [];
  const cursorMapRow = cursor.sourceLine != null
    ? sourceMap.find((entry) =>
        entry.sourceLines[0] <= cursor.sourceLine! && cursor.sourceLine! < entry.sourceLines[1])
    : null;
  const cursorInstruction = cursor.instIdx != null && data
    ? data.instructions[cursor.instIdx]
    : null;

  const selectedGroup =
    model.selection.commandGroup
    ?? cursorMapRow?.commandGroup
    ?? selectedMapRow?.entry.commandGroup
    ?? cursorInstruction?.group
    ?? null;
  const selectedPc =
    cursor.pc
    ?? cursorMapRow?.bytecodeRange[0]
    ?? selectedMapRow?.entry.bytecodeRange[0]
    ?? cursorInstruction?.pc
    ?? null;

  const groupCrumb = selectedGroup != null
    ? `c${String(selectedGroup).padStart(3, "0")}`
    : "c —";
  const pcCrumb = selectedPc != null
    ? `pc 0x${selectedPc.toString(16).padStart(4, "0")}`
    : "pc —";

  return (
    <Panel id={id} title="Source Map" icon={<ViewIcon viewId="source" />} badge={model.available ? String(model.lines.length) : "unavailable"} focused={focused} designSurface="view-source-map" onClose={onClose} onFocus={onFocus}>
      <div className="tcld-source-editor">
        <div className="tcld-source-tabs">
          <span className="tcld-source-file">{data?.metadata.filename ?? "source"}</span>
          <span className="tcld-source-crumb">source</span>
          <span className="tcld-source-crumb" data-source-crumb="group">{groupCrumb}</span>
          <span className="tcld-source-crumb" data-source-crumb="pc">{pcCrumb}</span>
          <span className="tcld-source-engine">tree-sitter-tcl</span>
        </div>

        <div className="tcld-source-toolbar">
          <button onClick={() => commandRegistry.execute("source:fold-all")}>fold all</button>
          <button onClick={() => commandRegistry.execute("source:unfold-all")}>unfold all</button>
          <span>{foldModel ? `${foldedIds.size}/${foldModel.foldRegions.length} folded` : "0/0 folded"}</span>
          <span className="tcld-source-toolbar-spacer" />
          <Link2 size={12} aria-hidden />
          <SplitSquareHorizontal size={12} aria-hidden />
        </div>

        <div className="tcld-source-body" ref={bodyRef}>
          {model.available && foldModel ? (
            foldModel.visibleRows.map((row) => {
              if (row.kind === "fold") {
                return (
                  <button
                    key={row.id}
                    type="button"
                    className="tcld-source-fold-row"
                    data-fold-id={row.id}
                    data-fold-disclosure="command"
                    data-fold-state="collapsed"
                    aria-label={`Unfold ${row.commandPath}`}
                    title={`Unfold ${row.commandPath}`}
                    onClick={() => updateFoldedIds((prev) => {
                      const next = new Set(prev);
                      next.delete(row.id);
                      return next;
                    })}
                  >
                    <span className="tcld-source-fold-button" aria-hidden>
                      <ChevronRight size={12} />
                    </span>
                    <span>{row.hiddenLineCount} lines</span>
                    <span>{row.commandPath}</span>
                    <span>{row.snippet}</span>
                  </button>
                );
              }
              const text = model.lines[row.lineIndex] ?? row.text;
              const selected = cursor.sourceLine === row.lineIndex || selectedMapRow?.entry.sourceLines.includes(row.lineIndex + 1);
              const foldRegion = foldModel.foldRegions.find((candidate) => candidate.start === row.lineIndex);
              return (
                <div
                  key={row.id}
                  data-source-line={row.lineIndex}
                  data-foldable={foldRegion ? "true" : undefined}
                  className={selected ? "tcld-source-line tcld-source-line-selected" : "tcld-source-line"}
                  onClick={(event) => {
                    // ⌘+click → resolve the token under the cursor and navigate
                    // (cmd-click-to-symbol). Regular click → select the line so
                    // the cursor / highlight follows the user, regardless of
                    // whether the click landed on text or empty trailing space.
                    if (event.metaKey && data) {
                      const result = resolveCmdClickNavigation(data, {
                        tokenText: window.getSelection()?.toString() || text.trim().split(/\s+/)[0] || "",
                        sourceLine: row.lineIndex,
                        column: 0,
                      });
                      if (result.ok) {
                        setCursor(result.target, { source: "user" });
                        return;
                      }
                    }
                    setCursor({ sourceLine: row.lineIndex }, { source: "user" });
                  }}
                >
                  {foldRegion ? (
                    <button
                      type="button"
                      className="tcld-source-fold-button"
                      onClick={(event) => {
                        event.stopPropagation();
                        updateFoldedIds((prev) => new Set(prev).add(foldRegion.id));
                      }}
                      data-fold-disclosure="command"
                      data-fold-state="expanded"
                      aria-label={`Fold ${foldRegion.label}`}
                      title={`Fold ${foldRegion.label}`}
                    >
                      <ChevronDown size={12} aria-hidden />
                    </button>
                  ) : (
                    <span className="tcld-source-fold-button" aria-hidden />
                  )}
                  <span className="tcld-source-line-number">{row.lineIndex + 1}</span>
                  <span className="tcld-source-code">
                    {tokenizeSourceLine(text, []).map((token, index) => (
                      <span key={`${row.id}:${index}`} className={token.className} style={{ color: `var(${token.themeToken})` }}>
                        {token.text}
                      </span>
                    ))}
                  </span>
                </div>
              );
            })
          ) : (
            <div className="tcld-source-empty">{model.unavailableMessage}</div>
          )}
        </div>

        <div className="tcld-source-status">
          <span>Ln {statusSourceLine ?? "-"}</span>
          <span>Col 1</span>
          <span>i{statusInstruction ?? "-"}</span>
          <span className="tcld-source-status-spacer" />
          {selectedMapRow ? (
            <button
              type="button"
              className="tcld-source-map-chip"
              onClick={() => {
                const target = data ? sourceMapSelectionTarget(data, selectedMapRow.index) : null;
                if (target) setCursor(target, { source: "user" });
              }}
            >
              <GitBranch size={10} aria-hidden />
              bc {selectedMapRow.entry.bytecodeRange[0]}..{selectedMapRow.entry.bytecodeRange[1]}
            </button>
          ) : null}
          <span>Tcl</span>
        </div>
      </div>
    </Panel>
  );
});
