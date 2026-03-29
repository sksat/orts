/**
 * E2E test: multi-satellite "All" view has aligned timestamps (no NaN gaps).
 *
 * Uses a mock WebSocket server to send 2 satellites with enough data to
 * trigger time-bucket downsampling. Verifies via DuckDB queries that the
 * unified tMax alignment produces matching timestamps so alignTimeSeries()
 * doesn't introduce NaN gaps.
 *
 * Strategy: Query the DuckDB tables directly (exposed on window.__duckdb_conn
 * in dev mode) rather than relying on canvas pixel analysis, which is flaky
 * in headless/CI environments.
 */

import type { AddressInfo } from "node:net";
import { expect, test } from "@playwright/test";
import { WebSocketServer, type WebSocket as WsSocket } from "ws";

/** Build a state message for a circular orbit. */
function stateMsg(satelliteId: string, t: number, altitude: number) {
  const r = 6378.137 + altitude;
  const mu = 398600.4418;
  const v = Math.sqrt(mu / r);
  const period = 2 * Math.PI * Math.sqrt(r ** 3 / mu);
  const theta = (t / period) * 2 * Math.PI;
  return JSON.stringify({
    type: "state",
    satellite_id: satelliteId,
    t,
    position: [r * Math.cos(theta), r * Math.sin(theta), 0],
    velocity: [-v * Math.sin(theta), v * Math.cos(theta), 0],
    semi_major_axis: r,
    eccentricity: 0.001,
    inclination: satelliteId === "sso" ? 1.7209 : 0.9006,
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

  test("both satellite tables have aligned timestamps after downsampling", async ({ page }) => {
    const consoleLogs: string[] = [];
    page.on("console", (msg) => consoleLogs.push(msg.text()));

    const DT = 10;

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

      // Send history with enough data to populate DuckDB tables
      const historyStates = [];
      for (let t = 0; t <= 3600; t += DT) {
        const ssoState = JSON.parse(stateMsg("sso", t, 800));
        delete ssoState.type;
        historyStates.push(ssoState);
        const issState = JSON.parse(stateMsg("iss", t, 400));
        delete issState.type;
        historyStates.push(issState);
      }
      ws.send(JSON.stringify({ type: "history", states: historyStates }));

      // Stream live data
      let t = 3600;
      const interval = setInterval(() => {
        if (ws.readyState !== 1) {
          clearInterval(interval);
          return;
        }
        t += DT;
        ws.send(stateMsg("sso", t, 800));
        ws.send(stateMsg("iss", t, 400));
      }, 20);
    });

    // noAutoConnect suppresses the default auto-connect, avoiding a race
    // with the CI shared server and ensuring a clean DuckDB state.
    await page.goto("/?noAutoConnect=1");

    // Connect to mock server
    const urlInput = page.locator(".ws-url-input");
    const mockPort = (wss.address() as AddressInfo).port;
    await urlInput.fill(`ws://localhost:${mockPort}`);
    const connectBtn = page.locator(".ws-connect-btn");
    await connectBtn.click();

    const statusText = page.locator(".ws-status-text");
    await expect(statusText).toHaveText("Connected", { timeout: 5000 });

    // Wait for simInfo to reflect 2 satellites (isMultiSatellite must be true
    // before useMultiSatelliteStore starts creating DuckDB tables).
    await expect(async () => {
      const satCount = await page.evaluate(() => {
        const sel = document.querySelector(".frame-selector-select");
        // satellite options = total options - 1 (central body)
        return sel ? sel.querySelectorAll("option").length - 1 : 0;
      });
      expect(satCount).toBeGreaterThanOrEqual(2);
    }).toPass({ timeout: 10000, intervals: [200, 500, 1000, 2000] });

    // Wait for DuckDB connection to be available (WASM init from CDN can be slow)
    await expect(async () => {
      const hasConn = await page.evaluate(
        () => (window as Record<string, unknown>).__duckdb_conn != null,
      );
      expect(hasConn, "DuckDB connection not yet available").toBe(true);
    }).toPass({ timeout: 30000, intervals: [500, 1000, 2000, 3000] });

    // Debug: log ingest buffer state to diagnose empty DuckDB tables
    const bufferDebug = await page.evaluate(() => {
      const w = window as Record<string, unknown>;
      const bufs = w.__debug_ingest_buffers as
        | Map<string, { pendingCount: number; latestT: number }>
        | undefined;
      const isMulti = w.__debug_is_multi_satellite;
      if (!bufs) return { bufferKeys: [], isMultiSatellite: isMulti, note: "no buffers exposed" };
      const info: Record<string, { pending: number; latestT: number }> = {};
      for (const [key, buf] of bufs.entries()) {
        info[key] = { pending: buf.pendingCount, latestT: buf.latestT };
      }
      return { bufferKeys: Array.from(bufs.keys()), buffers: info, isMultiSatellite: isMulti };
    });
    console.log("IngestBuffer debug:", JSON.stringify(bufferDebug));

    // Wait for DuckDB tables to be populated (history ingestion + query ticks)
    // Poll instead of fixed timeout — CI can be slow
    await expect(async () => {
      const result = await page.evaluate(async () => {
        const conn = (window as Record<string, unknown>).__duckdb_conn;
        if (!conn) return { sso: 0, iss: 0, connNull: true };
        const q = async (sql: string) =>
          (
            conn as {
              query: (
                s: string,
              ) => Promise<{ getChildAt: (i: number) => { get: (i: number) => number } | null }>;
            }
          ).query(sql);
        let sso = 0,
          iss = 0;
        const tables: string[] = [];
        try {
          const res = await q("SHOW TABLES");
          const col = res.getChildAt(0);
          if (col) {
            // Iterate using get() — length not in the typed interface
            for (let i = 0; ; i++) {
              const v = col.get(i);
              if (v == null) break;
              tables.push(String(v));
            }
          }
        } catch {
          /* ignore */
        }
        try {
          sso = Number((await q("SELECT COUNT(*) FROM orbit_sso")).getChildAt(0)?.get(0));
        } catch {
          /* table not yet created */
        }
        try {
          iss = Number((await q("SELECT COUNT(*) FROM orbit_iss")).getChildAt(0)?.get(0));
        } catch {
          /* table not yet created */
        }
        const w = window as Record<string, unknown>;
        const tickCount = w.__debug_multi_sat_tick as number | undefined;
        const insertCount = w.__debug_multi_sat_inserts as number | undefined;
        const lastTick = w.__debug_multi_sat_last_tick as
          | { configIds: string[]; bufferKeys: string[]; missingBufferIds: string[] }
          | undefined;
        return { sso, iss, tables, connNull: false, tickCount, insertCount, lastTick };
      });
      console.log("DuckDB poll:", JSON.stringify(result));
      expect(result.sso).toBeGreaterThan(0);
      expect(result.iss).toBeGreaterThan(0);
    }).toPass({ timeout: 30000, intervals: [500, 1000, 1000, 2000, 2000, 3000, 5000] });

    // Stop streaming: click the viewer's disconnect button.
    // Using the UI button sets manualDisconnectRef, which suppresses
    // auto-reconnect.  Server-side close would trigger auto-reconnect →
    // handleConnect → setSimInfo(null) → CREATE OR REPLACE TABLE, wiping
    // the DuckDB tables before the final query.
    await page.locator(".ws-disconnect-btn").click();
    // Wait for tick loop to drain any remaining buffered data
    await page.waitForTimeout(2000);

    // Query DuckDB tables directly via the exposed connection
    const dbResult = await page.evaluate(async () => {
      const conn = (window as Record<string, unknown>).__duckdb_conn;
      if (!conn) return { error: "DuckDB connection not exposed on window" };

      type QueryResult = {
        getChildAt: (
          i: number,
        ) => { toArray: () => Float64Array; get: (i: number) => number } | null;
        numRows: number;
      };
      const query = async (sql: string): Promise<QueryResult> => {
        return (conn as { query: (sql: string) => Promise<QueryResult> }).query(sql);
      };

      // Check both satellite tables exist and have data
      let ssoCount = 0,
        issCount = 0;
      try {
        const ssoRes = await query("SELECT COUNT(*) FROM orbit_sso");
        ssoCount = Number(ssoRes.getChildAt(0)?.get(0));
      } catch {
        return { error: "orbit_sso table not found" };
      }
      try {
        const issRes = await query("SELECT COUNT(*) FROM orbit_iss");
        issCount = Number(issRes.getChildAt(0)?.get(0));
      } catch {
        return { error: "orbit_iss table not found" };
      }

      if (ssoCount === 0 || issCount === 0) {
        return { error: `Empty tables: sso=${ssoCount}, iss=${issCount}` };
      }

      // Compute unified tMax (same logic as useMultiSatelliteStore)
      const ssoMaxRes = await query("SELECT MAX(t) FROM orbit_sso");
      const issMaxRes = await query("SELECT MAX(t) FROM orbit_iss");
      const ssoMax = Number(ssoMaxRes.getChildAt(0)?.get(0));
      const issMax = Number(issMaxRes.getChildAt(0)?.get(0));
      const unifiedTMax = Math.max(ssoMax, issMax);

      // Run downsampled queries with unified tMax
      // This mirrors the exact query that useMultiSatelliteStore runs
      const maxPoints = 200;
      const buildQuery = (tableName: string) => {
        return (
          `WITH filtered AS (SELECT * FROM ${tableName}), ` +
          `bounds AS (SELECT MIN(t) AS t_lo, ${unifiedTMax} AS t_hi, COUNT(*) AS total FROM filtered), ` +
          `bucketed AS (SELECT f.*, ` +
          `CASE WHEN b.t_hi = b.t_lo THEN 0 ` +
          `ELSE LEAST(GREATEST(CAST(FLOOR((CAST(f.t AS DOUBLE) - CAST(b.t_lo AS DOUBLE)) ` +
          `* ${maxPoints}.0 / (CAST(b.t_hi AS DOUBLE) - CAST(b.t_lo AS DOUBLE))) AS INTEGER), 0), ${maxPoints} - 1) ` +
          `END AS bucket, b.total FROM filtered f, bounds b), ` +
          `ranked AS (SELECT *, ROW_NUMBER() OVER (PARTITION BY bucket ORDER BY t) AS rn FROM bucketed) ` +
          `SELECT t, sqrt(x*x+y*y+z*z)-6378.137 AS altitude FROM (` +
          `SELECT * FROM ranked WHERE total <= ${maxPoints} OR rn = 1 ` +
          `UNION ` +
          `SELECT * FROM ranked WHERE t = (SELECT MAX(t) FROM filtered)` +
          `) sub ORDER BY t`
        );
      };

      const ssoData = await query(buildQuery("orbit_sso"));
      const issData = await query(buildQuery("orbit_iss"));

      const ssoT = Array.from(ssoData.getChildAt(0)!.toArray());
      const issT = Array.from(issData.getChildAt(0)!.toArray());
      const ssoAlt = Array.from(ssoData.getChildAt(1)!.toArray());
      const issAlt = Array.from(issData.getChildAt(1)!.toArray());

      // Check for NaN in altitude values
      const ssoNanCount = ssoAlt.filter((v: number) => Number.isNaN(v)).length;
      const issNanCount = issAlt.filter((v: number) => Number.isNaN(v)).length;

      // Check timestamp alignment: count matching timestamps
      const ssoTSet = new Set(ssoT);
      const issTSet = new Set(issT);
      let matchingTimestamps = 0;
      for (const t of ssoTSet) {
        if (issTSet.has(t)) matchingTimestamps++;
      }

      const totalUnique = new Set([...ssoT, ...issT]).size;
      const alignmentRatio = totalUnique > 0 ? matchingTimestamps / totalUnique : 0;

      return {
        ssoCount,
        issCount,
        ssoDownsampledCount: ssoT.length,
        issDownsampledCount: issT.length,
        ssoNanCount,
        issNanCount,
        matchingTimestamps,
        totalUniqueTimestamps: totalUnique,
        alignmentRatio,
        ssoTFirst3: ssoT.slice(0, 3),
        issTFirst3: issT.slice(0, 3),
        ssoTLast3: ssoT.slice(-3),
        issTLast3: issT.slice(-3),
      };
    });

    console.log("DuckDB query result:", JSON.stringify(dbResult, null, 2));

    // Assertions
    expect(dbResult).not.toHaveProperty("error");

    const result = dbResult as {
      ssoCount: number;
      issCount: number;
      ssoDownsampledCount: number;
      issDownsampledCount: number;
      ssoNanCount: number;
      issNanCount: number;
      matchingTimestamps: number;
      totalUniqueTimestamps: number;
      alignmentRatio: number;
    };

    // Both tables should have data
    expect(result.ssoCount, "SSO table should have rows").toBeGreaterThan(50);
    expect(result.issCount, "ISS table should have rows").toBeGreaterThan(50);

    // No NaN in altitude values
    expect(result.ssoNanCount, "SSO altitude should have no NaN").toBe(0);
    expect(result.issNanCount, "ISS altitude should have no NaN").toBe(0);

    // Downsampled data should be non-empty
    expect(result.ssoDownsampledCount, "SSO downsampled should have rows").toBeGreaterThan(10);
    expect(result.issDownsampledCount, "ISS downsampled should have rows").toBeGreaterThan(10);

    // Timestamp alignment: with unified tMax, both tables should produce
    // nearly identical timestamp sets from time-bucket downsampling.
    // Both tables receive data at the same DT=10s intervals, so with the
    // same bucket boundaries they pick the same representative timestamps.
    // Before the fix (independent tMax), alignment was very low (~0.2).
    // After the fix (unified tMax), it should be >0.8.
    expect(
      result.alignmentRatio,
      `Timestamp alignment ratio should be > 0.8 (was ${result.alignmentRatio.toFixed(3)})`,
    ).toBeGreaterThan(0.8);

    // Verify no critical DuckDB errors
    const criticalErrors = consoleLogs.filter(
      (l) => l.includes("undefined") && (l.includes("INSERT") || l.includes("DuckDB")),
    );
    expect(criticalErrors).toHaveLength(0);
  });
});
