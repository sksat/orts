import { useState, useCallback } from "react";

export type OrbitMode = "preset" | "circular" | "tle";

export interface PresetDef {
  label: string;
  altitude: number;
  inclination: number;
  raan: number;
}

export const PRESETS: PresetDef[] = [
  { label: "ISS-like", altitude: 420, inclination: 51.6, raan: 0 },
  { label: "SSO", altitude: 800, inclination: 98.6, raan: 0 },
  { label: "GEO", altitude: 35786, inclination: 0, raan: 0 },
];

interface CircularOrbit {
  type: "circular";
  altitude: number;
  inclination: number;
  raan: number;
}

interface TleOrbit {
  type: "tle";
  line1: string;
  line2: string;
}

export interface SimConfigPayload {
  dt: number;
  output_interval: number;
  atmosphere: string;
  satellites: { orbit: CircularOrbit | TleOrbit }[];
}

export interface FormState {
  orbitMode: OrbitMode;
  presetIndex: number;
  altitude: number;
  inclination: number;
  raan: number;
  tleLine1: string;
  tleLine2: string;
  dt: number;
  outputInterval: number;
  atmosphere: string;
}

/** Pure function: build SimConfig payload from form state. */
export function buildSimConfig(state: FormState): SimConfigPayload {
  let orbit: CircularOrbit | TleOrbit;

  if (state.orbitMode === "tle") {
    orbit = { type: "tle", line1: state.tleLine1, line2: state.tleLine2 };
  } else if (state.orbitMode === "preset") {
    const preset = PRESETS[state.presetIndex];
    orbit = { type: "circular", altitude: preset.altitude, inclination: preset.inclination, raan: preset.raan };
  } else {
    orbit = { type: "circular", altitude: state.altitude, inclination: state.inclination, raan: state.raan };
  }

  return {
    dt: state.dt,
    output_interval: state.outputInterval,
    atmosphere: state.atmosphere,
    satellites: [{ orbit }],
  };
}

export interface SimConfigFormProps {
  onStart: (config: SimConfigPayload) => void;
}

export function SimConfigForm({ onStart }: SimConfigFormProps) {
  const [orbitMode, setOrbitMode] = useState<OrbitMode>("preset");
  const [presetIndex, setPresetIndex] = useState(0);
  const [altitude, setAltitude] = useState(400);
  const [inclination, setInclination] = useState(0);
  const [raan, setRaan] = useState(0);
  const [tleLine1, setTleLine1] = useState("");
  const [tleLine2, setTleLine2] = useState("");
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [dt, setDt] = useState(1);
  const [outputInterval, setOutputInterval] = useState(10);
  const [atmosphere, setAtmosphere] = useState("exponential");

  const handleStart = useCallback(() => {
    const config = buildSimConfig({
      orbitMode, presetIndex, altitude, inclination, raan,
      tleLine1, tleLine2, dt, outputInterval, atmosphere,
    });
    onStart(config);
  }, [orbitMode, presetIndex, altitude, inclination, raan, tleLine1, tleLine2, dt, outputInterval, atmosphere, onStart]);

  return (
    <div className="sim-config-form">
      <div className="sim-config-section">
        <div className="mode-toggle" style={{ marginBottom: 8 }}>
          <button className={`mode-toggle-btn ${orbitMode === "preset" ? "active" : ""}`} onClick={() => setOrbitMode("preset")}>Preset</button>
          <button className={`mode-toggle-btn ${orbitMode === "circular" ? "active" : ""}`} onClick={() => setOrbitMode("circular")}>Custom</button>
          <button className={`mode-toggle-btn ${orbitMode === "tle" ? "active" : ""}`} onClick={() => setOrbitMode("tle")}>TLE</button>
        </div>

        {orbitMode === "preset" && (
          <div className="preset-group">
            {PRESETS.map((p, i) => (
              <button
                key={p.label}
                className={`preset-btn ${presetIndex === i ? "active" : ""}`}
                onClick={() => setPresetIndex(i)}
              >
                {p.label}
                <span className="preset-detail">{p.altitude} km / {p.inclination}°</span>
              </button>
            ))}
          </div>
        )}

        {orbitMode === "circular" && (
          <div className="sim-config-inputs">
            <label className="sim-config-label">
              Altitude (km)
              <input type="number" className="sim-config-input" value={altitude} onChange={(e) => setAltitude(Number(e.target.value))} />
            </label>
            <label className="sim-config-label">
              Inclination (°)
              <input type="number" className="sim-config-input" value={inclination} onChange={(e) => setInclination(Number(e.target.value))} step={0.1} />
            </label>
            <label className="sim-config-label">
              RAAN (°)
              <input type="number" className="sim-config-input" value={raan} onChange={(e) => setRaan(Number(e.target.value))} step={0.1} />
            </label>
          </div>
        )}

        {orbitMode === "tle" && (
          <div className="sim-config-inputs">
            <label className="sim-config-label">
              TLE Line 1
              <input type="text" className="sim-config-input sim-config-tle" value={tleLine1} onChange={(e) => setTleLine1(e.target.value)} placeholder="1 25544U ..." />
            </label>
            <label className="sim-config-label">
              TLE Line 2
              <input type="text" className="sim-config-input sim-config-tle" value={tleLine2} onChange={(e) => setTleLine2(e.target.value)} placeholder="2 25544 ..." />
            </label>
          </div>
        )}
      </div>

      <button className="sim-config-advanced-toggle" onClick={() => setShowAdvanced(!showAdvanced)}>
        {showAdvanced ? "▾ Advanced" : "▸ Advanced"}
      </button>

      {showAdvanced && (
        <div className="sim-config-inputs">
          <label className="sim-config-label">
            dt (s)
            <input type="number" className="sim-config-input" value={dt} onChange={(e) => setDt(Number(e.target.value))} min={0.1} step={0.5} />
          </label>
          <label className="sim-config-label">
            Output interval (s)
            <input type="number" className="sim-config-input" value={outputInterval} onChange={(e) => setOutputInterval(Number(e.target.value))} min={1} step={5} />
          </label>
          <label className="sim-config-label">
            Atmosphere
            <select className="sim-config-select" value={atmosphere} onChange={(e) => setAtmosphere(e.target.value)}>
              <option value="exponential">Exponential</option>
              <option value="harris-priester">Harris-Priester</option>
              <option value="nrlmsise00">NRLMSISE-00</option>
            </select>
          </label>
        </div>
      )}

      <button className="sim-config-start-btn" onClick={handleStart}>
        Start Simulation
      </button>
    </div>
  );
}
