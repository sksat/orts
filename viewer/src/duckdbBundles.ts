import ehWorker from "@duckdb/duckdb-wasm/dist/duckdb-browser-eh.worker.js?url";
import mvpWorker from "@duckdb/duckdb-wasm/dist/duckdb-browser-mvp.worker.js?url";
import ehWasm from "@duckdb/duckdb-wasm/dist/duckdb-eh.wasm?url";
import mvpWasm from "@duckdb/duckdb-wasm/dist/duckdb-mvp.wasm?url";
import type { DuckDBBundleUrls } from "@sksat/uneri";

/**
 * Self-hosted DuckDB-wasm bundle URLs, resolved by Vite from the installed
 * `@duckdb/duckdb-wasm` package — no jsDelivr CDN.
 *
 * Vite emits these assets into the build and rewrites the imports to hashed,
 * root-relative URLs, so they resolve against the document origin even from
 * the nested DuckDB worker. Keeping the CDN out of the runtime path makes the
 * viewer hermetic (offline-capable, no third-party dependency) and removes the
 * intermittent CDN/worker-asset load failures behind the viewer-e2e flakiness
 * (issue #70). This is bundler-specific glue, so it lives in the app rather
 * than in uneri (which only consumes the resolved string URLs).
 *
 * `coi` (cross-origin-isolated / pthreads) is intentionally omitted: it needs
 * COOP/COEP headers we don't serve, and `selectBundle` only picks it when
 * `crossOriginIsolated` is true. The mvp + eh variants cover every browser we
 * target.
 */
// Vite resolves `?url` to a *root-relative* path (`/assets/…` in a build,
// `/@fs/…` in dev). DuckDB instantiates its worker via a Blob that calls
// `importScripts(mainWorker)`; inside that Blob worker the base URL is the
// opaque `blob:` URL, against which a root-relative path cannot be resolved
// ("invalid URL"). Resolving to an absolute URL against the document origin
// up front makes both the `importScripts` and the worker's own wasm fetch
// work regardless of the worker's base.
const abs = (url: string): string => new URL(url, window.location.origin).href;

export const duckdbBundles: DuckDBBundleUrls = {
  mvp: { mainModule: abs(mvpWasm), mainWorker: abs(mvpWorker) },
  eh: { mainModule: abs(ehWasm), mainWorker: abs(ehWorker) },
};
