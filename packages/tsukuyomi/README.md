# @orts/tsukuyomi

DuckDB-wasm + uPlot time-series charting library for real-time streaming data.

Generic schema-driven design: define columns and derived SQL expressions, and tsukuyomi handles DuckDB ingestion, query-time downsampling, and chart rendering.

## Build

```bash
pnpm install
pnpm build
```

## Test

```bash
pnpm test        # vitest unit tests (59 tests)
pnpm test:e2e    # Playwright E2E tests (5 tests)
```

## Examples

### sine-wave

Basic real-time sine/cosine visualization.

```bash
# Terminal 1: WebSocket server (port 9002)
npx tsx examples/sine-wave/server.ts

# Terminal 2: Vite dev server (port 5174)
npx vite --config vite.example.config.ts --port 5174
```

Open http://localhost:5174

### mixed-density

Mixed-density data test: sparse overview (100 points over 5000s) + dense streaming (100 msg/sec). Used for E2E testing of the time-bucket downsampling algorithm.

```bash
# Terminal 1: WebSocket server (port 9003)
npx tsx examples/mixed-density/server.ts

# Terminal 2: Vite dev server (port 5175)
npx vite --config vite.mixed-density.config.ts --port 5175
```

Open http://localhost:5175

## Architecture

- `IngestBuffer<T>` — staging buffer with drain pattern (decouples WebSocket arrival from DuckDB insertion)
- `useDuckDB(schema)` — initializes DuckDB-wasm, creates table
- `useTimeSeriesStore(options)` — tick loop: drain buffer → INSERT → periodic query with downsampling
- `TimeSeriesChart` — uPlot wrapper with programmatic update guard
- `buildDerivedQuery(schema, tMin, maxPoints)` — time-bucket downsampling SQL
- `sliceArrays` / `lowerBound` / `upperBound` — binary search viewport clipping
