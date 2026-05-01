import assert from "node:assert/strict";
import test from "node:test";
import { buildLocationIndex } from "@/lib/location-index";
import { resolveLocation } from "@/lib/location-resolve";
import { createLocationFixture } from "./location-fixture.ts";

test("resolveLocation prioritizes instruction index and keeps canonical fields consistent", () => {
  const fixture = createLocationFixture();
  const index = buildLocationIndex(
    fixture.instructions,
    fixture.blocks,
    fixture.decompileLines,
    fixture.scopeId,
  );

  const resolved = resolveLocation(
    { scopeId: fixture.scopeId, instIdx: 2, pc: 9999, group: 999 },
    index,
  );

  assert.equal(resolved.instIdx, 2);
  assert.equal(resolved.pc, 14);
  assert.equal(resolved.group, 2);
  assert.equal(resolved.blockId, 8);
});

test("resolveLocation follows fallback chain pc -> group -> block -> decompileLine -> sourceLine", () => {
  const fixture = createLocationFixture();
  const index = buildLocationIndex(
    fixture.instructions,
    fixture.blocks,
    fixture.decompileLines,
    fixture.scopeId,
  );

  assert.equal(resolveLocation({ pc: 12 }, index).instIdx, 1);
  assert.equal(resolveLocation({ group: 3 }, index).instIdx, 3);
  assert.equal(resolveLocation({ blockId: 7 }, index).instIdx, 0);
  assert.equal(resolveLocation({ decompileLine: 1 }, index).instIdx, 2);
  assert.equal(resolveLocation({ sourceLine: 20 }, index).instIdx, 0);
});

test("resolveLocation guards against scope mismatch", () => {
  const fixture = createLocationFixture();
  const index = buildLocationIndex(
    fixture.instructions,
    fixture.blocks,
    fixture.decompileLines,
    fixture.scopeId,
  );

  const resolved = resolveLocation({ scopeId: "other-scope", pc: 12 }, index);
  assert.equal(resolved.scopeId, "other-scope");
  assert.equal(resolved.instIdx, null);
  assert.equal(resolved.pc, null);
});
