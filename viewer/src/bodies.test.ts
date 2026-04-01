import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { entityPathToBodyId, getBodyRadius } from "./bodies.js";

describe("entityPathToBodyId", () => {
  it("returns null for satellite paths", () => {
    expect(entityPathToBodyId("/world/sat/iss")).toBeNull();
    expect(entityPathToBodyId("/world/sat/apollo11")).toBeNull();
  });

  it("returns body id for known body paths", () => {
    expect(entityPathToBodyId("/world/moon")).toBe("moon");
    expect(entityPathToBodyId("/world/sun")).toBe("sun");
    expect(entityPathToBodyId("/world/mars")).toBe("mars");
    expect(entityPathToBodyId("/world/earth")).toBe("earth");
  });

  it("returns null for unknown body paths", () => {
    expect(entityPathToBodyId("/world/pluto")).toBeNull();
  });
});

describe("getBodyRadius", () => {
  it("returns radius for known bodies", () => {
    expect(getBodyRadius("moon")).toBeCloseTo(1737.4, 0);
    expect(getBodyRadius("earth")).toBeCloseTo(6378.137, 0);
  });

  it("returns null for unknown bodies", () => {
    expect(getBodyRadius("pluto")).toBeNull();
  });
});

/**
 * Verify that the IAU body orientation + pole alignment pipeline
 * correctly maps the Three.js sphere texture center to the expected
 * direction in ECI space.
 *
 * Three.js SphereGeometry UV mapping:
 *   U=0.0 (left edge)   → -X direction at equator
 *   U=0.5 (center)      → +X direction at equator
 *   V=0.0 (top)         → +Y (Three.js north pole)
 *
 * Standard equirectangular Moon texture:
 *   0° longitude (sub-Earth point) → image center (U=0.5) → +X
 *
 * After pole alignment Rx(π/2): +Y→+Z, +X stays +X
 * After IAU quaternion: body-fixed +X (prime meridian) → ECI
 *
 * For Moon: prime meridian should point roughly toward Earth.
 */
