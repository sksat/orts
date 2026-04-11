import { OrbitControls } from "@react-three/drei";
import { Canvas } from "@react-three/fiber";
import { useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { useDebouncedValue } from "../hooks/useDebouncedValue.js";
import { overlayFrag, overlayVert } from "../shaders/fieldOverlay.js";
import type { ViewerParams } from "../types.js";
import { earthRotationAngle, isArikaReady } from "../wasm/arikaInit.js";
import {
  atmosphereLatlonMapAsync,
  atmosphereLatlonMapSwAsync,
  atmosphereVolumeAsync,
  atmosphereVolumeSwAsync,
  magneticFieldLatlonMapAsync,
  magneticFieldLinesAsync,
  magneticFieldVolumeAsync,
} from "../wasm/workerClient.js";

interface Props {
  params: ViewerParams;
  layer: "magnetic" | "atmosphere";
}

const EARTH_RADIUS = 1.0; // normalized
const EARTH_RADIUS_KM = 6371.0;
const N_SHELLS = 12;

// GMST is computed by arika WASM via the caller (App) and passed as a prop.

/**
 * Rotation to align Three.js SphereGeometry (pole along Y) with ECI (pole along Z).
 * +π/2 around X maps: local +Y → local +Z.
 * Same as viewer's POLE_ALIGNMENT_ROTATION.
 */
const POLE_ALIGN: [number, number, number] = [Math.PI / 2, 0, 0];

/**
 * Rotation to map ECI (Z-up) to Three.js world (Y-up).
 * -π/2 around X maps: Z → Y.
 * Applied to the outer group containing everything.
 */
const ECI_TO_THREEJS: [number, number, number] = [-Math.PI / 2, 0, 0];

// ---------------------------------------------------------------------------
// Shell mesh: one altitude layer (semi-transparent data overlay)
// ---------------------------------------------------------------------------

function ShellMesh({
  dataTexture,
  radius,
  dataMin,
  dataMax,
  useLogScale,
  opacity,
}: {
  dataTexture: THREE.DataTexture;
  radius: number;
  dataMin: number;
  dataMax: number;
  useLogScale: boolean;
  opacity: number;
}) {
  const material = useMemo(
    () =>
      new THREE.ShaderMaterial({
        uniforms: {
          dataMap: { value: dataTexture },
          dataMin: { value: dataMin },
          dataMax: { value: dataMax },
          opacity: { value: opacity },
          useLogScale: { value: useLogScale },
        },
        vertexShader: overlayVert,
        fragmentShader: overlayFrag,
        transparent: true,
        side: THREE.DoubleSide,
        depthWrite: false,
        blending: THREE.NormalBlending,
      }),
    [],
  );

  useEffect(() => {
    material.uniforms.dataMap.value = dataTexture;
    material.uniforms.dataMin.value = dataMin;
    material.uniforms.dataMax.value = dataMax;
    material.uniforms.opacity.value = opacity;
    material.uniforms.useLogScale.value = useLogScale;
    material.needsUpdate = true;
  }, [material, dataTexture, dataMin, dataMax, opacity, useLogScale]);

  // Pole alignment: SphereGeometry Y-pole → Z-pole (ECI)
  return (
    <group rotation={POLE_ALIGN}>
      <mesh material={material} renderOrder={radius}>
        <sphereGeometry args={[radius, 64, 32]} />
      </mesh>
    </group>
  );
}

// ---------------------------------------------------------------------------
// Multi-shell atmosphere
// ---------------------------------------------------------------------------

interface ShellData {
  texture: THREE.DataTexture;
  min: number;
  max: number;
}

function AtmosphereShells({ params }: { params: ViewerParams }) {
  const [shells, setShells] = useState<ShellData[]>([]);
  // Fix shell ranges across epochs so density bulge movement is visible.
  // Only recalculate when model/altitude/space-weather-mode changes.
  const shellRangesRef = useRef<{ min: number; max: number }[] | null>(null);
  const prevRangeKeyRef = useRef("");
  const nLat = Math.min(params.nLat, 45);
  const nLon = nLat * 2;

  useEffect(() => {
    let cancelled = false;

    const fetchVol =
      params.spaceWeatherMode === "real"
        ? atmosphereVolumeSwAsync(params.atmoModel, 100, 1000, N_SHELLS, params.epochJd, nLat, nLon)
        : atmosphereVolumeAsync(
            params.atmoModel,
            100,
            1000,
            N_SHELLS,
            params.epochJd,
            nLat,
            nLon,
            params.f107,
            params.ap,
          );
    fetchVol.then((vol) => {
      if (cancelled || !vol) return;

      const sliceSize = nLat * nLon;
      const rangeKey = `${params.atmoModel}:${params.spaceWeatherMode}:${params.f107}:${params.ap}`;
      const needNewRanges = !shellRangesRef.current || rangeKey !== prevRangeKeyRef.current;

      const newShells: ShellData[] = [];
      for (let i = 0; i < N_SHELLS; i++) {
        const slice = vol.data.slice(i * sliceSize, (i + 1) * sliceSize);

        let sMin: number;
        let sMax: number;
        if (needNewRanges) {
          sMin = Infinity;
          sMax = -Infinity;
          for (let j = 0; j < slice.length; j++) {
            const v = slice[j];
            if (v > 0 && v < sMin) sMin = v;
            if (v > sMax) sMax = v;
          }
        } else {
          sMin = shellRangesRef.current![i].min;
          sMax = shellRangesRef.current![i].max;
        }

        const tex = new THREE.DataTexture(slice, nLon, nLat, THREE.RedFormat, THREE.FloatType);
        tex.needsUpdate = true;
        tex.wrapS = THREE.RepeatWrapping;
        tex.wrapT = THREE.ClampToEdgeWrapping;
        tex.minFilter = THREE.LinearFilter;
        tex.magFilter = THREE.LinearFilter;
        newShells.push({ texture: tex, min: sMin, max: sMax });
      }

      if (needNewRanges) {
        shellRangesRef.current = newShells.map((s) => ({ min: s.min, max: s.max }));
        prevRangeKeyRef.current = rangeKey;
      }

      setShells((prev) => {
        for (const s of prev) s.texture.dispose();
        return newShells;
      });
    });

    return () => {
      cancelled = true;
    };
  }, [
    params.atmoModel,
    params.epochJd,
    params.f107,
    params.ap,
    params.spaceWeatherMode,
    nLat,
    nLon,
  ]);

  if (shells.length === 0) return null;

  return (
    <>
      {shells.map((shell, i) => {
        const alt = 100 + (900 * i) / (N_SHELLS - 1);
        const radius = EARTH_RADIUS * (1 + alt / EARTH_RADIUS_KM);
        const opacity = 0.06 + i * 0.01;
        return (
          <ShellMesh
            // biome-ignore lint/suspicious/noArrayIndexKey: shells have no stable ID
            key={i}
            dataTexture={shell.texture}
            radius={radius}
            dataMin={shell.min}
            dataMax={shell.max}
            useLogScale={true}
            opacity={Math.max(0.02, opacity)}
          />
        );
      })}
    </>
  );
}

// ---------------------------------------------------------------------------
// Single shell (one altitude, for both atmosphere and magnetic)
// ---------------------------------------------------------------------------

function SingleShell({
  params,
  layer,
}: {
  params: ViewerParams;
  layer: "atmosphere" | "magnetic";
}) {
  const [dataTexture, setDataTexture] = useState<THREE.DataTexture | null>(null);
  const [dataRange, setDataRange] = useState({ min: 0, max: 1 });
  const rangeRef = useRef<{ min: number; max: number } | null>(null);
  const prevKeyRef = useRef("");
  // Debounce epoch for magnetic layer (IGRF barely changes with epoch)
  const debouncedEpoch = useDebouncedValue(params.epochJd, layer === "magnetic" ? 1000 : 0);
  const effectEpoch = layer === "magnetic" ? debouncedEpoch : params.epochJd;

  useEffect(() => {
    let cancelled = false;
    const nLat = params.nLat;
    const nLon = nLat * 2;

    const fetchData =
      layer === "atmosphere"
        ? params.spaceWeatherMode === "real"
          ? atmosphereLatlonMapSwAsync(params.atmoModel, params.altitudeKm, effectEpoch, nLat, nLon)
          : atmosphereLatlonMapAsync(
              params.atmoModel,
              params.altitudeKm,
              effectEpoch,
              nLat,
              nLon,
              params.f107,
              params.ap,
            )
        : magneticFieldLatlonMapAsync(
            params.magModel,
            params.fieldComponent,
            params.altitudeKm,
            effectEpoch,
            nLat,
            nLon,
          );

    fetchData.then((data) => {
      if (cancelled || !data) return;

      const floats = new Float32Array(data.length);
      for (let i = 0; i < data.length; i++) {
        floats[i] = data[i];
      }

      // Fixed range per model/component, not per epoch
      const key =
        layer === "atmosphere"
          ? `atmo:${params.atmoModel}:${params.altitudeKm}:${params.spaceWeatherMode}:${params.f107}:${params.ap}:${nLat}`
          : `mag:${params.magModel}:${params.fieldComponent}:${params.altitudeKm}:${nLat}`;

      if (!rangeRef.current || key !== prevKeyRef.current) {
        let min = Number.POSITIVE_INFINITY;
        let max = Number.NEGATIVE_INFINITY;
        for (let i = 0; i < data.length; i++) {
          if (Number.isFinite(data[i])) {
            if (data[i] < min) min = data[i];
            if (data[i] > max) max = data[i];
          }
        }
        rangeRef.current = { min, max };
        prevKeyRef.current = key;
      }
      setDataRange(rangeRef.current);

      const tex = new THREE.DataTexture(floats, nLon, nLat, THREE.RedFormat, THREE.FloatType);
      tex.needsUpdate = true;
      tex.wrapS = THREE.RepeatWrapping;
      tex.wrapT = THREE.ClampToEdgeWrapping;
      tex.minFilter = THREE.LinearFilter;
      tex.magFilter = THREE.LinearFilter;
      setDataTexture((prev) => {
        prev?.dispose();
        return tex;
      });
    });

    return () => {
      cancelled = true;
    };
  }, [
    layer,
    params.atmoModel,
    params.magModel,
    params.fieldComponent,
    params.altitudeKm,
    params.nLat,
    params.f107,
    params.ap,
    params.spaceWeatherMode,
    effectEpoch,
  ]);

  // Cleanup on unmount via ref (setState during unmount is unreliable)
  const texRef = useRef<THREE.DataTexture | null>(null);
  useEffect(() => {
    texRef.current = dataTexture;
  }, [dataTexture]);
  useEffect(() => {
    return () => {
      texRef.current?.dispose();
    };
  }, []);

  if (!dataTexture) return null;

  const radius = EARTH_RADIUS * (1 + params.altitudeKm / EARTH_RADIUS_KM);
  return (
    <ShellMesh
      dataTexture={dataTexture}
      radius={radius}
      dataMin={dataRange.min}
      dataMax={dataRange.max}
      useLogScale={layer === "atmosphere"}
      opacity={0.7}
    />
  );
}

// ---------------------------------------------------------------------------
// Magnetic volume shells
// ---------------------------------------------------------------------------

function MagneticShells({ params }: { params: ViewerParams }) {
  const [shells, setShells] = useState<ShellData[]>([]);
  const shellRangesRef = useRef<{ min: number; max: number }[] | null>(null);
  const prevRangeKeyRef = useRef("");
  const nLat = Math.min(params.nLat, 45);
  const nLon = nLat * 2;
  const debouncedEpoch = useDebouncedValue(params.epochJd, 1000);

  useEffect(() => {
    let cancelled = false;

    magneticFieldVolumeAsync(
      params.magModel,
      params.fieldComponent,
      100,
      1000,
      N_SHELLS,
      debouncedEpoch,
      nLat,
      nLon,
    ).then((vol) => {
      if (cancelled || !vol) return;

      const sliceSize = nLat * nLon;
      const rangeKey = `${params.magModel}:${params.fieldComponent}`;
      const needNewRanges = !shellRangesRef.current || rangeKey !== prevRangeKeyRef.current;

      const newShells: ShellData[] = [];
      for (let i = 0; i < N_SHELLS; i++) {
        const slice = vol.data.slice(i * sliceSize, (i + 1) * sliceSize);

        let sMin: number;
        let sMax: number;
        if (needNewRanges) {
          sMin = Infinity;
          sMax = -Infinity;
          for (let j = 0; j < slice.length; j++) {
            const v = slice[j];
            if (Number.isFinite(v)) {
              if (v < sMin) sMin = v;
              if (v > sMax) sMax = v;
            }
          }
        } else {
          sMin = shellRangesRef.current![i].min;
          sMax = shellRangesRef.current![i].max;
        }

        const tex = new THREE.DataTexture(slice, nLon, nLat, THREE.RedFormat, THREE.FloatType);
        tex.needsUpdate = true;
        tex.wrapS = THREE.RepeatWrapping;
        tex.wrapT = THREE.ClampToEdgeWrapping;
        tex.minFilter = THREE.LinearFilter;
        tex.magFilter = THREE.LinearFilter;
        newShells.push({ texture: tex, min: sMin, max: sMax });
      }

      if (needNewRanges) {
        shellRangesRef.current = newShells.map((s) => ({ min: s.min, max: s.max }));
        prevRangeKeyRef.current = rangeKey;
      }

      setShells((prev) => {
        for (const s of prev) s.texture.dispose();
        return newShells;
      });
    });

    return () => {
      cancelled = true;
    };
  }, [params.magModel, params.fieldComponent, debouncedEpoch, nLat, nLon]);

  // Cleanup on unmount via ref
  const shellsRef = useRef<ShellData[]>([]);
  useEffect(() => {
    shellsRef.current = shells;
  }, [shells]);
  useEffect(() => {
    return () => {
      for (const s of shellsRef.current) s.texture.dispose();
    };
  }, []);

  if (shells.length === 0) return null;

  return (
    <>
      {shells.map((shell, i) => {
        const alt = 100 + (900 * i) / (N_SHELLS - 1);
        const radius = EARTH_RADIUS * (1 + alt / EARTH_RADIUS_KM);
        const opacity = 0.06 + i * 0.01;
        return (
          <ShellMesh
            // biome-ignore lint/suspicious/noArrayIndexKey: shells have no stable ID
            key={i}
            dataTexture={shell.texture}
            radius={radius}
            dataMin={shell.min}
            dataMax={shell.max}
            useLogScale={false}
            opacity={Math.max(0.02, opacity)}
          />
        );
      })}
    </>
  );
}

// ---------------------------------------------------------------------------
// Field lines (already in ECI coordinates — no pole alignment needed)
// ---------------------------------------------------------------------------

function FieldLines({
  params,
  earthRotation = 0,
  seedAltitude,
}: {
  params: ViewerParams;
  earthRotation?: number;
  seedAltitude?: number;
}) {
  const [lines, setLines] = useState<{ vertices: Float32Array; nPoints: number }[]>([]);
  // GMST at the time field lines were computed (ECI reference frame)
  const [computedGmst, setComputedGmst] = useState(0);
  const debouncedEpoch = useDebouncedValue(params.epochJd, 1000);

  useEffect(() => {
    let cancelled = false;
    const seedLats: number[] = [];
    const seedLons: number[] = [];
    for (let lat = -75; lat <= 75; lat += 15) {
      for (let lon = -180; lon < 180; lon += 30) {
        seedLats.push(lat);
        seedLons.push(lon);
      }
    }

    const alt = seedAltitude ?? params.altitudeKm;
    magneticFieldLinesAsync(
      new Float64Array(seedLats),
      new Float64Array(seedLons),
      alt,
      debouncedEpoch,
      params.magModel,
      500,
      50,
    ).then((raw) => {
      if (cancelled || !raw) return;
      const nLines = raw[0];
      const parsed: { vertices: Float32Array; nPoints: number }[] = [];
      let offset = 1;
      for (let i = 0; i < nLines; i++) {
        const nPts = raw[offset];
        offset++;
        const verts = raw.slice(offset, offset + nPts * 3);
        offset += nPts * 3;
        parsed.push({ vertices: verts, nPoints: nPts });
      }
      setLines(parsed);
      if (isArikaReady()) {
        setComputedGmst(earthRotationAngle(debouncedEpoch));
      }
    });

    return () => {
      cancelled = true;
    };
  }, [params.magModel, seedAltitude, params.altitudeKm, debouncedEpoch]);

  const lineObjects = useMemo(() => {
    return lines
      .filter((line) => line.nPoints >= 2)
      .map((line) => {
        const points: THREE.Vector3[] = [];
        for (let j = 0; j < line.nPoints; j++) {
          // ECI coordinates (Z-up), already in Earth radii
          points.push(
            new THREE.Vector3(
              line.vertices[j * 3],
              line.vertices[j * 3 + 1],
              line.vertices[j * 3 + 2],
            ),
          );
        }
        const geometry = new THREE.BufferGeometry().setFromPoints(points);
        const material = new THREE.LineBasicMaterial({
          color: 0x66aaff,
          transparent: true,
          opacity: 0.4,
        });
        return new THREE.Line(geometry, material);
      });
  }, [lines]);

  // Differential rotation: compensate ECI→ECEF at computation time,
  // then apply current Earth rotation. This makes field lines rotate with Earth.
  const deltaRotation = earthRotation - computedGmst;

  return (
    <group rotation={[0, 0, deltaRotation]}>
      {lineObjects.map((obj, i) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: field lines have no stable ID
        <primitive key={i} object={obj} />
      ))}
    </group>
  );
}

