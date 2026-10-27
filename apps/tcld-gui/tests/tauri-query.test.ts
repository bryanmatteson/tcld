import assert from "node:assert/strict";
import test from "node:test";
import { clearAll, inspect, invalidate, query, setCache } from "@/lib/tauri-query";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

test("invalidated requests cannot cache stale results or erase their replacement", async () => {
  clearAll();
  const old = deferred<string>();
  const next = deferred<string>();
  const first = query(["file", "a"], () => old.promise);
  invalidate("file");
  const second = query(["file", "a"], () => next.promise);
  old.resolve("old");
  assert.equal(await first, "old");
  assert.deepEqual(inspect(), { cacheSize: 0, inflightSize: 1 });
  const deduplicated = query(["file", "a"], async () => "unexpected");
  next.resolve("new");
  assert.equal(await second, "new");
  assert.equal(await deduplicated, "new");
  assert.equal(await query(["file", "a"], async () => "unexpected"), "new");
});

test("zero TTL deduplicates simultaneous loads but rereads on reopening", async () => {
  clearAll();
  const pending = deferred<string>();
  let calls = 0;
  const read = () => { calls++; return pending.promise; };
  const first = query(["load-file", "a"], read, { ttlMs: 0 });
  const second = query(["load-file", "a"], read, { ttlMs: 0 });
  pending.resolve("original");
  assert.deepEqual(await Promise.all([first, second]), ["original", "original"]);
  assert.equal(calls, 1);
  assert.equal(await query(["load-file", "a"], async () => "edited", { ttlMs: 0 }), "edited");
  assert.equal(inspect().cacheSize, 0);
});

test("explicit fresh cache data survives older requests and prefix matching is exact", async () => {
  clearAll();
  const pending = deferred<string>();
  const first = query(["file", "a"], () => pending.promise);
  setCache(["file", "a"], "fresh");
  setCache(["file-other", "a"], "unrelated");
  pending.resolve("old");
  await first;
  assert.equal(await query(["file", "a"], async () => "unexpected"), "fresh");
  invalidate("file");
  assert.equal(await query(["file-other", "a"], async () => "unexpected"), "unrelated");
});
