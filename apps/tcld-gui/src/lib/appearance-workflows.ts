import type { Settings } from "@/lib/settings";
import type { DensityMode } from "@/lib/settings";
import type { FontId, ThemeId } from "@/lib/themes";
import type { RenderToken } from "@/lib/syntax-rendering";
import { tokenizeInstructionRow, tokenizeSourceLine } from "@/lib/syntax-rendering";

export interface AppearanceUpdate {
  theme?: ThemeId;
  fontFamily?: FontId;
  fontSize?: number;
  density?: DensityMode;
}

export interface AppearancePreviewModel {
  sourceTokens: RenderToken[];
  instructionTokens: RenderToken[];
  graphTokens: {
    nodeRoles: Record<string, string>;
    edgeRoles: Record<string, string>;
  };
}

export function updateAppearanceSettings(settings: Settings, update: AppearanceUpdate): Settings {
  return {
    ...settings,
    theme: update.theme ?? settings.theme,
    fontFamily: update.fontFamily ?? settings.fontFamily,
    fontSize: update.fontSize ?? settings.fontSize,
    density: update.density ?? settings.density,
  };
}

export function buildAppearancePreviewModel(settings: Settings): AppearancePreviewModel {
  const selectedNodeToken = settings.theme === "tokyo-night-light"
    ? "--color-tcld-accent"
    : "--color-tcld-accent";
  return {
    sourceTokens: tokenizeSourceLine("if {$x} { puts \"yes\" } # branch", []),
    instructionTokens: tokenizeInstructionRow({
      idx: 0,
      pc: 0,
      op: "jumpFalse1",
      args: "8",
      operands: [8],
      argc: 1,
      group: 0,
      commandPcRange: [0, 2],
      groupStart: true,
      comment: "",
      size: 2,
      isJump: true,
      isJumpConditional: true,
      isJumpUnconditional: false,
      jumpTarget: 8,
      fallthrough: 2,
      primaryNextLocation: 8,
      secondaryNextLocation: 2,
      nextLocations: [8, 2],
    }, { groupLabel: "c000", comment: "branch", execution: true }),
    graphTokens: {
      nodeRoles: {
        selected: selectedNodeToken,
        execution: "--color-tcld-success",
        predecessor: "--color-tcld-group",
        successor: "--color-tcld-group",
      },
      edgeRoles: {
        fallthrough: "--color-tcld-fg-dim",
        unconditional: "--color-tcld-accent",
        "conditional-true": "--color-tcld-success",
        "conditional-false": "--color-tcld-error",
      },
    },
  };
}
