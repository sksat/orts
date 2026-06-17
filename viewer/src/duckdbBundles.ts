import ehWorker from "@duckdb/duckdb-wasm/dist/duckdb-browser-eh.worker.js?url";
import mvpWorker from "@duckdb/duckdb-wasm/dist/duckdb-browser-mvp.worker.js?url";
import ehWasm from "@duckdb/duckdb-wasm/dist/duckdb-eh.wasm?url";
import mvpWasm from "@duckdb/duckdb-wasm/dist/duckdb-mvp.wasm?url";
import type { DuckDBBundleUrls } from "@sksat/uneri";

/**
 * Self-hosted DuckDB-wasm bundle URLs, resolved by Vite from the installed
 * `@duckdb/duckdb-wasm` package — no jsDelivr CDN.
 *
 * Vite emits these assets into the build and rewrites the imports to hashed
 * asset URLs. Keeping the CDN out of the runtime path makes the viewer
 * hermetic (offline-capable, no third-party dependency) and removes the
 * intermittent CDN/worker-asset load failures behind the viewer-e2e flakiness
 * (issue #70). This is bundler-specific glue, so it lives in the app rather
 * than in uneri (which only consumes the resolved string URLs). The URLs Vite
 * produces are root-relative; `initDuckDB` absolutizes them against the worker
 * origin so they work inside DuckDB's blob worker (see `toAbsoluteBundleUrl`),
 * matching the duckdb-wasm Vite usage example.
 *
 * `coi` (cross-origin-isolated / pthreads) is intentionally omitted: it needs
 * COOP/COEP headers we don't serve, and `selectBundle` only picks it when
 * `crossOriginIsolated` is true. The mvp + eh variants cover every browser we
 * target.
 */
export const duckdbBundles: DuckDBBundleUrls = {
  mvp: { mainModule: mvpWasm, mainWorker: mvpWorker },
  eh: { mainModule: ehWasm, mainWorker: ehWorker },
};
