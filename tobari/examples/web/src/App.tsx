import { useCallback, useEffect, useRef, useState } from "react";
import { Controls } from "./controls/Controls.js";
import { PlaybackBar } from "./controls/PlaybackBar.js";
import { AtmosphereMap } from "./panels/AtmosphereMap.js";
import { AtmosphereProfile } from "./panels/AtmosphereProfile.js";
import { GlobeView } from "./panels/GlobeView.js";
import { MagneticFieldMap } from "./panels/MagneticFieldMap.js";
import { SpaceWeatherChart } from "./panels/SpaceWeatherChart.js";
import { dateToJd, type ViewerParams } from "./types.js";
import { earthRotationAngle, initKaname } from "./wasm/kanameInit.js";
import { initWorker, onSpaceWeatherReady } from "./wasm/workerClient.js";

type TabId =
  | "globe-mag"
  | "globe-atmo"
  | "magnetic-map"
  | "atmosphere-profile"
  | "atmosphere-map"
  | "space-weather";

const BASE_TABS: { id: TabId; label: string }[] = [
  { id: "globe-mag", label: "Globe (Magnetic)" },
  { id: "globe-atmo", label: "Globe (Atmosphere)" },
  { id: "magnetic-map", label: "Magnetic Field Map" },
  { id: "atmosphere-profile", label: "Atmosphere Profile" },
  { id: "atmosphere-map", label: "Atmosphere Map" },
];

const DEFAULT_PARAMS: ViewerParams = {
  epochJd: dateToJd(new Date("2025-01-01T00:00:00Z")),
  altitudeKm: 400,
  f107: 150,
  ap: 15,
  fieldComponent: "total",
  atmoModel: "harris-priester",
  magModel: "igrf",
  nLat: 90,
  spaceWeatherMode: "constant",
};

const styles = {
  app: {
    display: "flex",
    flexDirection: "column" as const,
    height: "100vh",
    background: "#0a0a0f",
    color: "#e0e0e0",
  },
  header: {
    display: "flex",
    alignItems: "center",
    gap: "24px",
    padding: "8px 16px",
    background: "#10101a",
    borderBottom: "1px solid #2a2a35",
  },
  title: {
    fontSize: "16px",
    fontWeight: 700,
    color: "#8ab4f8",
    letterSpacing: "0.5px",
  },
  tabs: {
    display: "flex",
    gap: "2px",
  },
  tab: (active: boolean) => ({
    padding: "6px 14px",
    fontSize: "13px",
    background: active ? "#2a2a45" : "transparent",
    color: active ? "#e0e0e0" : "#888",
    border: "none",
    borderBottom: active ? "2px solid #6688cc" : "2px solid transparent",
    cursor: "pointer",
    borderRadius: "4px 4px 0 0",
  }),
  content: {
    flex: 1,
    overflow: "hidden",
  },
  loading: {
    display: "flex",
    justifyContent: "center",
    alignItems: "center",
    height: "100vh",
    fontSize: "18px",
    color: "#888",
  },
};

function controlsTab(tab: TabId): string {
  if (tab === "globe-mag") return "magnetic-map";
  if (tab === "globe-atmo") return "atmosphere-map";
  return tab;
}

export function App() {
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<TabId>("globe-mag");
  const [params, setParams] = useState<ViewerParams>(DEFAULT_PARAMS);
  const [playing, setPlaying] = useState(false);
  const [speedDaysPerSec, setSpeedDaysPerSec] = useState(30);
  const [showRotation, setShowRotation] = useState(false);
  const [globeDisplayMode, setGlobeDisplayMode] = useState<"single" | "volume">("volume");
  const [swRange, setSwRange] = useState<{ jdFirst: number; jdLast: number } | null>(null);
  const lastFrameRef = useRef<number>(0);

  useEffect(() => {
    onSpaceWeatherReady((range) => {
      setSwRange(range);
    });
    Promise.all([initKaname(), initWorker()])
      .then(() => setReady(true))
      .catch((e) => setError(String(e)));
  }, []);

  // Playback loop: advance epoch
  useEffect(() => {
    if (!playing) return;
    let rafId: number;
    lastFrameRef.current = performance.now();

    const tick = (now: number) => {
      const dt = (now - lastFrameRef.current) / 1000;
      lastFrameRef.current = now;
      setParams((prev) => ({
        ...prev,
        epochJd: prev.epochJd + dt * speedDaysPerSec,
      }));
      rafId = requestAnimationFrame(tick);
    };
    rafId = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(rafId);
  }, [playing, speedDaysPerSec]);

  const handleParamsChange = useCallback((newParams: ViewerParams) => {
    setParams(newParams);
  }, []);

  if (error) {
    return <div style={styles.loading}>Failed to load WASM: {error}</div>;
  }
  if (!ready) {
    return <div style={styles.loading}>Loading tobari WASM...</div>;
  }

  // Compute Earth rotation via kaname WASM (only when enabled)
  const rotation = showRotation ? earthRotationAngle(params.epochJd) : 0;

  // Show Space Weather tab only when data is available
  const tabs = swRange
    ? [...BASE_TABS, { id: "space-weather" as TabId, label: "Space Weather" }]
    : BASE_TABS;

  const isGlobe = activeTab === "globe-mag" || activeTab === "globe-atmo";

  return (
    <div style={styles.app}>
      <div style={styles.header}>
        <span style={styles.title}>tobari</span>
        <div style={styles.tabs}>
          {tabs.map((tab) => (
            <button
              key={tab.id}
              type="button"
              style={styles.tab(activeTab === tab.id)}
              onClick={() => setActiveTab(tab.id)}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>

      <Controls
        params={params}
        onChange={handleParamsChange}
        activeTab={controlsTab(activeTab)}
        swAvailable={swRange !== null}
        isGlobe={isGlobe}
        globeDisplayMode={globeDisplayMode}
        onDisplayModeChange={setGlobeDisplayMode}
      />
      <PlaybackBar
        playing={playing}
        onTogglePlay={() => setPlaying((p) => !p)}
        speed={speedDaysPerSec}
        onSpeedChange={setSpeedDaysPerSec}
        epochJd={params.epochJd}
      >
        {isGlobe && (
          <label
            style={{
              display: "flex",
              alignItems: "center",
              gap: "4px",
              color: "#888",
              cursor: "pointer",
            }}
          >
            <input
              type="checkbox"
              checked={showRotation}
              onChange={(e) => setShowRotation(e.target.checked)}
            />
            Earth rotation
          </label>
        )}
      </PlaybackBar>

      <div style={styles.content}>
        {activeTab === "globe-mag" && (
          <GlobeView
            params={params}
            layer="magnetic"
            earthRotation={rotation}
            displayMode={globeDisplayMode}
          />
        )}
        {activeTab === "globe-atmo" && (
          <GlobeView
            params={params}
            layer="atmosphere"
            earthRotation={rotation}
            displayMode={globeDisplayMode}
          />
        )}
        {activeTab === "magnetic-map" && <MagneticFieldMap params={params} />}
        {activeTab === "atmosphere-profile" && <AtmosphereProfile params={params} />}
        {activeTab === "atmosphere-map" && <AtmosphereMap params={params} />}
        {activeTab === "space-weather" && <SpaceWeatherChart epochJd={params.epochJd} />}
      </div>
    </div>
  );
}
