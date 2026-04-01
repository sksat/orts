import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { useEffect, useRef, useState } from "react";
import {
  type ChartDataMap,
  COMPACT_DEFAULTS,
  type CompactOptions,
  clearTable,
  compactTable,
  createTable,
  DISPLAY_MAX_POINTS,
  type IngestBuffer,
  insertPoints,
  queryDerived,
  type TableSchema,
  type TimePoint,
  type TimeRange,
} from "uneri";
import {
  buildMultiChartData,
  type MultiChartDataMap,
  type SatelliteConfig,
} from "./buildMultiChartData.js";
import { computeUnifiedTMin } from "./computeGlobalLatestT.js";

export type { MultiChartDataMap, SatelliteConfig } from "./buildMultiChartData.js";
export { buildMultiChartData } from "./buildMultiChartData.js";

export interface UseMultiSatelliteStoreOptions<T extends TimePoint> {
  conn: AsyncDuckDBConnection | null;
  /** Base schema (used to create per-satellite tables with different names). */
  baseSchema: TableSchema<T>;
  satelliteConfigs: SatelliteConfig[];
  ingestBuffers: Map<string, IngestBuffer<T>>;
  /** Derived metric names to include in the output. */
  metricNames: string[];
  timeRange?: TimeRange;
  maxPoints?: number;
  tickInterval?: number;
  queryEveryN?: number;
  compactEveryN?: number;
  compactOptions?: CompactOptions;
}

export interface UseMultiSatelliteStoreReturn {
  data: MultiChartDataMap | null;
  isLoading: boolean;
}

/** Create a per-satellite schema with a unique table name. */
function makeSatelliteSchema<T extends TimePoint>(
  baseSchema: TableSchema<T>,
  entityPath: string,
): TableSchema<T> {
  const safeName = entityPath.replace(/[^a-zA-Z0-9_]/g, "_");
  return { ...baseSchema, tableName: `orbit_${safeName}` };
}

/**
 * Hook that manages N DuckDB tables (one per satellite) and produces
 * aligned MultiChartDataMap for multi-series chart rendering.
 */
