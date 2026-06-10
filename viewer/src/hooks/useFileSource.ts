/**
 * useFileSource — encapsulates file loading logic (CSV / RRD) for the viewer.
 *
 * This hook handles only parsing and event emission. Coordination concerns
 * (resetBuffers, setActiveSourceId, WS disconnect, goLive) remain in the
 * App coordinator — the caller must handle source switching before calling
 * loadFile().
 *
 * Both formats parse off the main thread via a SourceAdapter (Web Worker),
 * so large files don't block the UI.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { CSVFileAdapter } from "../sources/CSVFileAdapter.js";
import { RrdFileAdapter } from "../sources/RrdFileAdapter.js";
import type { SourceAdapter, SourceEvent } from "../sources/types.js";

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
   * succeeds (CSV produced data points / RRD ready) but before events are emitted.
   * The coordinator should do source switching (disconnect WS, reset buffers, etc.) there.
   */
  loadFile: (file: File, onBeforeEmit?: () => void) => void;
  handleLoadClick: () => void;
  /** Stop any active file adapter. Called by coordinator during source switch. */
  stopFileAdapter: () => void;
  /** Reset file source active flag (call when switching to WS source). */
  clearFileSourceActive: () => void;
}

export function useFileSource({ handleEvent }: UseFileSourceOptions): FileSourceResult {
  const [orbitInfo, setOrbitInfo] = useState<string>("");
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [fileSourceActive, setFileSourceActive] = useState(false);
  const fileAdapterRef = useRef<SourceAdapter | null>(null);

  const stopFileAdapter = useCallback(() => {
    if (fileAdapterRef.current) {
      fileAdapterRef.current.stop();
      fileAdapterRef.current = null;
    }
  }, []);

  // Cleanup adapter (abort reader / terminate worker) on unmount
  useEffect(() => stopFileAdapter, [stopFileAdapter]);

  const loadCSVFile = useCallback(
    (file: File, onBeforeEmit?: () => void) => {
      // The CSV worker always produces metadata — even for junk input — so
      // an "info" event alone does not prove the file is valid. Gate source
      // switching on the first non-empty history-chunk instead: buffer the
      // info event, and only when actual data points arrive fire
      // onBeforeEmit, flush the buffered info, and start forwarding.
      // A file with zero valid rows therefore never disturbs the current
      // source (matching the old synchronous-validation behavior).
      let pendingInfo: SourceEvent | null = null;
      let gateOpened = false;
      let pointCount = 0;
      let tMin = Number.POSITIVE_INFINITY;
      let tMax = Number.NEGATIVE_INFINITY;

      const wrapped: typeof handleEvent = (sourceId, event) => {
        // Ignore events from an adapter that has been superseded (a newer
        // load or a Connect stopped it); its worker may still flush messages.
        if (fileAdapterRef.current !== adapter) return;

        switch (event.kind) {
          case "info":
            pendingInfo = event;
            return;
          case "history-chunk": {
            if (!gateOpened && event.points.length > 0) {
              gateOpened = true;
              onBeforeEmit?.();
              if (pendingInfo) handleEvent(sourceId, pendingInfo);
              setFileSourceActive(true);
            }
            for (const p of event.points) {
              pointCount++;
              if (p.t < tMin) tMin = p.t;
              if (p.t > tMax) tMax = p.t;
            }
            if (gateOpened) handleEvent(sourceId, event);
            return;
          }
          case "complete":
            if (!gateOpened) {
              setOrbitInfo("No valid orbit data found in file.");
              fileAdapterRef.current = null;
              return;
            }
            handleEvent(sourceId, event);
            setOrbitInfo(
              `Loaded: ${file.name} | ${pointCount} points | Duration: ${(tMax - tMin).toFixed(1)} s`,
            );
            return;
          case "error":
            if (gateOpened) handleEvent(sourceId, event);
            setOrbitInfo(`Failed to load ${file.name}: ${event.message}`);
            return;
          default:
            if (gateOpened) handleEvent(sourceId, event);
        }
      };

      const adapter = new CSVFileAdapter(CSV_SOURCE_ID, file, wrapped);
      fileAdapterRef.current = adapter;
      setOrbitInfo(`Loading: ${file.name}...`);
      adapter.start();
    },
    [handleEvent],
  );

  const loadRrdFile = useCallback(
    (file: File, onBeforeEmit?: () => void) => {
      // RRD validation happens in the worker, so switch sources eagerly
      onBeforeEmit?.();
      let totalPoints = 0;
      const wrapped: typeof handleEvent = (sourceId, event) => {
        if (fileAdapterRef.current !== adapter) return;
        handleEvent(sourceId, event);
        if (event.kind === "history-chunk") {
          totalPoints += event.points.length;
        }
        if (event.kind === "complete") {
          setOrbitInfo(`Loaded: ${file.name} | ${totalPoints} points`);
        }
      };

      const adapter = new RrdFileAdapter(RRD_SOURCE_ID, file, wrapped);
      fileAdapterRef.current = adapter;
      adapter.start();
      setFileSourceActive(true);
      setOrbitInfo(`Loading: ${file.name}...`);
    },
    [handleEvent],
  );

  /** Route file to appropriate loader based on extension. */
  const loadFile = useCallback(
    (file: File, onBeforeEmit?: () => void) => {
      // Stop any in-flight load first so two adapters never stream into
      // the same buffers concurrently.
      stopFileAdapter();
      if (file.name.endsWith(".rrd")) {
        loadRrdFile(file, onBeforeEmit);
      } else {
        loadCSVFile(file, onBeforeEmit);
      }
    },
    [loadCSVFile, loadRrdFile, stopFileAdapter],
  );

  const handleLoadClick = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const clearFileSourceActive = useCallback(() => {
    setFileSourceActive(false);
  }, []);

  return {
    fileInputRef,
    orbitInfo,
    fileSourceActive,
    loadFile,
    handleLoadClick,
    stopFileAdapter,
    clearFileSourceActive,
  };
}
