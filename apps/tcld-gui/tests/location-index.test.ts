import assert from "node:assert/strict";
import test from "node:test";
import { buildLocationIndex } from "@/lib/location-index";
import { createLocationFixture } from "./location-fixture.ts";

test("buildLocationIndex builds forward and reverse lookup maps", () => {
  const fixture = createLocationFixture();
  const index = buildLocationIndex(
    fixture.instructions,
    fixture.blocks,
    fixture.decompileLines,
    fixture.scopeId,
  );

  assert.equal(index.scopeId, fixture.scopeId);
  assert.equal(index.instructionCount, 4);

  assert.equal(index.instIdxToPc[0], 10);
  assert.equal(index.instIdxToPc[3], 24);
  assert.equal(index.instIdxToGroup[2], 2);
  assert.equal(index.instIdxToBlock[2], 8);

  assert.equal(index.pcToInstIdx.get(12), 1);
  assert.equal(index.groupToFirstInstIdx.get(1), 0);
  assert.equal(index.groupToFirstInstIdx.get(3), 3);
  assert.equal(index.blockToFirstInstIdx.get(7), 0);
  assert.equal(index.decompileLineToGroup.get(1), 2);
  assert.equal(index.sourceLineToFirstInstIdx.get(20), 0);
  assert.equal(index.sourceLineToFirstInstIdx.get(21), 2);
});
