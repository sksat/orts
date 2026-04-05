/**
 * E2E test: multi-sat chart pipeline wiring + alignment invariants.
 *
 * What this test gates:
 * - `isMultiSatellite` detection flips when `simInfo.satellites.length >= 2`
 *   and `useMultiSatelliteStoreWorker` starts on that transition (regression
 *   gate for the `[enabled]` lifecycle bug fix).
 * - The full history ingest path works end-to-end: WS `history` message →
 *   per-sat `IngestBuffer.markRebuild` → worker `multi-rebuild` → DuckDB
 *   populated → tick query → `buildDerivedQuery` with downsampling actually
 *   triggered (we send > `maxPoints` rows per sat) → aligned
 *   `MultiChartDataMap` broadcast → main-thread deserialization.
 * - `alignTimeSeries`'s length invariant (`values[i].length === t.length`).
 * - No critical console errors from malformed INSERT / DuckDB hiccups.
 *
 * What this test DOES NOT gate:
 * - That the worker actually passes a *unified* tMax to `queryDerived`.
 *   With identical row sets per sat, per-sat `MAX(t)` equals unified tMax,
 *   so a hypothetical revert to per-sat tMax at
 *   `uneri/src/worker/multiChartDataWorker.ts:252-263` would still produce
 *   matching bucket boundaries and this test would pass.
 *   The SQL-level "same tMax → same bucket boundaries" property is covered
 *   by `uneri/src/db/store.test.ts:259-276`; the worker → `queryDerived`
 *   wiring with unified tMax is currently UNCOVERED by any test. A follow-up
 *   would need either a skewed-data E2E with carefully tuned assertions, or
 *   a mocked-DuckDB worker integration test.
 *
 * Historical note: an earlier version of this test queried DuckDB tables
 * directly via `window.__duckdb_conn` and re-implemented the bucketing SQL
 * in the test body. When the multi-sat chart pipeline moved into a Web
 * Worker (commit `b783463`), the main-thread DuckDB connection disappeared
 * and that approach stopped working. The current test reads the higher-
 * level `MultiChartDataMap` payload — the deserialized output of the
 * worker's `multi-chart-data` broadcast — which is stable across refactors
 * of the worker's internals.
 */

import type { AddressInfo } from "node:net";
import { expect, test } from "@playwright/test";
import { WebSocketServer, type WebSocket as WsSocket } from "ws";

/** Build a state message for a circular orbit. */
function stateMsg(entityPath: string, t: number, altitude: number) {
  const r = 6378.137 + altitude;
  const mu = 398600.4418;
  const v = Math.sqrt(mu / r);
  const period = 2 * Math.PI * Math.sqrt(r ** 3 / mu);
  const theta = (t / period) * 2 * Math.PI;
  return JSON.stringify({
    type: "state",
    entity_path: entityPath,
    t,
    position: [r * Math.cos(theta), r * Math.sin(theta), 0],
    velocity: [-v * Math.sin(theta), v * Math.cos(theta), 0],
    semi_major_axis: r,
    eccentricity: 0.001,
    inclination: entityPath === "sso" ? 1.7209 : 0.9006,
    raan: 0,
    argument_of_periapsis: 0,
    true_anomaly: theta,
  });
}

