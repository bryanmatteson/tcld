import assert from "node:assert/strict";
import test from "node:test";
import {
  goBack,
  goBackTo,
  goForward,
  isJumpLocation,
  pushBackLocation,
  sameLocation,
  type NavigationStacks,
} from "@/lib/navigation-history";
import type { Location } from "@/lib/location-resolve";

function loc(instIdx: number): Location {
  return {
    scopeId: "fixture-scope",
    instIdx,
    pc: 100 + instIdx,
    group: Math.floor(instIdx / 2),
    blockId: Math.floor(instIdx / 2),
    decompileLine: null,
    sourceFile: null,
    sourceLine: null,
  };
}

test("sameLocation and isJumpLocation enforce deterministic history rules", () => {
  assert.equal(sameLocation(loc(1), loc(1)), true);
  assert.equal(isJumpLocation(loc(1), loc(2)), true, "group/block changed");
  assert.equal(isJumpLocation({ ...loc(1), group: 1, blockId: 1 }, { ...loc(3), group: 1, blockId: 1 }), false);
});

test("back/forward stacks preserve order and cap history size", () => {
  let stacks: NavigationStacks = { back: [], forward: [] };
  for (let i = 0; i < 120; i += 1) {
    stacks = pushBackLocation(stacks, loc(i));
  }

  assert.equal(stacks.back.length, 100);
  assert.equal(stacks.forward.length, 0);

  const back = goBack(stacks, loc(999));
  assert.equal(back.target?.instIdx, 119);
  assert.equal(back.stacks.forward.length, 1);

  const forward = goForward(back.stacks, back.target ?? loc(0));
  assert.equal(forward.target?.instIdx, 999);
});

test("goBackTo jumps to requested offset and preserves skipped entries in forward order", () => {
  const base = { back: [loc(1), loc(2), loc(3), loc(4)], forward: [] };
  const jumped = goBackTo(base, loc(10), 3);

  assert.equal(jumped.target?.instIdx, 2);
  assert.deepEqual(jumped.stacks.back.map((entry) => entry.instIdx), [1]);
  assert.deepEqual(jumped.stacks.forward.map((entry) => entry.instIdx), [10, 4, 3]);
});
