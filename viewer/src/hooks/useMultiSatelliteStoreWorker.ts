/**
 * Worker-based replacement for useMultiSatelliteStore.
 *
 * Moves per-satellite DuckDB management, queries, alignment, and
 * buildMultiChartData entirely to a Web Worker.
 */

import { useEffect, useRef, useState } from "react";
import type { IngestBuffer, TableSchema, TimePoint, TimeRange } from "uneri";
import type { MultiChartDataResult, MultiChartDataWorkerClient } from "uneri/multiWorkerClient";
import type { WorkerSatelliteConfig, WorkerTableSchema } from "uneri/workerProtocol";
import type { MultiChartDataMap, SatelliteConfig } from "./buildMultiChartData.js";

export interface UseMultiSatelliteStoreWorkerOptions<T extends TimePoint> {
  baseSchema: TableSchema<T>;
  satelliteConfigs: SatelliteConfig[];
  ingestBuffers: Map<string, IngestBuffer<T>>;
  metricNames: string[];
  timeRange?: TimeRange;
  maxPoints?: number;
  drainInterval?: number;
  enabled?: boolean;
  /**
   * Optional caller-supplied ref that the hook populates with the live
   * worker client once the dynamic import finishes. Mirrors the
   * single-sat `useTimeSeriesStoreWorker`'s `clientRef` pattern so the
   * consumer can read `.current` from an earlier-declared handler
   * (e.g. `handleChartZoom`) and fire ad-hoc `zoomQuery` requests.
   */
  clientRef?: React.RefObject<MultiChartDataWorkerClient | null>;
}

export interface UseMultiSatelliteStoreWorkerReturn {
  data: MultiChartDataMap | null;
  isLoading: boolean;
}

function toWorkerSchema(schema: TableSchema): WorkerTableSchema {
  return {
    tableName: schema.tableName,
    columns: schema.columns,
    derived: schema.derived,
  };
}

function toWorkerConfigs(configs: SatelliteConfig[]): WorkerSatelliteConfig[] {
  return configs.map((c) => ({ id: c.id, label: c.label, color: c.color }));
}

/** Convert Worker result to viewer MultiChartDataMap. */
function toMultiChartDataMap(result: MultiChartDataResult): MultiChartDataMap {
  const map: MultiChartDataMap = {};
  for (const [metricName, data] of Object.entries(result)) {
    if (!data) {
      map[metricName] = null;
      continue;
    }
    map[metricName] = {
      t: data.t,
      values: data.values,
      series: data.series,
    };
  }
  return map;
}

export function useMultiSatelliteStoreWorker<T extends TimePoint>(
  options: UseMultiSatelliteStoreWorkerOptions<T>,
): UseMultiSatelliteStoreWorkerReturn {
  const {
    baseSchema,
    satelliteConfigs,
    ingestBuffers,
    metricNames,
    timeRange = null,
    maxPoints = 2000,
    drainInterval = 500,
    enabled = true,
    clientRef: externalClientRef,
  } = options;

  const [data, setData] = useState<MultiChartDataMap | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  const baseSchemaRef = useRef(baseSchema);
  baseSchemaRef.current = baseSchema;
  const configsRef = useRef(satelliteConfigs);
  configsRef.current = satelliteConfigs;
  const buffersRef = useRef(ingestBuffers);
  buffersRef.current = ingestBuffers;
  const metricNamesRef = useRef(metricNames);
  metricNamesRef.current = metricNames;
  const timeRangeRef = useRef(timeRange);
  timeRangeRef.current = timeRange;
  const maxPointsRef = useRef(maxPoints);
  maxPointsRef.current = maxPoints;
  const enabledRef = useRef(enabled);
  enabledRef.current = enabled;

  const prevTimeRange = useRef(timeRange);
  const prevMaxPoints = useRef(maxPoints);
  const prevConfigIds = useRef(satelliteConfigs.map((c) => c.id).join(","));

  const clientRef = useRef<MultiChartDataWorkerClient | null>(null);

  useEffect(() => {
    if (!enabledRef.current) return;

    // Dynamic import to avoid bundling the Worker when not used
    let cancelled = false;
    let drainTimer = 0;

    (async () => {
      let ClientClass: typeof MultiChartDataWorkerClient;
      try {
        ({ MultiChartDataWorkerClient: ClientClass } = await import("uneri/multiWorkerClient"));
      } catch (e) {
        console.error("useMultiSatelliteStoreWorker: failed to load Worker module:", e);
        return;
      }
      if (cancelled) return;

      const client = new ClientClass();
      clientRef.current = client;
      if (externalClientRef) {
        externalClientRef.current = client;
      }

      client.onData((result) => {
        setData(toMultiChartDataMap(result));
        setIsLoading(false);
      });

      client.onError((message) => {
        console.warn("useMultiSatelliteStoreWorker: Worker error:", message);
      });

      client.init(
        toWorkerSchema(baseSchemaRef.current),
        toWorkerConfigs(configsRef.current),
        metricNamesRef.current,
      );

      client.configure(timeRangeRef.current, maxPointsRef.current);

      const drain = () => {
        if (cancelled) return;

        // Drain each satellite's IngestBuffer
        for (const [satId, buf] of buffersRef.current.entries()) {
          const rebuildData = buf.consumeRebuild();
          if (rebuildData !== null) {
            const rows = rebuildData.map((p) => baseSchemaRef.current.toRow(p));
            client.rebuild(satId, rows, buf.latestT);
          } else {
            const points = buf.drain();
            if (points.length > 0) {
              const rows = points.map((p) => baseSchemaRef.current.toRow(p));
              client.ingest(satId, rows, buf.latestT);
            }
          }
        }

        // Config changes
        if (
          timeRangeRef.current !== prevTimeRange.current ||
          maxPointsRef.current !== prevMaxPoints.current
        ) {
          client.configure(timeRangeRef.current, maxPointsRef.current);
          prevTimeRange.current = timeRangeRef.current;
          prevMaxPoints.current = maxPointsRef.current;
        }

        // Satellite config changes (added/removed/replaced)
        const currentConfigIds = configsRef.current.map((c) => c.id).join(",");
        if (currentConfigIds !== prevConfigIds.current) {
          client.updateConfigs(toWorkerConfigs(configsRef.current), metricNamesRef.current);
          prevConfigIds.current = currentConfigIds;
        }

        drainTimer = window.setTimeout(drain, drainInterval) as unknown as number;
      };

      drainTimer = window.setTimeout(drain, drainInterval) as unknown as number;
    })();

    return () => {
      cancelled = true;
      clearTimeout(drainTimer);
      clientRef.current?.dispose();
      clientRef.current = null;
      if (externalClientRef) {
        externalClientRef.current = null;
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { data, isLoading };
}
