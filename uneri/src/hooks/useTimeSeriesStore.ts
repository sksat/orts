import type { AsyncDuckDBConnection } from "@duckdb/duckdb-wasm";
import { useEffect, useRef, useState } from "react";
import type { IngestBuffer } from "../db/IngestBuffer.js";
import type { CompactOptions } from "../db/store.js";
import {
  COMPACT_DEFAULTS,
  clearTable,
  compactTable,
  insertPoints,
  queryDerived,
  queryDerivedIncremental,
  replacePoints,
} from "../db/store.js";
import type { ChartDataMap, TableSchema, TimePoint } from "../types.js";
import { mergeChartData, trimChartDataLeft } from "../utils/mergeChartData.js";

/** Maximum number of points to display in charts. Query-time downsampling
 *  keeps chart rendering fast regardless of total data in DuckDB. */
export const DISPLAY_MAX_POINTS = 2000;

/** Time range for chart display: null = all history, number = last N seconds. */
export type TimeRange = number | null;

/** Compute the minimum t value for a chart query given a time range window. */
export function computeTMin(timeRange: TimeRange, latestT: number): number | undefined {
  if (timeRange == null) return undefined;
  return latestT - timeRange;
}

export interface UseTimeSeriesStoreOptions<T extends TimePoint> {
  conn: AsyncDuckDBConnection | null;
  schema: TableSchema<T>;
  ingestBufferRef: React.RefObject<IngestBuffer<T>>;
  /** Show only last N seconds of data, or null for all history. */
  timeRange?: TimeRange;
  /** Maximum number of points to display (default: DISPLAY_MAX_POINTS). */
  maxPoints?: number;
  /** Polling interval in ms for realtime mode (default: 250). */
  tickInterval?: number;
  /** Run cold (full downsampled) refresh every Nth tick (default: 20). */
  coldRefreshEveryN?: number;
  /** Trigger cold refresh when hot buffer exceeds this many rows (default: 500). */
  hotRowBudget?: number;
  /** Run compaction check every Nth cold refresh (default: 5). */
  compactEveryN?: number;
  /** Compaction configuration (default: COMPACT_DEFAULTS). */
  compactOptions?: CompactOptions;
}

export interface UseTimeSeriesStoreReturn {
  data: ChartDataMap | null;
  isLoading: boolean;
}

