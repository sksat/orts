import * as duckdb from "@duckdb/duckdb-wasm";

let dbPromise: Promise<duckdb.AsyncDuckDB> | null = null;

const MAX_ATTEMPTS = 3;
// Generous backstop for a silently-dead/hung worker — kept long so it does NOT
// abort a slow-but-healthy first load (large wasm fetch + compile on a slow
// network/device). The worker `error` listener below fast-fails the common
// load-failure case well before this fires.
const INIT_TIMEOUT_MS = 45000;

/**
 * URLs for one DuckDB-wasm platform bundle variant. Matches the shape
 * `duckdb.selectBundle` expects, but every field is a plain string so the
 * whole object is structured-clonable across a `postMessage` boundary
 * (Worker `init`) and free of any bundler-specific import syntax.
 */
export interface DuckDBBundleUrls {
  mvp: { mainModule: string; mainWorker: string };
  eh?: { mainModule: string; mainWorker: string };
  coi?: { mainModule: string; mainWorker: string; pthreadWorker: string };
}

/**
 * How `initDuckDB` should source the DuckDB-wasm worker/wasm assets.
 *
 * The app (which owns a bundler) resolves the self-hosted asset URLs and
 * passes them in via `bundles`; uneri itself stays bundler-neutral and never
 * references `?url`/`new URL(..., import.meta.url)`. When `bundles` is omitted
 * the legacy jsDelivr CDN path is used so existing consumers keep working.
 */
export interface DuckDBInitOptions {
  /** Pre-resolved self-hosted bundle URLs. Omit to use the jsDelivr CDN. */
  bundles?: DuckDBBundleUrls;
  /**
   * When `bundles` is set, permit a last-ditch jsDelivr CDN attempt if every
   * local attempt fails. Defaults to `false`: an explicitly self-hosted setup
   * should fail loudly rather than silently reaching the network, which is the
   * whole point of going hermetic. Ignored when `bundles` is omitted.
   */
  fallbackToJsDelivr?: boolean;
}

/**
 * Decide which bundle set to load and whether a jsDelivr fallback is allowed.
 * Pure (no I/O) so the policy is unit-testable without instantiating DuckDB.
 *
 * - Injected `bundles` are returned verbatim — jsDelivr is never consulted,
 *   and a fallback is only allowed when explicitly opted into.
 * - With no options, the jsDelivr CDN bundles are the primary source; there is
 *   no secondary fallback because we are already on the CDN (the retry loop in
 *   `initDuckDB` handles transient CDN hiccups).
 */
export function resolveBundleSource(options?: DuckDBInitOptions): {
  bundles: duckdb.DuckDBBundles;
  allowJsDelivrFallback: boolean;
} {
  if (options?.bundles) {
    return {
      bundles: options.bundles as duckdb.DuckDBBundles,
      allowJsDelivrFallback: options.fallbackToJsDelivr ?? false,
    };
  }
  return { bundles: duckdb.getJsDelivrBundles(), allowJsDelivrFallback: false };
}

/**
 * One DuckDB-wasm instantiation attempt against an already-selected bundle set.
 *
 * Rejects (rather than hanging) when the worker/wasm can't be fetched: a failed
 * `importScripts` kills the worker, after which `instantiate()` would otherwise
 * wait forever for a dead worker to reply. A worker `error` event or an overall
 * timeout both abort the attempt so the caller can retry.
 */
async function instantiateOnce(bundles: duckdb.DuckDBBundles): Promise<duckdb.AsyncDuckDB> {
  const bundle = await duckdb.selectBundle(bundles);

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
        (ev: ErrorEvent) =>
          settle(() =>
            reject(new Error(`DuckDB worker failed to load: ${ev.message || "unknown error"}`)),
          ),
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

/** Run `instantiateOnce` up to `MAX_ATTEMPTS` times with linear backoff. */
async function instantiateWithRetry(bundles: duckdb.DuckDBBundles): Promise<duckdb.AsyncDuckDB> {
  let lastError: unknown;
  for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt++) {
    try {
      return await instantiateOnce(bundles);
    } catch (e) {
      lastError = e;
      if (attempt < MAX_ATTEMPTS) {
        await new Promise((r) => setTimeout(r, attempt * 500));
      }
    }
  }
  throw lastError instanceof Error ? lastError : new Error(String(lastError));
}

/**
 * Initialize DuckDB-wasm (singleton). By default the worker/wasm are loaded
 * from self-hosted URLs supplied via `options.bundles`; with no options the
 * jsDelivr CDN is used (legacy behavior). Retries a few times so a transient
 * hiccup doesn't fail the whole session — historically a source of viewer E2E
 * flakiness (see #70/#65). Safe to call multiple times — returns the same
 * promise, so the first caller's `options` win.
 */
export function initDuckDB(options?: DuckDBInitOptions): Promise<duckdb.AsyncDuckDB> {
  if (dbPromise) return dbPromise;

  const { bundles, allowJsDelivrFallback } = resolveBundleSource(options);

  dbPromise = (async () => {
    try {
      return await instantiateWithRetry(bundles);
    } catch (e) {
      // Only reachable when the caller injected local bundles AND opted into a
      // CDN fallback; the default hermetic path rethrows here.
      if (allowJsDelivrFallback) {
        return await instantiateWithRetry(duckdb.getJsDelivrBundles());
      }
      throw e;
    }
  })();

  // If every attempt failed, drop the cached rejected promise so a later call
  // (e.g. after the user reconnects) retries from scratch instead of replaying
  // the same rejection for the rest of the session.
  dbPromise.catch(() => {
    dbPromise = null;
  });

  return dbPromise;
}
