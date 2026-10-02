import { useCallback, useState } from "react";
import type { Pacing } from "../protocol/generated/Pacing.js";
import type { SatelliteConfig } from "../protocol/generated/SatelliteConfig.js";
import type { SimConfig } from "../protocol/generated/SimConfig.js";
import controlStyles from "../styles/controls.module.css";
import styles from "./SimConfigForm.module.css";

export type OrbitMode = "preset" | "circular" | "tle";

export interface PresetDef {
  label: string;
  detail: string;
  satellite: SatelliteConfig;
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

/** The form always populates `satellites`; narrow the generated wire type. */
export type FormSimConfig = SimConfig & { satellites: SatelliteConfig[] };

/** Pure function: build the `start_simulation` SimConfig payload from form state. */
export function buildSimConfig(state: FormState): FormSimConfig {
  let satellite: SatelliteConfig;

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

/** The speed choice: a pacing to ask for, or "" to leave it to the server. */
export type PacingChoice = Pacing | "";

/**
 * The speed choices the form offers, labelled. The server's default is named
 * when the server has said what it is (its idle status), since "server
 * default" alone does not say how fast the simulation will run.
 */
export function pacingChoices(
  serverDefault: Pacing | null,
): { value: PacingChoice; label: string }[] {
  return [
    {
      value: "",
      label: serverDefault === null ? "Server default" : `Server default (${serverDefault})`,
    },
    { value: "realtime", label: "Realtime (1 sim s = 1 s)" },
    { value: "accelerated", label: "Accelerated (faster than real time)" },
  ];
}

/** The pacing a choice asks the server for; `undefined` leaves it out. */
export function pacingOfChoice(choice: PacingChoice): Pacing | undefined {
  return choice === "" ? undefined : choice;
}

/** The integration step and the output interval, in seconds. */
export interface Steps {
  dt: number;
  outputInterval: number;
}

/**
 * The step defaults for a pacing. Realtime sends a state per output interval
 * of wall time, so it gets 0.1 s steps (10 states a second); accelerated keeps
 * the 1 s / 10 s it has always had, which the server plays out 100x faster.
 */
export function defaultStepsFor(pacing: Pacing): Steps {
  return pacing === "realtime" ? { dt: 0.1, outputInterval: 0.1 } : { dt: 1, outputInterval: 10 };
}

/**
 * The steps a start sends: each one the user typed (non-null in `typed`),
 * else the default for the pacing the start will run at. "Server default"
 * runs at what the server's idle status said, and accelerated before it has
 * said anything — the pacing a server without `--realtime` runs at.
 *
 * A default filled in next to a typed value is fitted around it, since the
 * server refuses an output interval below dt: a typed dt raises the output
 * interval to at least itself, a typed output interval caps dt. Two typed
 * values are sent as typed, and the server says what is wrong with them.
 */
export function resolveSteps(
  typed: { dt: number | null; outputInterval: number | null },
  choice: PacingChoice,
  serverDefault: Pacing | null,
): Steps {
  const pacing = pacingOfChoice(choice) ?? serverDefault ?? "accelerated";
  const defaults = defaultStepsFor(pacing);
  if (typed.dt !== null && typed.outputInterval !== null) {
    return { dt: typed.dt, outputInterval: typed.outputInterval };
  }
  if (typed.dt !== null) {
    return { dt: typed.dt, outputInterval: Math.max(defaults.outputInterval, typed.dt) };
  }
  if (typed.outputInterval !== null) {
    return {
      dt: Math.min(defaults.dt, typed.outputInterval),
      outputInterval: typed.outputInterval,
    };
  }
  return defaults;
}

export interface SimConfigFormProps {
  /** `pacing` left out runs at the server's default. */
  onStart: (config: SimConfig, pacing?: Pacing) => void;
  /** The server's default pacing, or null while it is not known. */
  serverDefaultPacing?: Pacing | null;
}

export function SimConfigForm({ onStart, serverDefaultPacing = null }: SimConfigFormProps) {
  const [orbitMode, setOrbitMode] = useState<OrbitMode>("preset");
  const [presetIndex, setPresetIndex] = useState(0);
  const [altitude, setAltitude] = useState(400);
  const [inclination, setInclination] = useState(0);
  const [raan, setRaan] = useState(0);
  const [tleLine1, setTleLine1] = useState("");
  const [tleLine2, setTleLine2] = useState("");
  const [showAdvanced, setShowAdvanced] = useState(false);
  // null until the user types one: the speed's default shows meanwhile, and
  // follows the speed when it changes.
  const [typedDt, setTypedDt] = useState<number | null>(null);
  const [typedOutputInterval, setTypedOutputInterval] = useState<number | null>(null);
  const [atmosphere, setAtmosphere] = useState("exponential");
  const [pacingChoice, setPacingChoice] = useState<PacingChoice>("");
  const { dt, outputInterval } = resolveSteps(
    { dt: typedDt, outputInterval: typedOutputInterval },
    pacingChoice,
    serverDefaultPacing,
  );

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
    onStart(config, pacingOfChoice(pacingChoice));
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
    pacingChoice,
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

      <div className={styles.inputs}>
        <label className={styles.label}>
          Speed
          <select
            className={styles.select}
            data-testid="sim-config-pacing"
            value={pacingChoice}
            onChange={(e) => setPacingChoice(e.target.value as PacingChoice)}
          >
            {pacingChoices(serverDefaultPacing).map((c) => (
              <option key={c.value} value={c.value}>
                {c.label}
              </option>
            ))}
          </select>
        </label>
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
              onChange={(e) => setTypedDt(Number(e.target.value))}
              min={0.1}
              step="any"
            />
          </label>
          <label className={styles.label}>
            Output interval (s)
            <input
              type="number"
              className={styles.input}
              value={outputInterval}
              onChange={(e) => setTypedOutputInterval(Number(e.target.value))}
              min={0.1}
              step="any"
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