test.describe("multi-satellite NaN alignment", () => {
  let wss: WebSocketServer;

  test.beforeEach(async () => {
    wss = new WebSocketServer({ port: 0 });
  });

  test.afterEach(async () => {
    wss.close();
  });

  test("aligned multi-sat chart data has matching series lengths and no NaN gaps", async ({
    page,
  }) => {
    const consoleLogs: string[] = [];
    page.on("console", (msg) => consoleLogs.push(msg.text()));

    const DT = 1;
    // Row count per sat must exceed `MultiChartDataWorkerClient`'s
    // default `maxPoints` (2000) so the SQL downsample's
    // `total <= maxPts OR rn = 1` fast path in `buildDerivedQuery`
    // does NOT return raw rows. Only when bucketing is actually
    // triggered is the per-sat vs unified tMax distinction observable
    // — that's the regression class this test exists to guard.
    // 3601 rows (t ∈ [0, 3600] at dt=1s) clears the 2000 threshold
    // comfortably.
    const T_END = 3600;

    wss.on("connection", (ws: WsSocket) => {
      ws.send(
        JSON.stringify({
          type: "info",
          mu: 398600.4418,
          dt: DT,
          output_interval: DT,
          stream_interval: DT,
          central_body: "earth",
          central_body_radius: 6378.137,
          epoch_jd: null,
          satellites: [
            { id: "sso", name: "SSO 800km", altitude: 800, period: 6052 },
            { id: "iss", name: "ISS", altitude: 400, period: 5554 },
          ],
        }),
      );

      // Both sats share the exact same time range at the same cadence,
      // so a correct unified-tMax downsample + alignTimeSeries produces
      // matched timestamps with no NaN. A regression where per-sat
      // tMax drifts would show up as NaN in `values[i]` after alignment
      // because each sat's bucket representatives would fall on
      // different timestamps.
      const historyStates = [];
      for (let t = 0; t <= T_END; t += DT) {
        const ssoState = JSON.parse(stateMsg("sso", t, 800));
        delete ssoState.type;
        historyStates.push(ssoState);
        const issState = JSON.parse(stateMsg("iss", t, 400));
        delete issState.type;
        historyStates.push(issState);
      }
      ws.send(JSON.stringify({ type: "history", states: historyStates }));
      // Intentionally no live streaming. Streaming introduces small
      // per-sat `MAX(t)` drift (because the two sats' ingest/drain/tick
      // cadences don't line up perfectly), and `alignTimeSeries` then
      // pads the mismatched tail with NaN via the
      // `UNION ... WHERE t = MAX(t)` branch in `buildDerivedQuery`.
      //
      // Pure history gives both tables the exact same row set, which
      // keeps the NaN assertion below stable. The tradeoff: the
      // streaming path's own alignment behavior is not covered here
      // (see the "DOES NOT gate" section of the file header).
    });

    // noAutoConnect suppresses the default auto-connect, avoiding a race
    // with the CI shared server and ensuring a clean worker state.
    await page.goto("/?noAutoConnect=1");

    // Connect to mock server
    const urlInput = page.locator('[data-testid="ws-url-input"]');
    const mockPort = (wss.address() as AddressInfo).port;
    await urlInput.fill(`ws://localhost:${mockPort}`);
    const connectBtn = page.locator('[data-testid="ws-connect-btn"]');
    await connectBtn.click();

    const statusText = page.locator('[data-testid="ws-status-text"]');
    await expect(statusText).toHaveText("Connected", { timeout: 5000 });

    // Wait for simInfo to reflect 2 satellites so `isMultiSatellite` flips
    // and `useMultiSatelliteStoreWorker` gets instantiated.
    await expect(async () => {
      const satCount = await page.evaluate(() => {
        const sel = document.querySelector('[data-testid="frame-selector-select"]');
        // satellite options = total options - 1 (central body)
        return sel ? sel.querySelectorAll("option").length - 1 : 0;
      });
      expect(satCount).toBeGreaterThanOrEqual(2);
    }).toPass({ timeout: 10000, intervals: [200, 500, 1000, 2000] });

    // Wait for the multi-sat worker to produce its first chart broadcast.
    // The worker initializes DuckDB (WASM, can be slow on CI), ingests the
    // history, runs its first tick, and posts `multi-chart-data` → main
    // thread deserializes and the dev-mode effect sets the window global.
    await expect(async () => {
      const hasData = await page.evaluate(() => {
        const w = window as Record<string, unknown>;
        const data = w.__debug_multi_chart_data as Record<string, unknown> | null | undefined;
        if (data == null) return false;
        return Object.values(data).some(
          (series) =>
            series != null &&
            typeof series === "object" &&
            "t" in series &&
            (series as { t: Float64Array }).t.length > 0,
        );
      });
      expect(hasData, "multi-sat chart data not yet populated").toBe(true);
    }).toPass({ timeout: 30000, intervals: [500, 1000, 2000, 3000] });

    // Disconnect via the UI button to set manualDisconnectRef and
    // suppress auto-reconnect (a server-side close would trigger a
    // buffer reset that clobbers the data we're about to inspect).
    await page.locator('[data-testid="ws-disconnect-btn"]').click();
    // Let the worker's final tick broadcast settle (tick every ~2s).
    await page.waitForTimeout(2500);

    // Snapshot the aligned chart data and run assertions entirely on
    // the main thread via `page.evaluate`. All properties we care about
    // (per-series NaN count, length consistency, metric count) are
    // observable from the deserialized `MultiChartDataMap`.
    const result = await page.evaluate(() => {
      const w = window as Record<string, unknown>;
      const data = w.__debug_multi_chart_data as Record<
        string,
        {
          t: Float64Array;
          values: Float64Array[];
          series: Array<{ label: string; color: string }>;
        } | null
      > | null;
      if (!data) return { error: "__debug_multi_chart_data is null" };

      const metrics: Array<{
        name: string;
        tLen: number;
        seriesCount: number;
        seriesLengths: number[];
        nanPerSeries: number[];
        tSpan: number;
      }> = [];

      for (const [name, series] of Object.entries(data)) {
        if (series == null) continue;
        const tLen = series.t.length;
        const seriesLengths = series.values.map((v) => v.length);
        const nanPerSeries = series.values.map((v) => {
          let n = 0;
          for (const x of v) if (Number.isNaN(x)) n++;
          return n;
        });
        const tSpan = tLen > 0 ? series.t[tLen - 1] - series.t[0] : 0;
        metrics.push({
          name,
          tLen,
          seriesCount: series.values.length,
          seriesLengths,
          nanPerSeries,
          tSpan,
        });
      }
      return { metrics };
    });

    console.log("Multi chart data result:", JSON.stringify(result, null, 2));

    expect(result).not.toHaveProperty("error");
    const { metrics } = result as {
      metrics: Array<{
        name: string;
        tLen: number;
        seriesCount: number;
        seriesLengths: number[];
        nanPerSeries: number[];
        tSpan: number;
      }>;
    };

    // At least one metric with data must be present. (If the tick loop
    // had not fired yet, the wait loop above would have timed out.)
    expect(metrics.length, "at least one metric should be populated").toBeGreaterThan(0);

    // For every metric: two satellites should be represented, per-series
    // lengths must equal the aligned `t` length (that is the alignment
    // contract), and NaN count per series must be very low. A broken
    // per-sat tMax pre-alignment would produce many NaN entries after
    // `alignTimeSeries` pads unmatched timestamps.
    for (const m of metrics) {
      expect(m.seriesCount, `${m.name}: should have 2 satellite series`).toBe(2);
      // With 3601 rows per sat and `maxPoints=2000`, the bucketing
      // path must actually fire and produce close-to-2000 aligned
      // points. A value near 3601 would mean bucketing was bypassed
      // (the `total <= maxPts` fast path) and this test lost its
      // grip on the unified-tMax property.
      expect(
        m.tLen,
        `${m.name}: aligned t should come from bucketed output, not raw passthrough`,
      ).toBeGreaterThan(500);
      expect(m.tLen, `${m.name}: aligned t should not exceed the downsample cap`).toBeLessThan(
        2500,
      );
      for (let i = 0; i < m.seriesLengths.length; i++) {
        expect(
          m.seriesLengths[i],
          `${m.name}: series ${i} length must equal t length (alignment invariant)`,
        ).toBe(m.tLen);
        // Allow a very small slack for edge alignment effects (≤ 2
        // unmatched points). A broken per-sat-tMax regression would
        // produce dozens to hundreds of NaNs because each sat's
        // bucket boundaries would drift and `alignTimeSeries` would
        // union with NaN padding.
        expect(
          m.nanPerSeries[i],
          `${m.name}: series ${i} NaN count (${m.nanPerSeries[i]}) should be ≤ 2`,
        ).toBeLessThanOrEqual(2);
      }
      // Time span should cover a meaningful portion of the 3600s of
      // history we sent.
      expect(m.tSpan, `${m.name}: t span should cover most of the sim`).toBeGreaterThan(1000);
    }

    // No console errors about malformed inserts / DuckDB failures.
    const criticalErrors = consoleLogs.filter(
      (l) => l.includes("undefined") && (l.includes("INSERT") || l.includes("DuckDB")),
    );
    expect(criticalErrors).toHaveLength(0);
  });
});
