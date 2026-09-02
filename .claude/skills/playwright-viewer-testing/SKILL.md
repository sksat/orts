---
name: playwright-viewer-testing
description: Guidelines for E2E testing the orts viewer with Playwright. Covers DuckDB query-level data verification, canvas pixel analysis, and common pitfalls.
---

# Playwright Viewer Testing Guide

## Data Verification Strategy

### Prefer DuckDB Query-Level Access

The viewer uses @orts/uneri which stores all chart data in DuckDB-wasm tables. When verifying data in Playwright E2E tests, **query DuckDB directly** rather than trying to extract data from uPlot or React component state.

**Why:**
- uPlot instances are stored in React refs and are not accessible from DOM properties
- React fiber traversal to find memoizedState is fragile and version-dependent
- DuckDB queries give you the raw source-of-truth data with full precision

**How:**
The DuckDB connection is not directly exposed on `window`, but you can inject a helper via `page.evaluate()` or expose it during dev mode. For existing E2E tests, the approach is:

1. **Check table existence and row counts** via the viewer's displayed point count (e.g., "4536 points" in the status bar)
2. **Use canvas pixel analysis** for visual verification (see below)
3. **For precise numerical checks**, add a dev-mode DuckDB query endpoint or expose the connection on window in test builds

### Canvas Pixel Analysis

For visual verification (e.g., "are both satellite series rendering without NaN gaps?"), scan the canvas pixels directly:

```javascript
// Color reference for multi-satellite charts:
// SSO (green):  rgb(0, 255, 136)  → R<50, G>200, B>100
// ISS (pink):   rgb(255, 68, 136) → R>200, G<100, B>100

const canvas = document.querySelectorAll('.u-wrap canvas')[chartIndex];
const ctx = canvas.getContext('2d');
const imgData = ctx.getImageData(0, 0, canvas.width, canvas.height);

// Scan each column for colored pixels
// Skip xStart=30 (Y-axis labels) and xEnd=w-10 (right margin)
```

Key metrics to check:
- **Coverage %**: Both series should have similar coverage (78-80% of plot area)
- **Max gap**: Maximum consecutive columns without a series' color. Equal gaps for both series = OK (chart padding). Asymmetric gaps = NaN issue
- **Both columns**: Columns where both colors present — should be majority of data area

### The 3D Scene: Measure the Scene Graph, Not Pixels

The pixel guidance above is for the uPlot **2D** canvases. For the WebGL 3D scene,
prefer a dev-only debug hook that reads the *rendered* Three.js scene graph:

```javascript
// window.__debug_get_sat_world_quat(id)      → rendered world quaternion
// window.__debug_get_direction_vectors(id)   → arrow directions, measured from
//                                              the arrow's origin to its head
```

Reasons this beats a pixel test for the 3D view:

- A few triangles on a dark background fail a pixel test for reasons unrelated to
  the geometry (anti-aliasing, camera framing, the swiftshader software renderer).
- Frame invariants ("body +X points along scene +Y") are immune to the ±q sign
  ambiguity, whereas a raw quaternion comparison is not.
- The hook must measure the scene graph, not report the value that was passed *in*.
  A hook that echoes its input passes even when the geometry's own axis, the
  rotation onto it, or a mesh's placement is wrong — the parts a rendering test
  exists to cover.

Register hooks behind `IS_DEV` and deregister on unmount: an *absent* hook is then
usable evidence that a subtree was dropped (e.g. the attitude view has no Earth).
Establish the hook exists before the action whose effect you assert, or its absence
afterwards proves nothing.

#### Inventorying the whole scene

To count or measure *everything* drawn — is that mesh a cylinder or a line, how
many sprites are there, what opacity did they get — reach the scene root through
an existing hook's registry rather than the canvas. `canvas.__r3f` is not a
documented API and reads back `null`; the registries hold the rendered object
itself, so walk up its `.parent` chain:

```javascript
const group = window.__debug_sat_quat_registry?.get(id)?.();   // rendered object
let root = group; while (root.parent != null) root = root.parent;
root.traverse((o) => { /* o.isSprite, o.geometry?.type, o.material?.opacity */ });
```

The registry is keyed by the id the *scene* was given, which in the app is the
entity path (`/world/sat/<id>`), not the config id — enumerate `registry.keys()`
instead of assuming. This route is for a one-off probe while developing. What a
committed test asserts should still go through a named hook, so the assertion
survives a refactor of the scene's internals.

### Common Pitfalls

1. **Cannot access uPlot data from DOM**: `chart._uplot`, `chart.__uplot` do not exist. uPlot stores instance internally in React ref via `useRef()`.

2. **Canvas `getContext('2d')` warns about readback**: When scanning multiple canvases, Chrome warns about "Multiple readback operations". This is harmless for testing but avoid in production.

3. **Color thresholds matter**: The viewer uses specific colors (not pure red/green). Always sample actual pixel colors first before writing detection logic:
   ```javascript
   // Sample actual colors from a chart
   const colorMap = {};
   // ... collect non-transparent, non-gray pixels
   // Sort by frequency to find the series colors
   ```

4. **Chart area vs axis area**: ~20% of canvas width is Y-axis labels (left side). Always skip `xStart=30` columns when analyzing data coverage.

5. **`gl.readPixels` on the 3D canvas returns zeros** unless the context was created
   with `preserveDrawingBuffer: true`: the drawing buffer is cleared at presentation.
   `page.screenshot()` composites correctly and needs no flag. A bounding-box scan
   ("does anything touch the viewport edge?") is a cheap way to check
   framing/clipping without judging the image itself.

## Multi-Satellite NaN Alignment

The unified `tMax` parameter in `buildDerivedQuery()` ensures all satellite tables use the same time-bucket boundaries for downsampling. Without this, independent NTILE bucketing produces different timestamps per table, causing NaN when series are merged via `alignTimeSeries()`.

Verification checklist for "All" view:
- Both series have equal coverage percentage
- Max gap length is symmetric (same for both colors)
- No alternating single-color columns (the signature of the old NaN bug)
