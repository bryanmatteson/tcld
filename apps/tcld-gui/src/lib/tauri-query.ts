/**
 * Tiny query layer over Tauri `invoke` calls.
 *
 * Two problems this solves:
 *  1. Two panels asking for the same backend data simultaneously fire two
 *     round-trips (no in-flight dedup).
 *  2. Cursor moves and tab switches re-call the same `invoke` for data that
 *     hasn't changed (no result cache).
 *
 * Usage:
 *   const detail = await query(
 *     ["instruction-detail", fileId, idx],
 *     () => invoke("get_instruction_detail", { idx }),
 *     { ttlMs: 60_000 },
 *   );
 *
 *   // After a file reload:
 *   invalidate("instruction-detail");
 */

type QueryKey = readonly [string, ...unknown[]];

interface CacheEntry {
  data: unknown;
  ts: number;
  ttlMs: number;
}

const cache = new Map<string, CacheEntry>();
const inflight = new Map<string, Promise<unknown>>();

const DEFAULT_TTL = 30_000;

function keyToString(key: QueryKey): string {
  // JSON.stringify is sufficient because keys are required to be primitive
  // (the docstring on `query` explains this). We don't try to handle Map /
  // Set / class instances; they shouldn't appear here.
  return JSON.stringify(key);
}

/**
 * Returns cached result if present and fresh; otherwise runs `fn` and caches
 * the result. Concurrent callers with the same key share one promise.
 */
export async function query<T>(
  key: QueryKey,
  fn: () => Promise<T>,
  opts: { ttlMs?: number } = {},
): Promise<T> {
  const k = keyToString(key);
  const ttlMs = opts.ttlMs ?? DEFAULT_TTL;

  const hit = cache.get(k);
  if (hit && Date.now() - hit.ts < hit.ttlMs) {
    return hit.data as T;
  }

  const running = inflight.get(k) as Promise<T> | undefined;
  if (running) return running;

  const p = fn()
    .then((data) => {
      cache.set(k, { data, ts: Date.now(), ttlMs });
      return data;
    })
    .finally(() => {
      inflight.delete(k);
    });
  inflight.set(k, p);
  return p;
}

/**
 * Invalidates every cached entry whose key starts with the given prefix
 * (after JSON-stringify). Pass the first key segment, e.g.
 *   invalidate("instruction-detail")  // clears all per-instruction details
 *   invalidate("file-load")           // clears the file-load record
 */
export function invalidate(prefix: string): void {
  // The serialized key always starts with `["${segment}"`, so we test for
  // exactly that to avoid accidentally matching `["instruction-detail2"]`.
  const needle = `[${JSON.stringify(prefix)}`;
  for (const k of cache.keys()) {
    if (k.startsWith(needle)) cache.delete(k);
  }
  for (const k of inflight.keys()) {
    if (k.startsWith(needle)) inflight.delete(k);
  }
}

/**
 * Eagerly stores a result under a key without going through `fn`. Useful
 * when an event arrives over Tauri's event bus and you want to keep the
 * cache consistent with what the panel would have fetched.
 */
export function setCache<T>(key: QueryKey, data: T, ttlMs: number = DEFAULT_TTL): void {
  cache.set(keyToString(key), { data, ts: Date.now(), ttlMs });
}

/** Clears the entire cache. Used on app reset / settings change / theme switch tests. */
export function clearAll(): void {
  cache.clear();
  inflight.clear();
}

/** Diagnostic helper for tests / devtools. Returns shallow stats. */
export function inspect(): { cacheSize: number; inflightSize: number } {
  return { cacheSize: cache.size, inflightSize: inflight.size };
}
