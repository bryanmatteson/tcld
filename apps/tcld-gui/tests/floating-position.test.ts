import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { computeFloatingPosition } from "@/lib/floating-position";

test("context menus open at the cursor unless they need viewport clamping", () => {
  const position = computeFloatingPosition(
    { left: 120, top: 80, width: 0, height: 0 },
    { width: 180, height: 120 },
    { width: 800, height: 600 },
    { placement: "bottom-start", offset: 2 },
  );

  assert.equal(position.left, 120);
  assert.equal(position.top, 82);
  assert.equal(position.ready, true);
});

test("floating menus flip and clamp instead of going offscreen", () => {
  const position = computeFloatingPosition(
    { left: 760, top: 560, width: 32, height: 28 },
    { width: 220, height: 180 },
    { width: 800, height: 600 },
    { placement: "bottom-end", offset: 4 },
  );

  assert.equal(position.left, 572);
  assert.equal(position.top, 376);
  assert.ok(position.maxHeight <= 548);
});

test("panel picker menu escapes header stacking contexts", () => {
  const source = readFileSync(new URL("../src/components/chrome/PanelPicker.tsx", import.meta.url), "utf8");
  const globals = readFileSync(new URL("../src/app/globals.css", import.meta.url), "utf8");

  // The menu escapes any header stacking context two ways: it portals to
  // document.body, and it sits on the tokenized context-menu z-layer (the
  // token must actually be defined, or the utility class silently resolves
  // to z-index:auto and the menu falls behind panel chrome).
  assert.match(source, /createPortal\([\s\S]*document\.body/);
  assert.match(source, /z-\[var\(--z-context-menu\)\]/);
  assert.match(globals, /--z-context-menu:\s*\d+/);
});
