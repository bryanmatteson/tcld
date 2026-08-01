"use client";

import clsx from "clsx";
import { useState } from "react";
import { Command, FileCode2, FilePlus2, X } from "lucide-react";
import { PanelPicker } from "./PanelPicker";
import { isMacOS } from "@/lib/tauri-api";
import type { FileMetadata, ViewId } from "@/types";
import type { SessionDocument } from "@/lib/store-data";

interface FileHeaderProps {
  meta: FileMetadata | null;
  documents: SessionDocument[];
  activeDocumentId: string | null;
  onOpenFile?: () => void;
  onOpenPalette: () => void;
  onSelectDocument: (documentId: string) => void;
  onCloseDocument: (documentId: string) => void;
  openPanels: ViewId[];
  panelCount: number;
  onAddPanel: (type: ViewId) => void;
  /**
   * Compact history controls (back / forward / history popover). Renders in
   * the titlebar trail, alongside the command palette icon and panel picker.
   *
   * Replaces the previous `navBreadcrumbs` prop, which docked a full-width
   * breadcrumb row under the titlebar showing scope info that was already
   * surfaced via panel badges and the StatusBar position counter. Removing
   * that row recovers ~30px of vertical real estate per the UI review.
   */
  navHistoryControls?: React.ReactNode;
  titlebarChrome?: React.ReactNode;
}

export function FileHeader({
  meta,
  documents,
  activeDocumentId,
  onOpenFile,
  onOpenPalette,
  onSelectDocument,
  onCloseDocument,
  openPanels,
  panelCount,
  onAddPanel,
  navHistoryControls,
  titlebarChrome,
}: FileHeaderProps) {
  // Only reserve the macOS overlay traffic-light gutter when we're actually
  // running on macOS — on Windows/Linux it's just dead space.
  // Lazy initializer: detection is sync and idempotent (navigator.platform
  // is stable for the page lifetime), so a single read at first render is
  // correct and avoids the cascading-render pattern of effect-based state.
  const [isMac] = useState<boolean>(() => isMacOS());
  return (
    <div className="tcld-file-header shrink-0 bg-tcld-bg-1" data-design-surface="top-tab-strip" data-tauri-drag-region>
      {/* Title bar row: traffic-light gap | tcld | nav | breadcrumbs | spacer | layout/launch | palette | + */}
      <div className="tcld-titlebar-row" data-tauri-drag-region>
        {isMac && <div className="tcld-titlebar-gap" aria-hidden />}
        <div className="tcld-product-tab">
          <span className="tcld-product-mark" aria-hidden>t</span>
          <span>tcld</span>
        </div>
        <div className="tcld-titlebar-spacer" data-tauri-drag-region />
        {titlebarChrome && (
          <div className="tcld-titlebar-chrome" data-tauri-drag-region={false}>
            {titlebarChrome}
          </div>
        )}
        <div className="tcld-titlebar-trail" data-tauri-drag-region={false}>
          {navHistoryControls}
          <button
            type="button"
            onClick={onOpenPalette}
            className="tcld-command-icon"
            title="Command Palette (⌘P)"
            aria-label="Command Palette"
          >
            <Command size={14} aria-hidden />
          </button>
          <PanelPicker openPanels={openPanels} onAddPanel={onAddPanel} />
        </div>
      </div>

      {/* Tab row: file tabs | search | command | panel picker */}
      <div className="tcld-tabbar">
        <div className="tcld-file-tabs">
          {documents.length > 0 ? documents.map((doc) => {
            const active = doc.id === activeDocumentId;
            const tabMeta = doc.data.metadata;
            return (
              <div
                key={doc.id}
                className={clsx(
                  "tcld-open-file-tab",
                  active ? "tcld-open-file-tab-active" : "tcld-open-file-tab-muted",
                )}
                data-file-id={doc.id}
                data-file-label={doc.label}
                data-active-file={String(active)}
                data-selected-inst={String(doc.workspaceState.selectedInst)}
                data-active-line={doc.workspaceState.activeLine == null ? "" : String(doc.workspaceState.activeLine)}
                data-debug-state={doc.workspaceState.debuggerState.sessionState}
                data-readonly-document={String(doc.readOnly)}
                data-document-dirty="false"
              >
                <button
                  type="button"
                  className="flex min-w-0 flex-1 items-center gap-2 text-left"
                  onClick={() => onSelectDocument(doc.id)}
                  title={doc.label}
                >
                  <span className="tcld-tab-name">{doc.label}</span>
                  {active ? (
                    <>
                      <span className="tcld-tab-meta">Tcl {tabMeta.tclVersion}</span>
                      <span className="tcld-tab-meta">{tabMeta.instructionCount} inst</span>
                      <span className="tcld-tab-meta">{tabMeta.literalCount} lit</span>
                    </>
                  ) : null}
                </button>
                <button
                  type="button"
                  className="rounded p-0.5 text-tcld-fg-dim hover:bg-tcld-bg-highlight hover:text-tcld-fg"
                  data-close-file-tab
                  aria-label={`Close ${doc.label}`}
                  title={`Close ${doc.label}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    onCloseDocument(doc.id);
                  }}
                >
                  <X size={12} aria-hidden />
                </button>
              </div>
            );
          }) : (
            <button
              type="button"
              className="tcld-no-file-tab"
              data-testid="topbar-empty-open-file"
              aria-label="Open Tcl bytecode or source to begin"
              title="Open Tcl bytecode or source to begin"
              onClick={onOpenFile}
              disabled={!onOpenFile}
            >
              <FileCode2 size={12} aria-hidden />
              <span>No file loaded</span>
              <span className="tcld-no-file-tab-separator" aria-hidden />
              <span>Open Tcl bytecode or source</span>
            </button>
          )}
          <button
            type="button"
            className="tcld-tab-open-button"
            data-testid="topbar-open-another-file"
            aria-label={documents.length > 0 ? "Open another Tcl file" : "Open Tcl bytecode or source"}
            title={documents.length > 0 ? "Open another Tcl file" : "Open Tcl bytecode or source"}
            onClick={onOpenFile}
            disabled={!onOpenFile}
          >
            <FilePlus2 size={13} aria-hidden />
          </button>
        </div>

        <div className="tcld-tabbar-spacer" />

        {!meta && (
          <span className="hidden min-w-0 items-center gap-1.5 font-mono text-label text-tcld-fg-dim lg:flex">
            <FileCode2 size={12} aria-hidden />
            Open Tcl bytecode or source to populate the workspace.
          </span>
        )}

        <span className="sr-only" data-status-panel-count={panelCount}>{panelCount} panel{panelCount !== 1 ? "s" : ""}</span>
      </div>
    </div>
  );
}