export function useTimeSeriesStore<T extends TimePoint>(
  options: UseTimeSeriesStoreOptions<T>,
): UseTimeSeriesStoreReturn {
  const {
    conn,
    schema,
    ingestBufferRef,
    timeRange = null,
    maxPoints = DISPLAY_MAX_POINTS,
    tickInterval = 250,
    coldRefreshEveryN = 20,
    hotRowBudget = 500,
    compactEveryN = 5,
    compactOptions = COMPACT_DEFAULTS,
  } = options;

  const [data, setData] = useState<ChartDataMap | null>(null);
  const [isLoading, _setIsLoading] = useState(false);
  const queryTimerRef = useRef<number>(0);
  const hasDataRef = useRef(false);

  // Refs to avoid stale closures in realtime tick loop
  const timeRangeRef = useRef(timeRange);
  timeRangeRef.current = timeRange;
  const schemaRef = useRef(schema);
  schemaRef.current = schema;
  const maxPointsRef = useRef(maxPoints);
  maxPointsRef.current = maxPoints;

  // IngestBuffer-based: cold snapshot + hot buffer architecture.
  // All data sources (WS streaming, CSV chunks, etc.) flow through IngestBuffer.
  // Use markRebuild() for bulk initial data, push() for streaming additions.
  useEffect(() => {
    if (!conn) return;

    let cancelled = false;
    let coldQueryCount = 0;
    /** Cooldown: skip compaction checks after a rebuild to avoid
     *  immediately deleting newly inserted detail data. */
    const COMPACT_COOLDOWN_AFTER_REBUILD = 5;
    let compactCooldown = 0;
    /** Give up on a failing rebuild after this many retries. */
    const MAX_REBUILD_RETRIES = 3;
    let rebuildRetries = 0;
    /**
     * A rebuild whose insert failed, kept here rather than pushed back into
     * the IngestBuffer: `markRebuild` would discard the points that streamed
     * in since, and would overwrite a newer replacement.
     */
    let pendingRebuild: T[] | null = null;

    // Cold/hot state
    let coldSnapshot: ChartDataMap | null = null;
    let coldTMax = -Infinity;
    let hotBuffer: ChartDataMap | null = null;
    let ticksSinceCold = 0;
    let coldRefreshNeeded = true; // start with a cold refresh

    // Change detection refs for cold refresh triggers
    let prevTimeRange: TimeRange = timeRangeRef.current;
    let prevMaxPoints: number = maxPointsRef.current;
    let prevSchema: TableSchema = schemaRef.current;

    const startPolling = async () => {
      try {
        await clearTable(conn, schemaRef.current);
        hasDataRef.current = false;
        setData(null);
      } catch (e) {
        console.warn("useTimeSeriesStore: failed to reset table:", e);
      }

      if (cancelled) return;

      const tick = async () => {
        if (cancelled) return;

        // 0. Check for rebuild signal. A newer replacement supersedes one
        //    that is waiting for a retry.
        const freshRebuild = ingestBufferRef.current.consumeRebuild();
        if (freshRebuild !== null) {
          pendingRebuild = null;
          rebuildRetries = 0;
        }
        const rebuildData = freshRebuild ?? pendingRebuild;
        if (rebuildData !== null) {
          pendingRebuild = null;
          try {
            // One transaction: a failure part-way through leaves the previous
            // content in place, so the re-queued retry cannot duplicate rows.
            await replacePoints(conn, schemaRef.current, rebuildData);
            hasDataRef.current = rebuildData.length > 0;
            if (!hasDataRef.current) setData(null); // clear chart for empty rebuild
            compactCooldown = COMPACT_COOLDOWN_AFTER_REBUILD;
            coldRefreshNeeded = true;
            hotBuffer = null;
            rebuildRetries = 0;
          } catch (e) {
            // Bounded, like the Worker path: re-queuing forever would retry a
            // permanently failing rebuild every tick, and each markRebuild
            // discards the points that streamed in since the last one.
            if (rebuildRetries < MAX_REBUILD_RETRIES) {
              rebuildRetries++;
              console.warn("useTimeSeriesStore: rebuild failed, retrying:", e);
              pendingRebuild = rebuildData;
            } else {
              rebuildRetries = 0;
              console.warn(
                "useTimeSeriesStore: dropping rebuild of",
                rebuildData.length,
                "points after",
                MAX_REBUILD_RETRIES,
                "retries:",
                e,
              );
              // The rolled-back transaction left the replaced dataset in the
              // table. Keeping it would splice it with the rows that keep
              // streaming in, so empty it.
              try {
                await clearTable(conn, schemaRef.current);
              } catch (clearError) {
                console.warn("useTimeSeriesStore: failed to empty the table:", clearError);
              }
              hasDataRef.current = false;
              coldRefreshNeeded = true;
              hotBuffer = null;
              setData(null);
            }
          }
        } else {
          // 1. Normal drain buffer → DuckDB insert
          const newPoints = ingestBufferRef.current.drain();
          if (newPoints.length > 0) {
            try {
              await insertPoints(conn, schemaRef.current, newPoints);
              hasDataRef.current = true;
            } catch (e) {
              console.warn(
                "useTimeSeriesStore: insert failed, re-queuing",
                newPoints.length,
                "points:",
                e,
              );
              ingestBufferRef.current.prependMany(newPoints);
            }
          }
        }

        // 2. Cold/hot query cycle
        if (hasDataRef.current) {
          ticksSinceCold++;

          // Detect option changes → trigger cold refresh
          const curTimeRange = timeRangeRef.current;
          const curMaxPoints = maxPointsRef.current;
          const curSchema = schemaRef.current;
          if (curTimeRange !== prevTimeRange) {
            coldRefreshNeeded = true;
            prevTimeRange = curTimeRange;
          }
          if (curMaxPoints !== prevMaxPoints) {
            coldRefreshNeeded = true;
            prevMaxPoints = curMaxPoints;
          }
          if (curSchema !== prevSchema) {
            coldRefreshNeeded = true;
            prevSchema = curSchema;
          }

          const needsCold =
            coldRefreshNeeded ||
            ticksSinceCold >= coldRefreshEveryN ||
            (hotBuffer != null && hotBuffer.t.length > hotRowBudget);

          const derivedNames = schemaRef.current.derived.map((d) => d.name);

          if (needsCold) {
            // COLD PATH: full downsampled query
            try {
              const tMin = computeTMin(curTimeRange, ingestBufferRef.current.latestT);
              coldSnapshot = await queryDerived(
                conn,
                schemaRef.current,
                tMin,
                maxPointsRef.current,
              );
              coldTMax =
                coldSnapshot.t.length > 0 ? coldSnapshot.t[coldSnapshot.t.length - 1] : -Infinity;
              hotBuffer = null;
              ticksSinceCold = 0;
              coldRefreshNeeded = false;

              // Compact check
              coldQueryCount++;
              if (compactCooldown > 0) {
                compactCooldown--;
              } else if (coldQueryCount % compactEveryN === 0) {
                const compacted = await compactTable(conn, schemaRef.current, compactOptions);
                if (compacted) coldRefreshNeeded = true;
              }
            } catch (e) {
              console.warn("useTimeSeriesStore: cold query/compact failed:", e);
            }
          } else {
            // HOT PATH: lightweight incremental query (no downsampling)
            try {
              const tMin = computeTMin(curTimeRange, ingestBufferRef.current.latestT);
              const hotLowerBound = tMin != null ? Math.max(coldTMax, tMin) : coldTMax;
              hotBuffer = await queryDerivedIncremental(conn, schemaRef.current, hotLowerBound);
            } catch (e) {
              console.warn("useTimeSeriesStore: hot query failed:", e);
            }
          }

          // Merge + trim for render
          if (coldSnapshot != null && !cancelled) {
            let merged = mergeChartData(coldSnapshot, hotBuffer, derivedNames);
            if (curTimeRange != null) {
              merged = trimChartDataLeft(
                merged,
                ingestBufferRef.current.latestT - curTimeRange,
                derivedNames,
              );
            }
            setData(merged);
          }
        }

        if (!cancelled) {
          queryTimerRef.current = window.setTimeout(tick, tickInterval) as unknown as number;
        }
      };

      queryTimerRef.current = window.setTimeout(tick, tickInterval) as unknown as number;
    };

    startPolling();

    return () => {
      cancelled = true;
      clearTimeout(queryTimerRef.current);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [conn]);

  return { data, isLoading };
}
