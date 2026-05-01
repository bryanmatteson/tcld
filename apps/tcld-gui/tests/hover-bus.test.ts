import assert from "node:assert/strict";
import test from "node:test";
import { hoverBus } from "@/lib/hover-bus";

test("hover bus emits latest payload once per animation frame", () => {
  const originalRaf = globalThis.requestAnimationFrame;
  const queued: FrameRequestCallback[] = [];
  let rafCalls = 0;

  globalThis.requestAnimationFrame = ((cb: FrameRequestCallback) => {
    rafCalls += 1;
    queued.push(cb);
    return rafCalls;
  }) as typeof requestAnimationFrame;

  const seen: Array<number | null> = [];
  const unsubscribe = hoverBus.subscribe((loc) => {
    seen.push(loc?.instIdx ?? null);
  });

  hoverBus.emit({ instIdx: 1 });
  hoverBus.emit({ instIdx: 2 });
  assert.equal(rafCalls, 1);
  assert.deepEqual(seen, []);
  assert.equal(hoverBus.peek()?.instIdx, 2);

  const frame = queued.shift();
  if (frame) frame(0);
  assert.deepEqual(seen, [2]);

  unsubscribe();
  hoverBus.emit(null);
  const nextFrame = queued.shift();
  if (nextFrame) nextFrame(1);
  assert.deepEqual(seen, [2]);

  globalThis.requestAnimationFrame = originalRaf;
});
