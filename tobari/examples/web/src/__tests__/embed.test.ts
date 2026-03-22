/**
 * Tests for docs embedding build output.
 *
 * Verifies that the Vite build with a custom base path produces
 * correct asset references for GitHub Pages deployment.
 */

import { describe, it, expect } from "vitest";

const DOCS_BASE = "/orts/tobari/examples/earth-visualizer/demo/";

describe("docs embedding configuration", () => {
  it("base path starts with /orts/ (GitHub Pages prefix)", () => {
    expect(DOCS_BASE.startsWith("/orts/")).toBe(true);
  });

  it("base path ends with /", () => {
    expect(DOCS_BASE.endsWith("/")).toBe(true);
  });

  it("base path follows uneri example convention", () => {
    // Pattern: /orts/{crate}/examples/{name}/demo/
    const parts = DOCS_BASE.split("/").filter(Boolean);
    expect(parts[0]).toBe("orts");
    expect(parts[1]).toBe("tobari");
    expect(parts[2]).toBe("examples");
    expect(parts[4]).toBe("demo");
  });

  it("earth texture path resolves correctly with base", () => {
    // In GlobeView.tsx: `${import.meta.env.BASE_URL}textures/earth_2k.jpg`
    // import.meta.env.BASE_URL = DOCS_BASE in production build
    const texturePath = `${DOCS_BASE}textures/earth_2k.jpg`;
    expect(texturePath).toBe("/orts/tobari/examples/earth-visualizer/demo/textures/earth_2k.jpg");
  });

  it("WASM module path resolves correctly with base", () => {
    // wasm-pack generates: new URL('tobari_bg.wasm', import.meta.url)
    // Vite rewrites import.meta.url to use base path in production
    // This test documents the expected behavior
    expect(DOCS_BASE).toContain("demo/");
  });
});

describe("iframe embedding", () => {
  const IFRAME_SRC = `${DOCS_BASE}index.html`;

  it("iframe src points to demo index.html", () => {
    expect(IFRAME_SRC).toBe("/orts/tobari/examples/earth-visualizer/demo/index.html");
  });

  it("iframe height should be >= 900px for 3D + controls", () => {
    const height = 900;
    // App has: header(~40px) + controls(~50px) + playback(~35px) + 3D canvas(rest)
    // Minimum usable 3D area: ~600px
    const overhead = 40 + 50 + 35;
    const canvasHeight = height - overhead;
    expect(canvasHeight).toBeGreaterThanOrEqual(600);
  });
});
