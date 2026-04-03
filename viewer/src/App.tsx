import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Scene } from "./components/Scene.js";
import { initKaname } from "./wasm/kanameInit.js";

// Start loading kaname WASM module immediately.
const kanameReady = initKaname();

import type { TimeRange } from "uneri";
import styles from "./App.module.css";
import { FrameSelector } from "./components/FrameSelector.js";
import { GraphPanel } from "./components/GraphPanel.js";
import { PlaybackBar } from "./components/PlaybackBar.js";
import { SimConfigModal } from "./components/SimConfigModal.js";
import { SimInfoBar } from "./components/SimInfoBar.js";
import { StatusBar } from "./components/StatusBar.js";
import { useFileSource } from "./hooks/useFileSource.js";
import { useRealtimePlayback } from "./hooks/useRealtimePlayback.js";
import { useSimulationData } from "./hooks/useSimulationData.js";
import { DEFAULT_FRAME, type ReferenceFrame } from "./referenceFrame.js";
import { useSourceRuntime } from "./sources/useSourceRuntime.js";
import { useWebSocketSource, WS_SOURCE_ID } from "./sources/useWebSocketSource.js";
import { readTimeRangeParam, writeTimeRangeParam } from "./utils/urlParams.js";

const DEFAULT_WS_URL: string =
  import.meta.env.VITE_WS_URL ??
  `${window.location.protocol === "https:" ? "wss:" : "ws:"}//${window.location.host}/ws`;

