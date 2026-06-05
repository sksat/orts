import * as duckdb from "@duckdb/duckdb-wasm";

let dbPromise: Promise<duckdb.AsyncDuckDB> | null = null;

const MAX_ATTEMPTS = 3;
const INIT_TIMEOUT_MS = 15000;

/**
 * One DuckDB-wasm instantiation attempt using the jsDelivr CDN bundles.
 *
 * Rejects (rather than hanging) when the CDN worker/wasm can't be fetched: a
 * failed `importScripts` kills the worker, after which `instantiate()` would
 * otherwise wait forever for a dead worker to reply. A worker `error` event or
 * an overall timeout both abort the attempt so the caller can retry.
 */
async function instantiateOnce(): Promise<duckdb.AsyncDuckDB> {
  const bundle = await duckdb.selectBundle(duckdb.getJsDelivrBundles());

  const workerUrl = URL.createObjectURL(
    new Blob([`importScripts("${bundle.mainWorker!}");`], { type: "text/javascript" }),
  );
  const worker = new Worker(workerUrl);
  const db = new duckdb.AsyncDuckDB(new duckdb.VoidLogger(), worker);

  try {
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`DuckDB init timed out after ${INIT_TIMEOUT_MS}ms`)),
        INIT_TIMEOUT_MS,
      );
      const settle = (fn: () => void) => {
        clearTimeout(timer);
        fn();
      };
      worker.addEventListener(
        "error",
        () => settle(() => reject(new Error("DuckDB worker failed to load (CDN unreachable?)"))),
        { once: true },
      );
      db.instantiate(bundle.mainModule, bundle.pthreadWorker).then(
        () => settle(resolve),
        (err) => settle(() => reject(err)),
      );
    });
    return db;
  } catch (e) {
    worker.terminate();
    throw e;
  } finally {
    URL.revokeObjectURL(workerUrl);
  }
}

/**
 * Initialize DuckDB-wasm (singleton), loading the worker/wasm from the jsDelivr
 * CDN. Retries a few times so a transient CDN hiccup doesn't fail the whole
 * session — a recurring source of viewer E2E flakiness (see #70).
 * Safe to call multiple times — returns the same promise.
 */
export function initDuckDB(): Promise<duckdb.AsyncDuckDB> {
  if (dbPromise) return dbPromise;

  dbPromise = (async () => {
    let lastError: unknown;
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
      try {
        return await instantiateOnce();
      } catch (e) {
        lastError = e;
        if (attempt < MAX_ATTEMPTS) {
          await new Promise((r) => setTimeout(r, attempt * 500));
        }
      }
    }
    throw lastError;
  })();

  // If every attempt failed, drop the cached rejected promise so a later
  // call (e.g. after the user reconnects) retries from scratch instead of
  // replaying the same rejection for the rest of the session.
  dbPromise.catch(() => {
    dbPromise = null;
  });

  return dbPromise;
}
