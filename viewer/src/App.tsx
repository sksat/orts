import { Canvas } from "@react-three/fiber";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { initArika } from "./wasm/arikaInit.js";

// Start loading arika WASM module immediately.
const arikaReady = initArika();

import type { TimeRange } from "@sksat/uneri";
import appStyles from "./App.module.css";
import { AttitudeOverlay } from "./components/AttitudeOverlay.js";
import { GraphPanel } from "./components/GraphPanel.js";
import { InitialCameraFit } from "./components/InitialCameraFit.js";
import { OrbitOverlay } from "./components/OrbitOverlay.js";
import { PlaybackBar } from "./components/PlaybackBar.js";
import { SimConfigModal } from "./components/SimConfigModal.js";
import { StatusBar } from "./components/StatusBar.js";
import { type ViewMode, ViewSelector } from "./components/ViewSelector.js";
import { type DirectionVectorKind, resolveDirectionVectors } from "./directionVectors.js";
import type { DisplayFrame, Vec3 as DisplayVec3 } from "./displayFrame.js";
import { toViewerReferenceFrame } from "./frameToViewer.js";
import { CSV_SOURCE_ID, RRD_SOURCE_ID, useFileSource } from "./hooks/useFileSource.js";
import { useRealtimePlayback } from "./hooks/useRealtimePlayback.js";
import { useSimInfoDerived } from "./hooks/useSimInfoDerived.js";
import { useSimulationData } from "./hooks/useSimulationData.js";
import {
  type AttitudeBodyState,
  type AttitudeFrame,
  AttitudeScene,
  type DirectionVectorOptions,
  OrbitScene,
  type SatelliteState,
} from "./lib/index.js";
import type { ClientMessage } from "./protocol/generated/ClientMessage.js";
import { DEFAULT_FRAME, type ReferenceFrame } from "./referenceFrame.js";
import { type MarkerShape, readSatShapeParam, writeSatShapeParam } from "./satelliteShapes.js";
import { computeLvlhAxes, DEFAULT_CAMERA_POSITION, SCENE_UP } from "./sceneFrame.js";
import { useSourceRuntime } from "./sources/useSourceRuntime.js";
import { useWebSocketSource, WS_SOURCE_ID } from "./sources/useWebSocketSource.js";
import { resolveTextureBaseUrl } from "./textureBaseUrl.js";
import { resolveDefaultWsUrl } from "./utils/defaultWsUrl.js";
import { planInitialRangeQuery } from "./utils/initialRangeQuery.js";
import {
  readTimeRangeParam,
  readViewParam,
  writeTimeRangeParam,
  writeViewParam,
} from "./utils/urlParams.js";

const DEFAULT_WS_URL: string = resolveDefaultWsUrl({
  explicitWsUrl: import.meta.env.VITE_WS_URL,
  baseUrl: import.meta.env.BASE_URL,
  protocol: window.location.protocol,
  host: window.location.host,
});

// Build-time texture base URL — present when VITE_TEXTURE_BASE_URL is set at Vite startup.
const VITE_TEXTURE_BASE_URL = import.meta.env.VITE_TEXTURE_BASE_URL;

/**
 * Frame the legend asks the resolver with. Which arrows resolve depends on their
 * inputs, not on the frame, so any frame answers the question.
 */
const LEGEND_FRAME: DisplayFrame = { kind: "inertial", origin: null };

/** Stands in for "the scene will have a Sun direction", which it computes itself. */
const SUN_PRESENT: DisplayVec3 = [1, 0, 0];

/** Both reference-direction arrows on, the starting state for either view. */
const DEFAULT_DIRECTION_VECTORS: DirectionVectorOptions = { sun: true, nadir: true };

/**
 * Camera framing for the attitude view, whose scene unit is the spacecraft.
 * `InitialCameraFit` pulls it further back on a viewport narrower than square.
 */
const ATTITUDE_FOV = 50;
const ATTITUDE_CAMERA_POSITION: [number, number, number] = [4.3, 0, 2.15];