describe("Moon texture orientation pipeline", () => {
  // IAU 2009 Moon rotation model (same constants as kaname/src/rotation.rs)
  const MOON_IAU = {
    alpha0: 269.9949,
    alpha1: 0.0031,
    delta0: 66.5392,
    delta1: 0.013,
    w0: 38.3213,
    wd: 13.17635815,
  };

  function computeIauQuaternion(model: typeof MOON_IAU, jd: number): THREE.Quaternion {
    const d = jd - 2451545.0;
    const T = d / 36525.0;
    const DEG = Math.PI / 180;

    const alpha = (model.alpha0 + model.alpha1 * T) * DEG;
    const delta = (model.delta0 + model.delta1 * T) * DEG;
    const W = (model.w0 + model.wd * d) * DEG;

    // Z_body = pole direction in ECI
    const zBody = new THREE.Vector3(
      Math.cos(alpha) * Math.cos(delta),
      Math.sin(alpha) * Math.cos(delta),
      Math.sin(delta),
    );

    // Node direction (ascending node of body equator on ECI equator)
    const node = new THREE.Vector3(-Math.sin(alpha), Math.cos(alpha), 0);

    // m = z_body × node
    const m = new THREE.Vector3().crossVectors(zBody, node);

    // X_body = node * cos(W) + m * sin(W)
    const xBody = node.clone().multiplyScalar(Math.cos(W)).addScaledVector(m, Math.sin(W));

    // Y_body = Z × X
    const yBody = new THREE.Vector3().crossVectors(zBody, xBody);

    // Rotation matrix: columns = body axes in ECI
    const mat = new THREE.Matrix4().makeBasis(xBody, yBody, zBody);
    return new THREE.Quaternion().setFromRotationMatrix(mat);
  }

  /** Apply the same transform chain as SecondaryBody in Scene.tsx */
  function applyFullPipeline(iauQuat: THREE.Quaternion): THREE.Quaternion {
    const poleAlign = new THREE.Quaternion().setFromEuler(new THREE.Euler(Math.PI / 2, 0, 0));
    return iauQuat.clone().multiply(poleAlign);
  }

  it("Three.js sphere UV seam is at -X direction", () => {
    // Verify our understanding of Three.js SphereGeometry:
    // U=0 at equator should be at -X
    const geo = new THREE.SphereGeometry(1, 32, 16);
    const uv = geo.getAttribute("uv");
    const pos = geo.getAttribute("position");

    // Find equatorial vertices (y ≈ 0) with U near 0
    let seam: THREE.Vector3 | null = null;
    for (let i = 0; i < uv.count; i++) {
      const u = uv.getX(i);
      const y = pos.getY(i);
      if (Math.abs(y) < 0.1 && u < 0.02) {
        seam = new THREE.Vector3(pos.getX(i), pos.getY(i), pos.getZ(i));
        break;
      }
    }

    expect(seam).not.toBeNull();
    // Seam should be near -X direction
    expect(seam!.x).toBeLessThan(-0.9);
    expect(Math.abs(seam!.z)).toBeLessThan(0.2);
  });

  it("Three.js sphere texture center (U=0.5) is at +X direction", () => {
    const geo = new THREE.SphereGeometry(1, 32, 16);
    const uv = geo.getAttribute("uv");
    const pos = geo.getAttribute("position");

    let center: THREE.Vector3 | null = null;
    for (let i = 0; i < uv.count; i++) {
      const u = uv.getX(i);
      const y = pos.getY(i);
      if (Math.abs(y) < 0.1 && Math.abs(u - 0.5) < 0.02) {
        center = new THREE.Vector3(pos.getX(i), pos.getY(i), pos.getZ(i));
        break;
      }
    }

    expect(center).not.toBeNull();
    // Center should be near +X
    expect(center!.x).toBeGreaterThan(0.9);
    expect(Math.abs(center!.z)).toBeLessThan(0.2);
  });

  it("pole alignment maps +Y to +Z and keeps +X unchanged", () => {
    const poleAlign = new THREE.Quaternion().setFromEuler(new THREE.Euler(Math.PI / 2, 0, 0));

    const y = new THREE.Vector3(0, 1, 0).applyQuaternion(poleAlign);
    expect(y.z).toBeCloseTo(1, 5);
    expect(Math.abs(y.x)).toBeLessThan(1e-10);
    expect(Math.abs(y.y)).toBeLessThan(1e-10);

    const x = new THREE.Vector3(1, 0, 0).applyQuaternion(poleAlign);
    expect(x.x).toBeCloseTo(1, 5);
  });

  it("full pipeline: Moon texture center faces Earth at Apollo 11 epoch", () => {
    // Apollo 11 mission end: ~1969-07-24T22:40:00Z
    // epoch_jd = 2440418.064, elapsed = 723374s
    const jd = 2440418.064 + 723374 / 86400;

    const iauQuat = computeIauQuaternion(MOON_IAU, jd);
    const combined = applyFullPipeline(iauQuat);

    // Texture center (+X in Three.js local) after full pipeline
    const textureCenter = new THREE.Vector3(1, 0, 0).applyQuaternion(combined);

    // Moon position at this time (from replay data)
    const moonPos = new THREE.Vector3(-164710, -287813, -158607);
    const earthDir = moonPos.clone().negate().normalize();

    // Angle between texture center and Earth direction
    const angle = textureCenter.angleTo(earthDir) * (180 / Math.PI);

    // Should be within ~15° (IAU model without libration)
    expect(angle).toBeLessThan(15);
  });

  it("full pipeline: UV seam faces away from Earth", () => {
    const jd = 2440418.064 + 723374 / 86400;

    const iauQuat = computeIauQuaternion(MOON_IAU, jd);
    const combined = applyFullPipeline(iauQuat);

    // UV seam (-X in Three.js local) after full pipeline
    const seamDir = new THREE.Vector3(-1, 0, 0).applyQuaternion(combined);

    // Moon position
    const moonPos = new THREE.Vector3(-164710, -287813, -158607);
    const earthDir = moonPos.clone().negate().normalize();

    // Seam should face AWAY from Earth (angle > 165°)
    const angle = seamDir.angleTo(earthDir) * (180 / Math.PI);
    expect(angle).toBeGreaterThan(165);
  });

  it("full pipeline: north pole points roughly toward ecliptic normal", () => {
    const jd = 2440418.064 + 723374 / 86400;

    const iauQuat = computeIauQuaternion(MOON_IAU, jd);
    const combined = applyFullPipeline(iauQuat);

    // Three.js sphere north pole is +Y; after pipeline it should point
    // roughly toward the Moon's north pole in ECI (near ecliptic normal)
    const pole = new THREE.Vector3(0, 1, 0).applyQuaternion(combined);

    // Ecliptic normal in ECI: (0, -sin(23.44°), cos(23.44°))
    const obliquity = (23.44 * Math.PI) / 180;
    const eclNormal = new THREE.Vector3(0, -Math.sin(obliquity), Math.cos(obliquity));

    const angle = pole.angleTo(eclNormal) * (180 / Math.PI);
    // Moon's pole is ~1.54° from ecliptic normal, but IAU model
    // approximation allows up to ~10°
    expect(angle).toBeLessThan(10);
  });
});
