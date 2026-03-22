import type { AtmoModel, FieldComponent, MagModel } from "../types.js";
import { dateToJd, type ViewerParams } from "../types.js";

interface ControlsProps {
  params: ViewerParams;
  onChange: (params: ViewerParams) => void;
  activeTab: string;
  swAvailable?: boolean;
  isGlobe?: boolean;
  globeDisplayMode?: "single" | "volume";
  onDisplayModeChange?: (mode: "single" | "volume") => void;
}

const SOLAR_PRESETS: Record<string, { f107: number; ap: number }> = {
  "Solar Min": { f107: 70, ap: 4 },
  "Solar Moderate": { f107: 150, ap: 15 },
  "Solar Max": { f107: 250, ap: 50 },
};

const styles = {
  panel: {
    padding: "12px 16px",
    background: "#14141a",
    borderBottom: "1px solid #2a2a35",
    display: "flex",
    flexWrap: "wrap" as const,
    gap: "16px",
    alignItems: "center",
    fontSize: "13px",
  },
  group: {
    display: "flex",
    alignItems: "center",
    gap: "6px",
  },
  label: {
    color: "#888",
    whiteSpace: "nowrap" as const,
  },
  input: {
    background: "#1e1e28",
    border: "1px solid #333",
    borderRadius: "4px",
    color: "#e0e0e0",
    padding: "4px 8px",
    fontSize: "13px",
  },
  select: {
    background: "#1e1e28",
    border: "1px solid #333",
    borderRadius: "4px",
    color: "#e0e0e0",
    padding: "4px 6px",
    fontSize: "13px",
  },
  button: {
    background: "#2a2a35",
    border: "1px solid #444",
    borderRadius: "4px",
    color: "#ccc",
    padding: "3px 8px",
    fontSize: "11px",
    cursor: "pointer",
  },
};

function jdToDateString(jd: number): string {
  // Approximate JD → Date (good enough for UI)
  const ms = (jd - 2440587.5) * 86400000;
  return new Date(ms).toISOString().slice(0, 10);
}

export function Controls({
  params,
  onChange,
  activeTab,
  swAvailable = false,
  isGlobe = false,
  globeDisplayMode = "volume",
  onDisplayModeChange,
}: ControlsProps) {
  const update = (partial: Partial<ViewerParams>) => onChange({ ...params, ...partial });

  const showAtmo =
    activeTab === "atmosphere-profile" || activeTab === "atmosphere-map" || activeTab === "globe";
  const showMag = activeTab === "magnetic-map" || activeTab === "globe";
  // SW toggle only for views that support _sw API (not profile)
  const showSwToggle = showAtmo && activeTab !== "atmosphere-profile";

  return (
    <div style={styles.panel}>
      {/* Epoch */}
      <div style={styles.group}>
        <span style={styles.label}>Epoch:</span>
        <input
          type="date"
          style={styles.input}
          value={jdToDateString(params.epochJd)}
          onChange={(e) => {
            const d = new Date(`${e.target.value}T00:00:00Z`);
            if (!Number.isNaN(d.getTime())) update({ epochJd: dateToJd(d) });
          }}
        />
      </div>

      {/* Globe display mode */}
      {isGlobe && onDisplayModeChange && (
        <div style={styles.group}>
          <span style={styles.label}>Display:</span>
          <select
            style={styles.select}
            value={globeDisplayMode}
            onChange={(e) => onDisplayModeChange(e.target.value as "single" | "volume")}
          >
            <option value="single">Single</option>
            <option value="volume">Volume</option>
          </select>
        </div>
      )}

      {/* Altitude */}
      <div style={styles.group}>
        <span style={styles.label}>Alt:</span>
        <input
          type="range"
          min={100}
          max={1000}
          step={10}
          value={params.altitudeKm}
          disabled={isGlobe && globeDisplayMode === "volume"}
          onChange={(e) => update({ altitudeKm: Number(e.target.value) })}
        />
        <span style={{ color: "#ccc", minWidth: "50px" }}>
          {isGlobe && globeDisplayMode === "volume" ? "100-1000 km" : `${params.altitudeKm} km`}
        </span>
      </div>

      {/* Magnetic model & component */}
      {showMag && (
        <>
          <div style={styles.group}>
            <span style={styles.label}>Mag:</span>
            <select
              style={styles.select}
              value={params.magModel}
              onChange={(e) => update({ magModel: e.target.value as MagModel })}
            >
              <option value="igrf">IGRF-14</option>
              <option value="dipole">Tilted Dipole</option>
            </select>
          </div>
          <div style={styles.group}>
            <span style={styles.label}>Component:</span>
            <select
              style={styles.select}
              value={params.fieldComponent}
              onChange={(e) => update({ fieldComponent: e.target.value as FieldComponent })}
            >
              <option value="total">Total Intensity</option>
              <option value="inclination">Inclination</option>
              <option value="declination">Declination</option>
              <option value="north">North (X)</option>
              <option value="east">East (Y)</option>
              <option value="down">Down (Z)</option>
            </select>
          </div>
        </>
      )}

      {/* Atmosphere model */}
      {showAtmo && activeTab !== "atmosphere-profile" && (
        <div style={styles.group}>
          <span style={styles.label}>Atmo:</span>
          <select
            style={styles.select}
            value={params.atmoModel}
            onChange={(e) => update({ atmoModel: e.target.value as AtmoModel })}
          >
            <option value="exponential">Exponential</option>
            <option value="harris-priester">Harris-Priester</option>
            <option value="nrlmsise00">NRLMSISE-00</option>
          </select>
        </div>
      )}

      {/* Solar activity */}
      {showAtmo && (
        <>
          {swAvailable && showSwToggle && (
            <div style={styles.group}>
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
                  checked={params.spaceWeatherMode === "real"}
                  onChange={(e) =>
                    update({ spaceWeatherMode: e.target.checked ? "real" : "constant" })
                  }
                />
                Real SW
              </label>
            </div>
          )}
          <div style={styles.group}>
            <span style={styles.label}>F10.7:</span>
            <input
              type="range"
              min={70}
              max={250}
              step={5}
              value={params.f107}
              disabled={params.spaceWeatherMode === "real"}
              onChange={(e) => update({ f107: Number(e.target.value) })}
            />
            <span style={{ color: "#ccc", minWidth: "30px" }}>{params.f107}</span>
          </div>
          <div style={styles.group}>
            <span style={styles.label}>Ap:</span>
            <input
              type="range"
              min={0}
              max={100}
              step={1}
              value={params.ap}
              disabled={params.spaceWeatherMode === "real"}
              onChange={(e) => update({ ap: Number(e.target.value) })}
            />
            <span style={{ color: "#ccc", minWidth: "24px" }}>{params.ap}</span>
          </div>
          {params.spaceWeatherMode === "constant" && (
            <div style={styles.group}>
              {Object.entries(SOLAR_PRESETS).map(([name, preset]) => (
                <button
                  key={name}
                  type="button"
                  style={styles.button}
                  onClick={() => update(preset)}
                >
                  {name}
                </button>
              ))}
            </div>
          )}
        </>
      )}

      {/* Resolution */}
      <div style={styles.group}>
        <span style={styles.label}>Grid:</span>
        <select
          style={styles.select}
          value={params.nLat}
          onChange={(e) => update({ nLat: Number(e.target.value) })}
        >
          <option value={36}>5° (fast)</option>
          <option value={90}>2°</option>
          <option value={180}>1° (slow)</option>
        </select>
      </div>
    </div>
  );
}
