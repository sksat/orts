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

## Multi-Satellite NaN Alignment

The unified `tMax` parameter in `buildDerivedQuery()` ensures all satellite tables use the same time-bucket boundaries for downsampling. Without this, independent NTILE bucketing produces different timestamps per table, causing NaN when series are merged via `alignTimeSeries()`.

Verification checklist for "All" view:
- Both series have equal coverage percentage
- Max gap length is symmetric (same for both colors)
- No alternating single-color columns (the signature of the old NaN bug)
