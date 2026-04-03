import { useCallback, useState } from "react";
import styles from "./SimConfigForm.module.css";
import controlStyles from "../styles/controls.module.css";

export type OrbitMode = "preset" | "circular" | "tle";

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

interface NoradOrbit {
  type: "norad";
  norad_id: number;
}

interface AttitudePayloadConfig {
  inertia_diag: [number, number, number];
  mass: number;
  initial_quaternion?: [number, number, number, number];
  initial_angular_velocity?: [number, number, number];
}

export interface SatellitePayload {
  id?: string;
  name?: string;
  orbit: CircularOrbit | TleOrbit | NoradOrbit;
  attitude?: AttitudePayloadConfig;
}

export interface PresetDef {
  label: string;
  detail: string;
  satellite: SatellitePayload;
}

export const PRESETS: PresetDef[] = [
  {
    label: "ISS",
    detail: "NORAD 25544",
    satellite: {
      id: "iss",
      name: "ISS",
      orbit: { type: "norad", norad_id: 25544 },
      attitude: {
        // Approximate ISS inertia tensor [kg·m²] and mass [kg]
        inertia_diag: [128_913_000, 107_321_000, 201_433_000],
        mass: 420_000,
      },
    },
  },
  {
    label: "SSO",
    detail: "800 km / 98.6°",
    satellite: {
      orbit: { type: "circular", altitude: 800, inclination: 98.6, raan: 0 },
    },
  },
  {
    label: "GEO",
    detail: "35786 km / 0°",
    satellite: {
      orbit: { type: "circular", altitude: 35786, inclination: 0, raan: 0 },
    },
  },
];

export interface SimConfigPayload {
  dt: number;
  output_interval: number;
  atmosphere: string;
  satellites: SatellitePayload[];
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
  let satellite: SatellitePayload;

  if (state.orbitMode === "tle") {
    satellite = { orbit: { type: "tle", line1: state.tleLine1, line2: state.tleLine2 } };
  } else if (state.orbitMode === "preset") {
    satellite = PRESETS[state.presetIndex].satellite;
  } else {
    satellite = {
      orbit: {
        type: "circular",
        altitude: state.altitude,
        inclination: state.inclination,
        raan: state.raan,
      },
    };
  }

  return {
    dt: state.dt,
    output_interval: state.outputInterval,
    atmosphere: state.atmosphere,
    satellites: [satellite],
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
      orbitMode,
      presetIndex,
      altitude,
      inclination,
      raan,
      tleLine1,
      tleLine2,
      dt,
      outputInterval,
      atmosphere,
    });
    onStart(config);
  }, [
    orbitMode,
    presetIndex,
    altitude,
    inclination,
    raan,
    tleLine1,
    tleLine2,
    dt,
    outputInterval,
    atmosphere,
    onStart,
  ]);

  return (
    <div className={styles.form} data-testid="sim-config-form">
      <div className={styles.section}>
        <div className={controlStyles.modeToggle} style={{ marginBottom: 8 }}>
          <button
            className={`${controlStyles.modeToggleBtn} ${orbitMode === "preset" ? controlStyles.active : ""}`}
            onClick={() => setOrbitMode("preset")}
          >
            Preset
          </button>
          <button
            className={`${controlStyles.modeToggleBtn} ${orbitMode === "circular" ? controlStyles.active : ""}`}
            onClick={() => setOrbitMode("circular")}
          >
            Custom
          </button>
          <button
            className={`${controlStyles.modeToggleBtn} ${orbitMode === "tle" ? controlStyles.active : ""}`}
            onClick={() => setOrbitMode("tle")}
          >
            TLE
          </button>
        </div>

        {orbitMode === "preset" && (
          <div className={styles.presetGroup}>
            {PRESETS.map((p, i) => (
              <button
                key={p.label}
                className={`${styles.presetBtn} ${presetIndex === i ? styles.active : ""}`}
                data-testid="preset-btn"
                data-state={presetIndex === i ? "active" : ""}
                onClick={() => setPresetIndex(i)}
              >
                {p.label}
                <span className={styles.presetDetail}>{p.detail}</span>
              </button>
            ))}
          </div>
        )}

        {orbitMode === "circular" && (
          <div className={styles.inputs}>
            <label className={styles.label}>
              Altitude (km)
              <input
                type="number"
                className={styles.input}
                value={altitude}
                onChange={(e) => setAltitude(Number(e.target.value))}
              />
            </label>
            <label className={styles.label}>
              Inclination (°)
              <input
                type="number"
                className={styles.input}
                value={inclination}
                onChange={(e) => setInclination(Number(e.target.value))}
                step={0.1}
              />
            </label>
            <label className={styles.label}>
              RAAN (°)
              <input
                type="number"
                className={styles.input}
                value={raan}
                onChange={(e) => setRaan(Number(e.target.value))}
                step={0.1}
              />
            </label>
          </div>
        )}

        {orbitMode === "tle" && (
          <div className={styles.inputs}>
            <label className={styles.label}>
              TLE Line 1
              <input
                type="text"
                className={`${styles.input} ${styles.tleInput}`}
                value={tleLine1}
                onChange={(e) => setTleLine1(e.target.value)}
                placeholder="1 25544U ..."
              />
            </label>
            <label className={styles.label}>
              TLE Line 2
              <input
                type="text"
                className={`${styles.input} ${styles.tleInput}`}
                value={tleLine2}
                onChange={(e) => setTleLine2(e.target.value)}
                placeholder="2 25544 ..."
              />
            </label>
          </div>
        )}
      </div>

      <button className={styles.advancedToggle} onClick={() => setShowAdvanced(!showAdvanced)}>
        {showAdvanced ? "▾ Advanced" : "▸ Advanced"}
      </button>

      {showAdvanced && (
        <div className={styles.inputs}>
          <label className={styles.label}>
            dt (s)
            <input
              type="number"
              className={styles.input}
              value={dt}
              onChange={(e) => setDt(Number(e.target.value))}
              min={0.1}
              step={0.5}
            />
          </label>
          <label className={styles.label}>
            Output interval (s)
            <input
              type="number"
              className={styles.input}
              value={outputInterval}
              onChange={(e) => setOutputInterval(Number(e.target.value))}
              min={1}
              step={5}
            />
          </label>
          <label className={styles.label}>
            Atmosphere
            <select
              className={styles.select}
              value={atmosphere}
              onChange={(e) => setAtmosphere(e.target.value)}
            >
              <option value="exponential">Exponential</option>
              <option value="harris-priester">Harris-Priester</option>
              <option value="nrlmsise00">NRLMSISE-00</option>
            </select>
          </label>
        </div>
      )}

      <button className={styles.startBtn} data-testid="sim-config-start-btn" onClick={handleStart}>
        Start Simulation
      </button>
    </div>
  );
}