export function App() {
  // WASM initialization (must complete before rendering ECEF transforms)
  const [wasmReady, setWasmReady] = useState(false);
  useEffect(() => {
    arikaReady.then(() => setWasmReady(true));
  }, []);

  const [referenceFrame, setReferenceFrame] = useState<ReferenceFrame>(DEFAULT_FRAME);

  // Which presentation is showing, persisted to the URL. Each view keeps its own
  // display state: the orbit view's frame pairs a centre with an orientation, the
  // attitude view has only an orientation, and mapping one onto the other on every
  // switch would quietly change what the reader is looking at.
  const [view, setView] = useState<ViewMode>(() => readViewParam());
  const [attitudeFrame, setAttitudeFrame] = useState<AttitudeFrame>("inertial");
  const [selectedSatelliteId, setSelectedSatelliteId] = useState<string | null>(null);
  const [directionVectors, setDirectionVectors] =
    useState<DirectionVectorOptions>(DEFAULT_DIRECTION_VECTORS);
  useEffect(() => {
    writeViewParam(view);
  }, [view]);

  // Chart time range
  const [timeRange, setTimeRange] = useState<TimeRange>(() => readTimeRangeParam());

  // Sync timeRange to URL query parameter
  useEffect(() => {
    writeTimeRangeParam(timeRange);
  }, [timeRange]);

  const [wsUrl, setWsUrl] = useState(DEFAULT_WS_URL);

  // Satellite marker shape: global default (persisted to URL) + per-satellite overrides.
  const [defaultMarkerShape, setDefaultMarkerShape] = useState<MarkerShape | null>(() =>
    readSatShapeParam(),
  );
  useEffect(() => {
    writeSatShapeParam(defaultMarkerShape);
  }, [defaultMarkerShape]);
  const [markerShapeOverrides, setMarkerShapeOverrides] = useState<Map<string, MarkerShape>>(
    () => new Map(),
  );
  const handleMarkerShapeOverride = useCallback((satId: string, shape: MarkerShape | null) => {
    setMarkerShapeOverrides((prev) => {
      const next = new Map(prev);
      if (shape == null) next.delete(satId);
      else next.set(satId, shape);
      return next;
    });
  }, []);

  const [simConfigOpen, setSimConfigOpen] = useState(false);

  // Source Runtime (manages buffers, state, event dispatch)
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

  // File source
  const fileSource = useFileSource({ handleEvent });

  // Realtime playback (history scrubbing)
  const realtimePlayback = useRealtimePlayback(trailBuffersMap, terminatedSatellites, timeRange);

  // Use ref for goLive to avoid including it in handleConnect deps.
  const goLiveRef = useRef(realtimePlayback.goLive);
  goLiveRef.current = realtimePlayback.goLive;

  // queryRange callback for useSimulationData fallback
  const sendRef = useRef<(msg: ClientMessage) => void>(() => {});
  const queryRange = useCallback((satId: string, tMin: number, tMax: number, maxPoints: number) => {
    sendRef.current({
      type: "query_range",
      t_min: tMin,
      t_max: tMax,
      max_points: maxPoints,
      entity_path: satId,
    });
  }, []);

  // Simulation data (DuckDB + chart pipeline)
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

  const wsSource = useWebSocketSource({
    wsUrl,
    handleEvent,
    trailBuffers: trailBuffersMap,
    simInfo,
    latestRequestedRangeRef: simData.latestRequestedRangeRef,
  });

  // Keep sendRef in sync with wsSource.send
  sendRef.current = wsSource.send;

  // Proactive initial range query
  //
  // The server's connect-time history is a bounded, sparse overview of the
  // entire simulation. For a client with a finite `timeRange` selected
  // (e.g. "last hour"), that overview is too sparse to render a smooth
  // chart. We compensate by pulling a higher-resolution slice of the
  // current display window via the existing `query_range` path, once per
  // WebSocket connection.
  //
  // Keyed off of `simInfo` identity: each new Info message from a fresh
  // connection is a distinct object, so this fires exactly once per
  // connect without needing manual reset logic.
  const firedInitialQueryForSimInfoRef = useRef<typeof simInfo>(null);
  // `trailBuffersMap` is mutated in place (it is a ref-held `Map`) so it
  // cannot be a real dep; `chartBufferVersion` is the observational
  // trigger bumped by the event dispatcher when new data arrives, and is
  // what actually makes this effect re-run after history lands.
  // biome-ignore lint/correctness/useExhaustiveDependencies: chartBufferVersion is the observational trigger; trailBuffersMap is read via a stable ref.
  useEffect(() => {
    if (!simInfo) {
      firedInitialQueryForSimInfoRef.current = null;
      return;
    }
    if (firedInitialQueryForSimInfoRef.current === simInfo) return;

    // Anchor the window on the most recent point we currently hold.
    let latestT = 0;
    for (const buf of trailBuffersMap.values()) {
      if (buf.length > 0) {
        const last = buf.getAll()[buf.length - 1];
        if (last && last.t > latestT) latestT = last.t;
      }
    }

    const plans = planInitialRangeQuery({
      simInfo,
      timeRange,
      latestT,
      alreadyQueried: false,
    });
    if (plans.length > 0) {
      // Tell the query_range staleness check that this window is what we
      // are currently asking for. Every plan in a multi-sat batch shares
      // the same (tMin, tMax) window, so a single update is enough. If
      // the user zooms before all responses arrive, `handleChartZoom`
      // will overwrite this with the zoom range and the in-flight
      // proactive responses are correctly dropped as stale.
      simData.latestRequestedRangeRef.current = {
        tMin: plans[0].tMin,
        tMax: plans[0].tMax,
      };
      for (const plan of plans) {
        queryRange(plan.satId, plan.tMin, plan.tMax, plan.maxPoints);
      }
      firedInitialQueryForSimInfoRef.current = simInfo;
    }
  }, [simInfo, timeRange, chartBufferVersion, queryRange, simData.latestRequestedRangeRef]);

  // Coordinator: connect
  const manualDisconnectRef = useRef(false);

  const handleConnect = useCallback(() => {
    manualDisconnectRef.current = false;
    fileSource.stopFileAdapter();
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
    fileSource.stopFileAdapter,
    fileSource.clearFileSourceActive,
    simData.resetZoomState,
  ]);

  const handleDisconnect = useCallback(() => {
    manualDisconnectRef.current = true;
    setSimConfigOpen(false);
    wsSource.disconnect();
  }, [wsSource.disconnect]);

  // Coordinator: file load
  // Source switching is deferred until the file is validated (CSV parsed / RRD ready)
  // via the onBeforeEmit callback to avoid destroying the session on invalid files
  // and to prevent the auto-connect race condition.
  const handleFileLoad = useCallback(
    (file: File) => {
      // Any previous in-flight file adapter is stopped inside loadFile,
      // before the new load starts.
      fileSource.loadFile(file, () => {
        // Called after validation succeeds — safe to switch sources.
        // Set manualDisconnectRef to suppress auto-connect until fileSourceActive
        // becomes true (set by useFileSource right after this callback returns).
        manualDisconnectRef.current = true;
        if (wsSource.isConnected) wsSource.disconnect();
        resetBuffers();
        simData.resetZoomState();
        setActiveSourceId(file.name.endsWith(".rrd") ? RRD_SOURCE_ID : CSV_SOURCE_ID);
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
      fileSource.loadFile,
      resetBuffers,
      setActiveSourceId,
      simData.resetZoomState,
    ],
  );

  // Drag & Drop
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

  // Auto-connect
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

  // Derived values
  const textureBaseUrl = useMemo(
    () => resolveTextureBaseUrl(wsSource.isConnected, wsUrl, VITE_TEXTURE_BASE_URL),
    [wsSource.isConnected, wsUrl, VITE_TEXTURE_BASE_URL],
  );

  useEffect(() => {
    if (!import.meta.env.DEV) return;
    const w = window as unknown as Record<string, unknown>;
    w.__debug_texture_base_url = textureBaseUrl;
    return () => {
      delete w.__debug_texture_base_url;
    };
  }, [textureBaseUrl]);

  // Values derived from the simulation metadata (with absent-simInfo defaults).
  const { centralBody, centralBodyRadius, epochJd, satelliteNames, activePerturbations } =
    useSimInfoDerived(simInfo);

  // Sim-declared marker shapes (from SatelliteInfo); the viewer can override these.
  const satelliteSimShapes = useMemo(() => {
    if (!simInfo) return undefined;
    const m = new Map<string, MarkerShape>();
    for (const sat of simInfo.satellites) {
      if (sat.shape) m.set(sat.id, sat.shape);
    }
    return m;
  }, [simInfo]);

  // Public display frame for <OrbitScene> (the app's UI state is the internal frame).
  const viewerFrame = useMemo(() => toViewerReferenceFrame(referenceFrame), [referenceFrame]);

  // Build the public SatelliteState[] for <OrbitScene>. Each satellite passes its
  // persistent TrailBuffer straight through (streaming mode), so trail growth stays
  // decoupled from React re-renders — no per-render point materialization.
  const snapshot = realtimePlayback.snapshot;
  // biome-ignore lint/correctness/useExhaustiveDependencies: trailBuffersMap is a stable ref-held Map mutated in place; `snapshot` triggers rebuilds on position/playback changes, and `chartBufferVersion` (bumped on ingest AND on resetBuffers) triggers rebuilds when the map is cleared — without it a reset would leave stale satellites referencing detached buffers, since an empty-buffer reset publishes no new snapshot.
  const satellites = useMemo<SatelliteState[]>(() => {
    const list: SatelliteState[] = [];
    for (const [id, buf] of trailBuffersMap) {
      const pos = snapshot.satellitePositions.get(id);
      // No current position means an empty buffer (positions are interpolated from
      // the buffers) — nothing to render, so skip it.
      if (!pos) continue;
      const visibleCount = snapshot.isLive ? undefined : snapshot.trailVisibleCounts.get(id);
      const drawStart = timeRange != null ? snapshot.trailDrawStarts.get(id) : undefined;
      const trailDisplay =
        visibleCount != null || drawStart != null ? { visibleCount, drawStart } : undefined;
      list.push({
        id,
        position: [pos.x, pos.y, pos.z],
        velocity: [pos.vx, pos.vy, pos.vz],
        // The interpolated point's own time — clamped for terminated/out-of-span
        // satellites — so its body-fixed marker transform uses the right epoch.
        time: pos.t,
        // All four components or none, matching `hasQuaternion` in `orbit.ts`.
        //
        // A complete tuple passes through whatever it holds, zero and non-finite
        // included, and the scene reads the refusal back off it — so an unusable
        // attitude does reach the scene as one. What cannot arrive is a *partly*
        // decoded tuple, and no source produces one: the rrd decoder yields four
        // components or none, and the WS payload carries the tuple as one value.
        attitude:
          pos.qw != null && pos.qx != null && pos.qy != null && pos.qz != null
            ? [pos.qw, pos.qx, pos.qy, pos.qz]
            : undefined,
        name: satelliteNames?.get(id) ?? undefined,
        markerShape: markerShapeOverrides.get(id) ?? satelliteSimShapes?.get(id),
        trailBuffer: buf,
        trailDisplay,
      });
    }
    return list;
  }, [
    snapshot,
    chartBufferVersion,
    satelliteNames,
    markerShapeOverrides,
    satelliteSimShapes,
    timeRange,
  ]);

  /** The orbit view's centred satellite, when it is one the viewer still has. */
  const centredSatellite = useMemo(() => {
    if (referenceFrame.center.type !== "satellite") return null;
    const id = referenceFrame.center.id;
    return satellites.find((s) => s.id === id) ?? null;
  }, [referenceFrame, satellites]);
  const centredSatelliteId = centredSatellite?.id ?? null;

  // Keep the attitude view's subject on a spacecraft the viewer still has. A
  // satellite-centred orbit view hands over its centre; otherwise the previous
  // choice stands while it is still in the list, and the first spacecraft takes
  // over when it is not — which happens when the source changes, not when a
  // satellite terminates: a terminated one keeps its buffers and its last state,
  // and its final attitude is worth looking at. The centre is only a candidate
  // while it is in the list too, since a frame can name a satellite the current
  // source never had.
  useEffect(() => {
    if (selectedSatelliteId != null && satellites.some((s) => s.id === selectedSatelliteId)) return;
    const fallback = centredSatelliteId ?? satellites[0]?.id ?? null;
    if (fallback !== selectedSatelliteId) setSelectedSatelliteId(fallback);
  }, [satellites, centredSatelliteId, selectedSatelliteId]);

  const handleViewChange = useCallback(
    (next: ViewMode) => {
      // Switching from a satellite-centred orbit view carries that satellite over:
      // it is the one the reader was already looking at.
      if (next === "attitude" && centredSatelliteId != null) {
        setSelectedSatelliteId(centredSatelliteId);
      }
      setView(next);
    },
    [centredSatelliteId],
  );

  /**
   * The spacecraft the attitude view can offer: the ones being rendered, which is
   * the same list the selection is validated against. The simulation's declared
   * list arrives with the `info` message, before any state does, and offering a
   * spacecraft from it would leave the select with no matching option until the
   * first sample landed.
   */
  const attitudeSubjects = useMemo(
    () => satellites.map((s) => ({ id: s.id, name: s.name })),
    [satellites],
  );

  /**
   * The attitude view's subject. Null when the chosen spacecraft has no attitude:
   * this view has nothing to show then, and inventing an identity quaternion would
   * present "no data" as "pointing at the reference frame".
   */
  const attitudeBody = useMemo<AttitudeBodyState | null>(() => {
    const sat = satellites.find((s) => s.id === selectedSatelliteId);
    if (sat?.attitude == null) return null;
    return {
      id: sat.id,
      attitude: sat.attitude,
      position: sat.position,
      velocity: sat.velocity,
      time: sat.time,
      name: sat.name,
      color: sat.color,
      markerShape: sat.markerShape,
    };
  }, [satellites, selectedSatelliteId]);

  /**
   * Which arrows a scene would draw for a spacecraft at this position — asked of
   * the resolver the scenes use, not re-derived from "is the input present". A
   * position can be there and still not yield a direction (zero, or non-finite
   * from a file source), and a control that offers an arrow the scene then drops
   * is lying.
   *
   * The kinds a resolver returns do not depend on the display frame, so the
   * inertial one stands in; the Sun's own direction is computed inside the scene,
   * so an epoch is all the app can know about it.
   */
  const resolvedVectorKinds = useCallback(
    (
      position: DisplayVec3 | null | undefined,
      options: DirectionVectorOptions,
    ): readonly DirectionVectorKind[] =>
      resolveDirectionVectors({
        frame: LEGEND_FRAME,
        sunEci: epochJd != null ? SUN_PRESENT : null,
        positionEci: position ?? null,
        options,
      }).map((v) => v.kind),
    [epochJd],
  );

  /** What the attitude view draws right now — the legend names exactly these. */
  const attitudeVectorKinds = useMemo<readonly DirectionVectorKind[]>(
    () =>
      attitudeBody == null ? [] : resolvedVectorKinds(attitudeBody.position, directionVectors),
    [resolvedVectorKinds, attitudeBody, directionVectors],
  );

  /**
   * What each view *could* draw, which is the separate question a disabled
   * control answers: a direction switched off is not an unavailable one.
   */
  const attitudeDrawableKinds = useMemo<readonly DirectionVectorKind[]>(
    () =>
      attitudeBody == null
        ? []
        : resolvedVectorKinds(attitudeBody.position, DEFAULT_DIRECTION_VECTORS),
    [resolvedVectorKinds, attitudeBody],
  );

  const orbitDrawableKinds = useMemo<readonly DirectionVectorKind[]>(
    () =>
      centredSatellite == null
        ? []
        : resolvedVectorKinds(centredSatellite.position, DEFAULT_DIRECTION_VECTORS),
    [resolvedVectorKinds, centredSatellite],
  );

  /**
   * The display orientation the attitude view actually renders in.
   *
   * A request whose inputs are absent falls back to inertial in the scene, so the
   * selection is normalised here rather than left showing "LVLH" over inertial
   * axes — the data behind a choice can disappear after it was made (a satellite
   * terminates, a source without velocities takes over).
   *
   * Local-orbital asks `computeLvlhAxes`, the same function the scene resolves
   * with: a position and a velocity can both be present and still not span an
   * orbit frame (parallel, zero, non-finite).
   */
  const attitudeFrameAvailable = useCallback(
    (frame: AttitudeFrame) => {
      if (frame === "localOrbital") {
        return (
          computeLvlhAxes(attitudeBody?.position ?? null, attitudeBody?.velocity ?? null) != null
        );
      }
      if (frame === "bodyFixed") return epochJd != null && centralBody === "earth";
      return true;
    },
    [attitudeBody, epochJd, centralBody],
  );
  useEffect(() => {
    if (!attitudeFrameAvailable(attitudeFrame)) setAttitudeFrame("inertial");
  }, [attitudeFrame, attitudeFrameAvailable]);

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

  // The charts are orbital, so the attitude view hides them — and the layout
  // collapses to a single column with them.
  const showGraph = simData.dbReady && view === "orbit";

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
        {/* The app owns the overlay area: the view switch has to outlive both
            views, and each view contributes only its own controls. */}
        <div className={appStyles.sceneOverlay}>
          <ViewSelector view={view} onChange={handleViewChange} />
          {view === "orbit" ? (
            <OrbitOverlay
              referenceFrame={referenceFrame}
              onReferenceFrameChange={setReferenceFrame}
              satellites={simInfo?.satellites}
              centralBody={centralBody}
              epochJd={epochJd}
              orbitInfo={fileSource.orbitInfo}
              simInfo={simInfo}
              totalPoints={totalPoints}
              activePerturbations={activePerturbations}
              defaultMarkerShape={defaultMarkerShape}
              onDefaultMarkerShapeChange={setDefaultMarkerShape}
              markerShapeOverrides={markerShapeOverrides}
              onMarkerShapeOverride={handleMarkerShapeOverride}
              directionVectors={directionVectors}
              onDirectionVectorsChange={setDirectionVectors}
              centredSatelliteId={centredSatelliteId}
              drawableVectorKinds={orbitDrawableKinds}
            />
          ) : (
            <AttitudeOverlay
              satellites={attitudeSubjects}
              selectedSatelliteId={selectedSatelliteId}
              onSelectedSatelliteChange={setSelectedSatelliteId}
              orientation={attitudeFrame}
              onOrientationChange={setAttitudeFrame}
              localOrbitalUnavailable={
                attitudeFrameAvailable("localOrbital")
                  ? undefined
                  : "Requires a position and velocity"
              }
              bodyFixedUnavailable={
                centralBody !== "earth"
                  ? "The viewer models only Earth's rotation"
                  : epochJd == null
                    ? "Requires epoch"
                    : undefined
              }
              sunUnavailable={epochJd == null ? "Requires epoch" : undefined}
              nadirUnavailable={
                attitudeDrawableKinds.includes("nadir") ? undefined : "Requires a position"
              }
              directionVectors={directionVectors}
              onDirectionVectorsChange={setDirectionVectors}
              drawnVectorKinds={attitudeVectorKinds}
              hasBody={attitudeBody != null}
            />
          )}
        </div>

        {/* The orts app dogfoods the public scene APIs: it owns the Canvas
            (camera up = SCENE_UP, no global THREE.Object3D.DEFAULT_UP mutation)
            and feeds the scene the public state types. Keyed on the view so the
            camera props — read only at mount — apply to the view being shown;
            keyed on nothing else, or a frame change would rebuild the WebGL
            context and re-fetch every GLTF. */}
        <Canvas
          key={view}
          camera={{
            position: view === "attitude" ? ATTITUDE_CAMERA_POSITION : DEFAULT_CAMERA_POSITION,
            up: SCENE_UP,
            fov: view === "attitude" ? ATTITUDE_FOV : 60,
            near: 0.01,
            far: view === "attitude" ? 100 : 1000,
          }}
          gl={{ logarithmicDepthBuffer: view !== "attitude" }}
          style={{ position: "absolute", top: 0, left: 0, width: "100%", height: "100%" }}
        >
          {/* The app asks for the viewer's framing: the constant above is the
              viewing direction and a starting distance, and the fit settles the
              distance and the far plane once the canvas has a size. */}
          {view === "attitude" && <InitialCameraFit fov={ATTITUDE_FOV} reframe />}
          {view === "orbit" ? (
            <OrbitScene
              centralBody={{ id: centralBody, radiusKm: centralBodyRadius }}
              satellites={satellites}
              referenceFrame={viewerFrame}
              epochJd={epochJd ?? undefined}
              time={snapshot.currentTime}
              defaultMarkerShape={defaultMarkerShape}
              directionVectors={directionVectors}
              atmosphereScale="visual"
              textureVersion={textureRevision}
              textureBaseUrl={textureBaseUrl}
            />
          ) : (
            attitudeBody != null && (
              <AttitudeScene
                centralBody={{ id: centralBody }}
                body={attitudeBody}
                orientation={attitudeFrame}
                epochJd={epochJd ?? undefined}
                time={snapshot.currentTime}
                defaultMarkerShape={defaultMarkerShape}
                directionVectors={directionVectors}
              />
            )
          )}
        </Canvas>

        {view === "attitude" && attitudeBody == null && (
          <div className="scene-placeholder" data-testid="attitude-no-data">
            No attitude data for this spacecraft.
          </div>
        )}
      </div>

      {/* Graph panel (row 2, column 2). The charts are orbital; the attitude view
          has no use for altitude or energy. */}
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
