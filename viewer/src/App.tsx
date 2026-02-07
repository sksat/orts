import { useState, useCallback, useRef } from "react";
import { Scene } from "./components/Scene.js";
import { PlaybackBar } from "./components/PlaybackBar.js";
import { usePlayback } from "./hooks/usePlayback.js";
import { parseOrbitCSV, OrbitPoint } from "./orbit.js";

/**
 * Main application component.
 *
 * Manages loaded orbit data, file input for CSV loading,
 * renders the 3D scene and playback controls.
 */
export function App() {
  const [points, setPoints] = useState<OrbitPoint[] | null>(null);
  const [orbitInfo, setOrbitInfo] = useState<string>("");
  const fileInputRef = useRef<HTMLInputElement>(null);

  const { controller, snapshot } = usePlayback(points);

  const handleLoadClick = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleFileChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (!file) return;

      const reader = new FileReader();
      reader.onload = () => {
        const text = reader.result as string;
        const parsed = parseOrbitCSV(text);

        if (parsed.length === 0) {
          setOrbitInfo("No valid orbit data found in file.");
          setPoints(null);
          return;
        }

        setPoints(parsed);

        const duration = parsed[parsed.length - 1].t - parsed[0].t;
        setOrbitInfo(
          `Loaded: ${file.name} | ${parsed.length} points | Duration: ${duration.toFixed(1)} s`
        );
      };

      reader.readAsText(file);

      // Reset file input so the same file can be re-loaded
      e.target.value = "";
    },
    []
  );

  return (
    <>
      {/* 3D Scene */}
      <Scene
        points={points}
        satellitePosition={snapshot.satellitePosition}
        trailVisibleCount={snapshot.trailVisibleCount}
      />

      {/* UI overlay: load button and orbit info */}
      <div className="ui-overlay">
        <button className="load-csv-btn" onClick={handleLoadClick}>
          Load Orbit CSV
        </button>
        {orbitInfo && <div className="orbit-info">{orbitInfo}</div>}
      </div>

      {/* Hidden file input */}
      <input
        ref={fileInputRef}
        type="file"
        accept=".csv,.txt"
        style={{ display: "none" }}
        onChange={handleFileChange}
      />

      {/* Playback bar (only shown when data is loaded) */}
      {controller && (
        <PlaybackBar
          playback={controller}
          isPlaying={snapshot.isPlaying}
          fraction={snapshot.fraction}
          elapsedTime={snapshot.elapsedTime}
          totalDuration={snapshot.totalDuration}
        />
      )}
    </>
  );
}