// ---------------------------------------------------------------------------
// Textured Earth sphere
// ---------------------------------------------------------------------------

function EarthSphere() {
  const [texture, setTexture] = useState<THREE.Texture | null>(null);

  useEffect(() => {
    const loader = new THREE.TextureLoader();
    loader.load(`${import.meta.env.BASE_URL}textures/earth_2k.jpg`, (tex) => {
      tex.colorSpace = THREE.SRGBColorSpace;
      setTexture(tex);
    });
  }, []);

  return (
    <group rotation={POLE_ALIGN}>
      <mesh renderOrder={0}>
        <sphereGeometry args={[EARTH_RADIUS, 64, 32]} />
        {texture ? (
          <meshStandardMaterial map={texture} roughness={0.9} metalness={0} />
        ) : (
          <meshPhongMaterial color={0x2244aa} emissive={0x112244} shininess={25} />
        )}
      </mesh>
    </group>
  );
}

// ---------------------------------------------------------------------------
// Main GlobeView
// ---------------------------------------------------------------------------

export function GlobeView({
  params,
  layer,
  earthRotation = 0,
  displayMode = "volume",
}: Props & { earthRotation?: number; displayMode?: "single" | "volume" }) {
  return (
    <div style={{ width: "100%", height: "100%" }}>
      <Canvas camera={{ position: [3, 1.5, 2.5], fov: 45 }} gl={{ alpha: false }}>
        <color attach="background" args={["#060610"]} />
        <ambientLight intensity={0.4} />
        <directionalLight position={[5, 3, 5]} intensity={0.8} />
        {/* ECI (Z-up) → Three.js (Y-up) */}
        <group rotation={ECI_TO_THREEJS}>
          {/* Earth-fixed elements rotate together (ECEF frame) */}
          <group rotation={[0, 0, earthRotation]}>
            <EarthSphere />
            {layer === "atmosphere" && displayMode === "volume" && (
              <AtmosphereShells params={params} />
            )}
            {layer === "atmosphere" && displayMode === "single" && (
              <SingleShell params={params} layer="atmosphere" />
            )}
            {layer === "magnetic" && displayMode === "volume" && <MagneticShells params={params} />}
            {layer === "magnetic" && displayMode === "single" && (
              <SingleShell params={params} layer="magnetic" />
            )}
          </group>
          {/* Field lines in ECI with differential rotation to follow Earth */}
          {layer === "magnetic" && (
            <FieldLines
              params={params}
              earthRotation={earthRotation}
              seedAltitude={displayMode === "volume" ? 400 : params.altitudeKm}
            />
          )}
        </group>
        <OrbitControls enableDamping dampingFactor={0.1} />
      </Canvas>
    </div>
  );
}
