"use client";

import React, { useEffect, useRef, useState } from "react";
import clsx from "clsx";
import { ChevronDown, Play, Settings } from "lucide-react";
import { DEBUGGER_UNAVAILABLE_MESSAGE } from "@/lib/capabilities";

export interface LaunchProfile {
  id: string;
  name: string;
}

interface LaunchControlProps {
  profiles: LaunchProfile[];
  selectedProfileId: string | null;
  hookStatus?: string;
  /**
   * Backend capability handshake. When false (backend built without the
   * `debugger` feature) the Run button renders disabled with an explanatory
   * tooltip instead of dispatching a launch. Profile selection and the
   * Configure entry stay usable — editing run configurations is valid
   * without a debugger.
   */
  debuggerAvailable?: boolean;
  onSelectProfile: (id: string | null) => void;
  onRun: () => void;
  onOpenConfigure: () => void;
}

export function LaunchControl({
  profiles,
  selectedProfileId,
  hookStatus,
  debuggerAvailable = true,
  onSelectProfile,
  onRun,
  onOpenConfigure,
}: LaunchControlProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (!open) return;
    const handlePointer = (event: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpen(false);
      }
    };
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("pointerdown", handlePointer);
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("pointerdown", handlePointer);
      document.removeEventListener("keydown", handleKey);
    };
  }, [open]);

  const activeProfile = profiles.find((profile) => profile.id === selectedProfileId);
  const activeName = activeProfile?.name ?? "Ad hoc";

  return (
    <div ref={rootRef} className="tcld-launch-split" data-launch-control="true">
      <button
        type="button"
        data-actionbar-launch-item="run"
        data-launch-unavailable={debuggerAvailable ? undefined : "true"}
        className={clsx(
          "tcld-launch-split-run",
          !debuggerAvailable && "cursor-not-allowed opacity-40",
        )}
        onClick={onRun}
        disabled={!debuggerAvailable}
        title={debuggerAvailable ? "Run / Debug (F5)" : DEBUGGER_UNAVAILABLE_MESSAGE}
      >
        <Play size={12} aria-hidden />
        <span className="tcld-launch-split-run-label">Run</span>
      </button>
      <button
        ref={triggerRef}
        type="button"
        data-actionbar-launch-item="profile"
        data-launch-profile-trigger="true"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label="Run configuration"
        className={clsx("tcld-launch-split-profile", open && "is-open")}
        onClick={() => setOpen((prev) => !prev)}
        title={hookStatus ? `${activeName} · ${hookStatus}` : activeName}
      >
        <span className="tcld-launch-profile-dot" aria-hidden />
        <span className="tcld-launch-split-profile-name">{activeName}</span>
        <ChevronDown size={11} aria-hidden className="tcld-launch-split-profile-chevron" />
      </button>
      {open && (
        <div className="tcld-launch-menu" role="menu" data-launch-menu="true">
          <button
            type="button"
            role="menuitemradio"
            aria-checked={selectedProfileId == null}
            className={clsx("tcld-launch-menu-item", selectedProfileId == null && "is-active")}
            data-launch-menu-item="profile"
            data-launch-menu-id=""
            onClick={() => {
              onSelectProfile(null);
              setOpen(false);
            }}
          >
            <span className="tcld-launch-menu-dot" aria-hidden />
            <span className="tcld-launch-menu-label">Ad hoc</span>
            <span className="tcld-launch-menu-detail">no saved profile</span>
          </button>
          {profiles.map((profile) => (
            <button
              key={profile.id}
              type="button"
              role="menuitemradio"
              aria-checked={profile.id === selectedProfileId}
              className={clsx(
                "tcld-launch-menu-item",
                profile.id === selectedProfileId && "is-active",
              )}
              data-launch-menu-item="profile"
              data-launch-menu-id={profile.id}
              onClick={() => {
                onSelectProfile(profile.id);
                setOpen(false);
              }}
            >
              <span className="tcld-launch-menu-dot" aria-hidden />
              <span className="tcld-launch-menu-label">{profile.name}</span>
            </button>
          ))}
          {hookStatus && (
            <div className="tcld-launch-menu-status" data-launch-menu-status="true">
              {hookStatus}
            </div>
          )}
          <div className="tcld-launch-menu-separator" aria-hidden />
          <button
            type="button"
            role="menuitem"
            data-testid="run-configuration-open-window"
            data-actionbar-launch-item="configure"
            data-launch-menu-item="configure"
            className="tcld-launch-menu-item tcld-launch-menu-configure"
            onClick={() => {
              setOpen(false);
              onOpenConfigure();
            }}
          >
            <Settings size={11} aria-hidden />
            <span className="tcld-launch-menu-label">Configure…</span>
          </button>
        </div>
      )}
    </div>
  );
}