export function useMultiSatelliteStore<T extends TimePoint>(
  options: UseMultiSatelliteStoreOptions<T>,
): UseMultiSatelliteStoreReturn {
  const {
    conn,
    baseSchema,
    satelliteConfigs,
    ingestBuffers,
    metricNames,
    timeRange = null,
    maxPoints = DISPLAY_MAX_POINTS,
    tickInterval = 500,
    queryEveryN = 4,
    compactEveryN = 20,
    compactOptions = COMPACT_DEFAULTS,
  } = options;

  const [data, setData] = useState<MultiChartDataMap | null>(null);
  const [isLoading, setIsLoading] = useState(false);

  // Refs for stable access in tick loop
  const timeRangeRef = useRef(timeRange);
  timeRangeRef.current = timeRange;
  const maxPointsRef = useRef(maxPoints);
  maxPointsRef.current = maxPoints;
  const configsRef = useRef(satelliteConfigs);
  configsRef.current = satelliteConfigs;
  const buffersRef = useRef(ingestBuffers);
  buffersRef.current = ingestBuffers;
  const metricNamesRef = useRef(metricNames);
  metricNamesRef.current = metricNames;
  const baseSchemaRef = useRef(baseSchema);
  baseSchemaRef.current = baseSchema;

  useEffect(() => {
    if (!conn) return;

    let cancelled = false;
    const timerRef = { current: 0 };
    let tickCount = 0;

    // Dev-only: expose tick counter for E2E diagnostics
    if (typeof window !== "undefined" && import.meta.env.DEV) {
      (window as unknown as Record<string, unknown>).__debug_multi_sat_tick = 0;
      (window as unknown as Record<string, unknown>).__debug_multi_sat_inserts = 0;
    }
    let queryCount = 0;
    const COMPACT_COOLDOWN_AFTER_REBUILD = 5;
    const compactCooldowns = new Map<string, number>();
    const createdTables = new Set<string>();
    const hasData = new Set<string>();

    const ensureTable = async (satId: string) => {
      if (createdTables.has(satId)) return;
      const schema = makeSatelliteSchema(baseSchemaRef.current, satId);
      await createTable(conn, schema);
      createdTables.add(satId);
    };

    const startPolling = async () => {
      for (const cfg of configsRef.current) {
        try {
          await ensureTable(cfg.id);
        } catch (e) {
          console.warn(`useMultiSatelliteStore: failed to create table for ${cfg.id}:`, e);
        }
      }
      if (cancelled) return;
      setIsLoading(false);

      const tick = async () => {
        if (cancelled) return;

        // Dev-only: increment tick counter and expose loop state for E2E diagnostics
        if (typeof window !== "undefined" && import.meta.env.DEV) {
          const w = window as unknown as Record<string, unknown>;
          w.__debug_multi_sat_tick = ((w.__debug_multi_sat_tick as number) ?? 0) + 1;
          const configIds = configsRef.current.map((c) => c.id);
          const bufferKeys = Array.from(buffersRef.current.keys());
          const missingBufferIds = configIds.filter((id) => !buffersRef.current.has(id));
          w.__debug_multi_sat_last_tick = { configIds, bufferKeys, missingBufferIds };
        }

        // Dev-only: per-tick ingest diagnostics
        const tickIngest: Array<{
          id: string;
          rebuildLen: number | null;
          drainLen: number;
          ensureFailed: boolean;
        }> = [];

        for (const cfg of configsRef.current) {
          const buf = buffersRef.current.get(cfg.id);
          if (!buf) continue;

          try {
            await ensureTable(cfg.id);
          } catch (e) {
            if (typeof window !== "undefined" && import.meta.env.DEV) {
              console.warn(`useMultiSatelliteStore: ensureTable failed for ${cfg.id}:`, e);
            }
            tickIngest.push({ id: cfg.id, rebuildLen: null, drainLen: 0, ensureFailed: true });
            continue;
          }

          const schema = makeSatelliteSchema(baseSchemaRef.current, cfg.id);

          const rebuildData = buf.consumeRebuild();
          if (rebuildData !== null) {
            tickIngest.push({
              id: cfg.id,
              rebuildLen: rebuildData.length,
              drainLen: 0,
              ensureFailed: false,
            });
            try {
              await clearTable(conn, schema);
              await insertPoints(conn, schema, rebuildData);
              if (rebuildData.length > 0) {
                hasData.add(cfg.id);
                if (typeof window !== "undefined" && import.meta.env.DEV) {
                  (window as unknown as Record<string, unknown>).__debug_multi_sat_inserts =
                    ((window as unknown as Record<string, unknown>)
                      .__debug_multi_sat_inserts as number) + rebuildData.length;
                }
              }
              compactCooldowns.set(cfg.id, COMPACT_COOLDOWN_AFTER_REBUILD);
            } catch (e) {
              console.warn(`useMultiSatelliteStore: rebuild failed for ${cfg.id}:`, e);
              buf.markRebuild(rebuildData);
              // Dev-only: expose the error for E2E diagnostics
              if (typeof window !== "undefined" && import.meta.env.DEV) {
                (window as unknown as Record<string, unknown>).__debug_multi_sat_last_error = {
                  entityPath: cfg.id,
                  error: e instanceof Error ? e.message : String(e),
                  stack: e instanceof Error ? e.stack : undefined,
                };
              }
            }
          } else {
            const newPoints = buf.drain();
            tickIngest.push({
              id: cfg.id,
              rebuildLen: null,
              drainLen: newPoints.length,
              ensureFailed: false,
            });
            if (newPoints.length > 0) {
              try {
                await insertPoints(conn, schema, newPoints);
                hasData.add(cfg.id);
              } catch (e) {
                console.warn(`useMultiSatelliteStore: insert failed for ${cfg.id}:`, e);
                buf.pushMany(newPoints);
              }
            }
          }
        }

        // Dev-only: expose per-tick ingest details
        if (typeof window !== "undefined" && import.meta.env.DEV && tickIngest.length > 0) {
          (window as unknown as Record<string, unknown>).__debug_multi_sat_last_ingest = tickIngest;
        }

        tickCount++;
        if (hasData.size > 0 && tickCount % queryEveryN === 0) {
          try {
            const perSatData = new Map<string, ChartDataMap>();

            // Use a unified tMin across all satellites so they share the same
            // time window. Without this, a terminated satellite's frozen latestT
            // would cause its query to cover a stale range, creating a wide gap
            // in the aligned chart time axis.
            // Returns undefined for "All" mode or when no buffers have data,
            // preventing invalid -Infinity SQL values.
            const tMin = computeUnifiedTMin(timeRangeRef.current, buffersRef.current);

            // Compute unified tMax across all satellite tables so that
            // time-bucket downsampling produces aligned bucket boundaries.
            // Without this, each table independently computes MAX(t) as t_hi,
            // leading to different bucket edges and misaligned timestamps that
            // cause NaN gaps when series are merged in alignTimeSeries().
            let unifiedTMax: number | undefined;
            if (hasData.size > 1) {
              let maxT = -Infinity;
              for (const cfg of configsRef.current) {
                if (!hasData.has(cfg.id)) continue;
                const schema = makeSatelliteSchema(baseSchemaRef.current, cfg.id);
                const res = await conn.query(`SELECT MAX(t) AS t_max FROM ${schema.tableName}`);
                const val = Number(res.getChildAt(0)?.get(0));
                if (Number.isFinite(val) && val > maxT) maxT = val;
              }
              if (Number.isFinite(maxT)) unifiedTMax = maxT;
            }

            for (const cfg of configsRef.current) {
              if (!hasData.has(cfg.id)) continue;
              const schema = makeSatelliteSchema(baseSchemaRef.current, cfg.id);
              const result = await queryDerived(
                conn,
                schema,
                tMin,
                maxPointsRef.current,
                unifiedTMax,
              );
              if (result.t.length > 0) {
                perSatData.set(cfg.id, result);
              }
            }

            if (!cancelled && perSatData.size > 0) {
              const multiData = buildMultiChartData(
                perSatData,
                metricNamesRef.current,
                configsRef.current,
              );
              setData(multiData);
            }

            queryCount++;
            if (queryCount % compactEveryN === 0) {
              for (const cfg of configsRef.current) {
                if (!hasData.has(cfg.id)) continue;
                const cd = compactCooldowns.get(cfg.id) ?? 0;
                if (cd > 0) {
                  compactCooldowns.set(cfg.id, cd - 1);
                  continue;
                }
                const schema = makeSatelliteSchema(baseSchemaRef.current, cfg.id);
                try {
                  await compactTable(conn, schema, compactOptions);
                } catch (e) {
                  console.warn(`useMultiSatelliteStore: compact failed for ${cfg.id}:`, e);
                }
              }
            }
          } catch (e) {
            console.warn("useMultiSatelliteStore: query failed:", e);
          }
        }

        if (!cancelled) {
          timerRef.current = window.setTimeout(tick, tickInterval) as unknown as number;
        }
      };

      timerRef.current = window.setTimeout(tick, tickInterval) as unknown as number;
    };

    setIsLoading(true);
    startPolling();

    return () => {
      cancelled = true;
      clearTimeout(timerRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn]);

  return { data, isLoading };
}