export function App() {
  // --- WASM initialization (must complete before rendering ECEF transforms) ---
  const [wasmReady, setWasmReady] = useState(false);
  useEffect(() => {
    kanameReady.then(() => setWasmReady(true));
  }, []);

  // --- Reference frame ---
  const [referenceFrame, setReferenceFrame] = useState<ReferenceFrame>(DEFAULT_FRAME);

  // --- Chart time range ---
  const [timeRange, setTimeRange] = useState<TimeRange>(() => readTimeRangeParam());

  // Sync timeRange to URL query parameter
  useEffect(() => {
    writeTimeRangeParam(timeRange);
  }, [timeRange]);

  // --- WS URL ---
  const [wsUrl, setWsUrl] = useState(DEFAULT_WS_URL);

  // --- SimConfig modal ---
  const [simConfigOpen, setSimConfigOpen] = useState(false);

  // --- Source Runtime (manages buffers, state, event dispatch) ---
  const runtime = useSourceRuntime();
  const {
    trailBuffers: trailBuffersMap,
    ingestBuffers: ingestBuffersMap,
    chartBuffer: runtimeChartBuffer,
    simInfo,
    serverState,
    terminatedSatellites,
    textureRevision,
    chartBufferVersion,
    handleEvent,
    setActiveSourceId,
    resetBuffers,
  } = runtime;

  // --- File source ---
  const fileSource = useFileSource({ handleEvent });

  // --- Realtime playback (history scrubbing) ---
  const realtimePlayback = useRealtimePlayback(trailBuffersMap, terminatedSatellites, timeRange);

  // Use ref for goLive to avoid including it in handleConnect deps.
  const goLiveRef = useRef(realtimePlayback.goLive);
  goLiveRef.current = realtimePlayback.goLive;

  // --- queryRange callback for useSimulationData fallback ---
  const sendRef = useRef<(msg: unknown) => void>(() => {});
  const queryRange = useCallback((satId: string, tMin: number, tMax: number, maxPoints: number) => {
    sendRef.current({
      type: "query_range",
      t_min: tMin,
      t_max: tMax,
      max_points: maxPoints,
      entity_path: satId,
    });
  }, []);

  // --- Simulation data (DuckDB + chart pipeline) ---
  const simData = useSimulationData({
    simInfo,
    ingestBuffers: ingestBuffersMap,
    chartBuffer: runtimeChartBuffer,
    chartBufferVersion,
    playback: {
      isLive: realtimePlayback.snapshot.isLive,
      currentTime: realtimePlayback.snapshot.currentTime,
    },
    timeRange,
    queryRange,
  });

  // --- WebSocket source ---
  const wsSource = useWebSocketSource({
    wsUrl,
    handleEvent,
    trailBuffers: trailBuffersMap,
    simInfo,
    latestRequestedRangeRef: simData.latestRequestedRangeRef,
  });

  // Keep sendRef in sync with wsSource.send
  sendRef.current = wsSource.send;

  // --- Coordinator: connect ---
  const manualDisconnectRef = useRef(false);

  const handleConnect = useCallback(() => {
    manualDisconnectRef.current = false;
    fileSource.stopRrdAdapter();
    fileSource.clearFileSourceActive();
    resetBuffers();
    setActiveSourceId(WS_SOURCE_ID);
    simData.resetZoomState();
    goLiveRef.current();
    wsSource.connect();
  }, [
    wsSource.connect,
    resetBuffers,
    setActiveSourceId,
    fileSource.stopRrdAdapter,
    fileSource.clearFileSourceActive,
    simData.resetZoomState,
  ]);

  const handleDisconnect = useCallback(() => {
    manualDisconnectRef.current = true;
    setSimConfigOpen(false);
    wsSource.disconnect();
  }, [wsSource.disconnect]);

  // --- Coordinator: file load ---
  // Source switching is deferred until the file is validated (CSV parsed / RRD ready)
  // via the onBeforeEmit callback to avoid destroying the session on invalid files
  // and to prevent the auto-connect race condition.
  const handleFileLoad = useCallback(
    (file: File) => {
      fileSource.loadFile(file, () => {
        // Called after validation succeeds — safe to switch sources.
        // Set manualDisconnectRef to suppress auto-connect until fileSourceActive
        // becomes true (set by useFileSource right after this callback returns).
        manualDisconnectRef.current = true;
        if (wsSource.isConnected) wsSource.disconnect();
        fileSource.stopRrdAdapter();
        resetBuffers();
        simData.resetZoomState();
        const sourceId = file.name.endsWith(".rrd") ? "rrd-file" : "csv-file";
        setActiveSourceId(sourceId);
        goLiveRef.current();
        // NOTE: manualDisconnectRef stays true here. It is cleared by handleConnect
        // when the user explicitly clicks Connect. Auto-connect is gated by
        // fileSourceActive (set true by useFileSource after this callback), so
        // it won't fire while a file source is active.
      });
    },
    [
      wsSource.isConnected,
      wsSource.disconnect,
      fileSource.stopRrdAdapter,
      fileSource.loadFile,
      resetBuffers,
      setActiveSourceId,
      simData.resetZoomState,
    ],
  );

  // --- Drag & Drop ---
  const [isDragOver, setIsDragOver] = useState(false);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(true);
  }, []);

  const handleDragLeave = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setIsDragOver(false);
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setIsDragOver(false);
      const file = e.dataTransfer.files[0];
      if (file) handleFileLoad(file);
    },
    [handleFileLoad],
  );

  // Wire file input change through the coordinator
  const handleFileChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;
      handleFileLoad(file);
      e.target.value = "";
    },
    [handleFileLoad],
  );

  // --- Auto-connect ---
  const handleConnectRef = useRef(handleConnect);
  handleConnectRef.current = handleConnect;
  const noAutoConnect = new URLSearchParams(window.location.search).has("noAutoConnect");

  useEffect(() => {
    if (
      !fileSource.fileSourceActive &&
      !wsSource.isConnected &&
      !manualDisconnectRef.current &&
      !noAutoConnect
    ) {
      handleConnectRef.current();
    }
  }, [fileSource.fileSourceActive, wsSource.isConnected, noAutoConnect]);

  // --- Derived values ---
  const textureBaseUrl = useMemo(() => {
    try {
      const u = new URL(wsUrl.replace(/^ws/, "http"));
      return `${u.origin}/textures/`;
    } catch {
      return `${import.meta.env.BASE_URL}textures/`;
    }
  }, [wsUrl]);

  const satelliteNames = useMemo(() => {
    if (!simInfo) return undefined;
    const m = new Map<string, string | null>();
    for (const sat of simInfo.satellites) m.set(sat.id, sat.name);
    return m;
  }, [simInfo]);

  const centralBody = simInfo?.central_body ?? "earth";
  const centralBodyRadius = simInfo?.central_body_radius ?? 6378.137;
  const epochJd = simInfo?.epoch_jd ?? undefined;

  // Total points across all satellite buffers.
  // chartBufferVersion bumps on data ingest AND on resetBuffers (clear),
  // so this recalculates when data arrives or buffers are cleared.
  // Note: trailBuffersMap is mutated in place, so we can't use it as a dep.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const totalPoints = useMemo(() => {
    let count = 0;
    for (const buf of trailBuffersMap.values()) count += buf.length;
    return count;
  }, [chartBufferVersion]);

  const showPlaybackBar = totalPoints > 0;

  // Union of active perturbation names across all satellites
  const activePerturbations = useMemo(() => {
    if (!simInfo) return [];
    const set = new Set<string>();
    for (const sat of simInfo.satellites) {
      for (const p of sat.perturbations) set.add(p);
    }
    return [...set];
  }, [simInfo]);

  // Auto-close SimConfig modal when leaving idle state or disconnecting
  useEffect(() => {
    if (serverState !== "idle" || !wsSource.isConnected) {
      setSimConfigOpen(false);
    }
  }, [serverState, wsSource.isConnected]);

  const handleOpenSimConfig = useCallback(() => {
    setSimConfigOpen(true);
  }, []);

  const handleCloseSimConfig = useCallback(() => {
    setSimConfigOpen(false);
  }, []);

  if (!wasmReady) return null;

  const showGraph = simData.dbReady;

  return (
    <div
      className={`app-root ${showGraph ? "" : "no-graph"}`}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {isDragOver && (
        <div className="drop-overlay">
          <div className="drop-overlay-text">Drop CSV file to load</div>
        </div>
      )}

      {/* Top status bar (row 1, spans all columns) — minimal */}
      <StatusBar
        isConnected={wsSource.isConnected}
        serverState={serverState}
        wsUrl={wsUrl}
        onWsUrlChange={setWsUrl}
        onConnect={handleConnect}
        onDisconnect={handleDisconnect}
        onPause={wsSource.handlePause}
        onResume={wsSource.handleResume}
        onTerminate={wsSource.handleTerminate}
        onLoadFileClick={fileSource.handleLoadClick}
        onOpenSimConfig={handleOpenSimConfig}
      />

      {/* 3D Scene (row 2, column 1) */}
      <div className="scene-container">
        {/* Scene overlay: frame selector + sim info (top-left of canvas) */}
        <div className={styles.sceneOverlay}>
          <FrameSelector
            referenceFrame={referenceFrame}
            onChange={setReferenceFrame}
            satellites={simInfo?.satellites}
            hasEpoch={epochJd != null}
            centralBody={centralBody}
          />
          {fileSource.orbitInfo && (
            <div className={styles.orbitInfo} data-testid="orbit-info-file">
              {fileSource.orbitInfo}
            </div>
          )}
          {simInfo && (
            <SimInfoBar
              simInfo={simInfo}
              totalPoints={totalPoints}
              epochJd={epochJd}
              activePerturbations={activePerturbations}
            />
          )}
        </div>

        <Scene
          trailBuffers={trailBuffersMap}
          satellitePositions={realtimePlayback.snapshot.satellitePositions}
          trailVisibleCounts={
            !realtimePlayback.snapshot.isLive
              ? realtimePlayback.snapshot.trailVisibleCounts
              : undefined
          }
          trailDrawStarts={
            timeRange != null ? realtimePlayback.snapshot.trailDrawStarts : undefined
          }
          centralBody={centralBody}
          centralBodyRadius={centralBodyRadius}
          epochJd={epochJd ?? null}
          referenceFrame={referenceFrame}
          satelliteNames={satelliteNames}
          physicalScale={false}
          textureRevision={textureRevision}
          textureBaseUrl={textureBaseUrl}
        />
      </div>

      {/* Graph panel (row 2, column 2) */}
      {showGraph && (
        <GraphPanel
          chartData={simData.isMultiSatellite ? undefined : simData.visibleChartData}
          multiChartData={simData.isMultiSatellite ? simData.multiChartData : undefined}
          isLoading={simData.chartsLoading}
          timeRange={timeRange}
          onTimeRangeChange={setTimeRange}
          onZoom={simData.handleChartZoom}
          activePerturbations={activePerturbations}
        />
      )}

      {/* Playback bar (row 3, spans all columns) */}
      {showPlaybackBar && (
        <PlaybackBar
          isPlaying={realtimePlayback.snapshot.isPlaying}
          fraction={realtimePlayback.snapshot.fraction}
          elapsedTime={realtimePlayback.snapshot.elapsedTime}
          totalDuration={realtimePlayback.snapshot.totalDuration}
          onTogglePlayPause={realtimePlayback.togglePlayPause}
          onSeekFraction={realtimePlayback.seekToFraction}
          onSpeedChange={realtimePlayback.setSpeed}
          isLive={realtimePlayback.snapshot.isLive}
          onGoLive={realtimePlayback.goLive}
          epochJd={epochJd}
        />
      )}

      {/* SimConfig modal (centered overlay) */}
      <SimConfigModal
        isOpen={simConfigOpen && wsSource.isConnected && serverState === "idle"}
        onStart={wsSource.handleStartSimulation}
        onClose={handleCloseSimConfig}
      />

      <input
        ref={fileSource.fileInputRef}
        type="file"
        accept=".csv,.txt,.rrd"
        style={{ display: "none" }}
        onChange={handleFileChange}
      />
    </div>
  );
}
