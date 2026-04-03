/**
 * useFileSource — encapsulates file loading logic (CSV / RRD) for the viewer.
 *
 * This hook handles only parsing and event emission. Coordination concerns
 * (resetBuffers, setActiveSourceId, WS disconnect, goLive) remain in the
 * App coordinator — the caller must handle source switching before calling
 * loadFile().
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { parseOrbitCSVWithMetadata } from "../orbit.js";
import { RrdFileAdapter } from "../sources/RrdFileAdapter.js";
import type { SourceEvent } from "../sources/types.js";

/** Source ID for CSV file sources. */
export const CSV_SOURCE_ID = "csv-file";

/** Source ID for RRD file sources. */
export const RRD_SOURCE_ID = "rrd-file";

interface UseFileSourceOptions {
  handleEvent: (sourceId: string, event: SourceEvent) => void;
}

interface FileSourceResult {
  fileInputRef: React.RefObject<HTMLInputElement | null>;
  orbitInfo: string;
  fileSourceActive: boolean;
  /**
   * Load a file. The optional `onBeforeEmit` callback is called after validation
   * succeeds (CSV parsed successfully / RRD ready) but before events are emitted.
   * The coordinator should do source switching (disconnect WS, reset buffers, etc.) there.
   */
  loadFile: (file: File, onBeforeEmit?: () => void) => void;
  handleLoadClick: () => void;
  handleFileChange: (e: React.ChangeEvent<HTMLInputElement>) => void;
  /** Stop any active RRD adapter. Called by coordinator during source switch. */
  stopRrdAdapter: () => void;
  /** Reset file source active flag (call when switching to WS source). */
  clearFileSourceActive: () => void;
}

export function useFileSource({ handleEvent }: UseFileSourceOptions): FileSourceResult {
  const [orbitInfo, setOrbitInfo] = useState<string>("");
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [fileSourceActive, setFileSourceActive] = useState(false);
  const rrdAdapterRef = useRef<RrdFileAdapter | null>(null);

  // Cleanup RRD adapter on unmount
  useEffect(() => {
    return () => {
      rrdAdapterRef.current?.stop();
    };
  }, []);

  const loadCSVFile = useCallback(
    (file: File, onBeforeEmit?: () => void) => {
      const reader = new FileReader();
      reader.onload = () => {
        const text = reader.result as string;
        const { points: parsed, metadata } = parseOrbitCSVWithMetadata(text);

        if (parsed.length === 0) {
          setOrbitInfo("No valid orbit data found in file.");
          return;
        }

        // Validation passed — let the coordinator switch sources now
        onBeforeEmit?.();

        // Build SimInfo from CSV metadata
        // For multi-sat, estimate dt from consecutive points of the same entity
        let dt = 10;
        if (metadata.satellites && metadata.satellites.length > 0) {
          // Multi-sat: find dt from same-entity consecutive points
          for (let i = 1; i < parsed.length; i++) {
            if (parsed[i].entityPath === parsed[0].entityPath && parsed[i].t > parsed[0].t) {
              dt = parsed[i].t - parsed[0].t;
              break;
            }
          }
        } else if (parsed.length >= 2) {
          dt = parsed[1].t - parsed[0].t;
        }

        // Build satellites list
        const satellites =
          metadata.satellites && metadata.satellites.length > 0
            ? metadata.satellites.map((id) => ({
                id,
                name: id,
                altitude: 0,
                period: 0,
                perturbations: [] as string[],
              }))
            : [
                {
                  id: "default",
                  name: metadata.satelliteName ?? `${file.name} (1 sat)`,
                  altitude: 0,
                  period: 0,
                  perturbations: [] as string[],
                },
              ];

        handleEvent(CSV_SOURCE_ID, {
          kind: "info",
          info: {
            mu: metadata.mu ?? 398600.4418,
            dt,
            output_interval: dt,
            stream_interval: dt,
            central_body: metadata.centralBody ?? "earth",
            central_body_radius: metadata.centralBodyRadius ?? 6378.137,
            epoch_jd: metadata.epochJd,
            satellites,
          },
        });

        // Push all CSV data as a history event, then mark complete.
        // NOTE: Do NOT dispatch server-state "idle" here — the dispatcher
        // clears simInfo on idle, which would erase the CSV metadata we just set.
        handleEvent(CSV_SOURCE_ID, { kind: "history", points: parsed });
        handleEvent(CSV_SOURCE_ID, { kind: "complete" });

        setFileSourceActive(true);

        const duration = parsed[parsed.length - 1].t - parsed[0].t;
        setOrbitInfo(
          `Loaded: ${file.name} | ${parsed.length} points | Duration: ${duration.toFixed(1)} s`,
        );
      };
      reader.readAsText(file);
    },
    [handleEvent],
  );

  const loadRrdFile = useCallback(
    (file: File, onBeforeEmit?: () => void) => {
      // RRD validation happens in the worker, so switch sources eagerly
      onBeforeEmit?.();
      let totalPoints = 0;
      const rrdHandleEvent: typeof handleEvent = (sourceId, event) => {
        handleEvent(sourceId, event);
        if (event.kind === "history-chunk") {
          totalPoints += event.points.length;
        }
        if (event.kind === "complete") {
          setOrbitInfo(`Loaded: ${file.name} | ${totalPoints} points`);
        }
      };

      const adapter = new RrdFileAdapter(RRD_SOURCE_ID, file, rrdHandleEvent);
      rrdAdapterRef.current = adapter;
      adapter.start();
      setFileSourceActive(true);
      setOrbitInfo(`Loading: ${file.name}...`);
    },
    [handleEvent],
  );

  const stopRrdAdapter = useCallback(() => {
    if (rrdAdapterRef.current) {
      rrdAdapterRef.current.stop();
      rrdAdapterRef.current = null;
    }
  }, []);

  /** Route file to appropriate loader based on extension. */
  const loadFile = useCallback(
    (file: File, onBeforeEmit?: () => void) => {
      if (file.name.endsWith(".rrd")) {
        loadRrdFile(file, onBeforeEmit);
      } else {
        loadCSVFile(file, onBeforeEmit);
      }
    },
    [loadCSVFile, loadRrdFile],
  );

  const handleLoadClick = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleFileChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;
      loadFile(file);
      e.target.value = "";
    },
    [loadFile],
  );

  const clearFileSourceActive = useCallback(() => {
    setFileSourceActive(false);
  }, []);

  return {
    fileInputRef,
    orbitInfo,
    fileSourceActive,
    loadFile,
    handleLoadClick,
    handleFileChange,
    stopRrdAdapter,
    clearFileSourceActive,
  };
}
