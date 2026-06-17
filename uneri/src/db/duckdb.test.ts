import * as duckdb from "@duckdb/duckdb-wasm";
import { describe, expect, it } from "vitest";
import { type DuckDBBundleUrls, resolveBundleSource } from "./duckdb.js";

// Self-hosted bundle URLs as an app bundler (e.g. Vite `?url`) would resolve
// them: root-relative asset paths, never a CDN.
const LOCAL_BUNDLES: DuckDBBundleUrls = {
  mvp: {
    mainModule: "/assets/duckdb-mvp.wasm",
    mainWorker: "/assets/duckdb-browser-mvp.worker.js",
  },
  eh: {
    mainModule: "/assets/duckdb-eh.wasm",
    mainWorker: "/assets/duckdb-browser-eh.worker.js",
  },
};

describe("resolveBundleSource", () => {
  it("returns injected bundles verbatim (never the CDN) when bundles are given", () => {
    const { bundles } = resolveBundleSource({ bundles: LOCAL_BUNDLES });
    // Referential identity proves jsDelivr was never consulted.
    expect(bundles).toBe(LOCAL_BUNDLES);
    expect(bundles.mvp.mainModule).not.toContain("jsdelivr");
  });

  it("falls back to the jsDelivr CDN bundles when no options are given", () => {
    const { bundles } = resolveBundleSource();
    const cdn = duckdb.getJsDelivrBundles();
    expect(bundles.mvp.mainModule).toBe(cdn.mvp.mainModule);
    expect(bundles.mvp.mainModule).toContain("jsdelivr");
  });

  it("forbids a jsDelivr fallback for explicit local bundles by default (hermetic)", () => {
    expect(resolveBundleSource({ bundles: LOCAL_BUNDLES }).allowJsDelivrFallback).toBe(false);
  });

  it("permits a jsDelivr fallback for local bundles only when explicitly opted in", () => {
    const { allowJsDelivrFallback } = resolveBundleSource({
      bundles: LOCAL_BUNDLES,
      fallbackToJsDelivr: true,
    });
    expect(allowJsDelivrFallback).toBe(true);
  });

  it("uses no secondary fallback when jsDelivr is already the primary source", () => {
    expect(resolveBundleSource().allowJsDelivrFallback).toBe(false);
  });
});
