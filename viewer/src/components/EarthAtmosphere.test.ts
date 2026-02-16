import { describe, it, expect } from "vitest";
import {
  ATMOSPHERE_SCALE_AMPLIFIED,
  ATMOSPHERE_SCALE_PHYSICAL,
  atmosphereVert,
  atmosphereFrag,
} from "../shaders/atmosphere.js";
import {
  createAtmosphereMaterial,
  ATMO_SEGMENTS,
  getAtmosphereRadius,
} from "./EarthAtmosphere.js";
import * as THREE from "three";

describe("createAtmosphereMaterial", () => {
  it("returns a ShaderMaterial with additive blending", () => {
    const mat = createAtmosphereMaterial();
    expect(mat).toBeInstanceOf(THREE.ShaderMaterial);
    expect(mat.blending).toBe(THREE.AdditiveBlending);
  });

  it("renders BackSide only", () => {
    const mat = createAtmosphereMaterial();
    expect(mat.side).toBe(THREE.BackSide);
  });

  it("is transparent and does not write depth", () => {
    const mat = createAtmosphereMaterial();
    expect(mat.transparent).toBe(true);
    expect(mat.depthWrite).toBe(false);
    expect(mat.depthTest).toBe(true);
  });

  it("uses atmosphere vertex and fragment shaders", () => {
    const mat = createAtmosphereMaterial();
    expect(mat.vertexShader).toBe(atmosphereVert);
    expect(mat.fragmentShader).toBe(atmosphereFrag);
  });

  it("has required uniforms", () => {
    const mat = createAtmosphereMaterial();
    expect(mat.uniforms.sunDirection).toBeDefined();
    expect(mat.uniforms.sunIntensity).toBeDefined();
    expect(mat.uniforms.cameraWorldPos).toBeDefined();
    expect(mat.uniforms.earthRadius).toBeDefined();
    expect(mat.uniforms.atmosphereRadius).toBeDefined();
  });

  it("has correct default uniform values", () => {
    const mat = createAtmosphereMaterial();
    expect(mat.uniforms.sunIntensity.value).toBe(1.0);
    expect(mat.uniforms.earthRadius.value).toBe(1.0);
  });
});

describe("getAtmosphereRadius", () => {
  it("returns amplified radius when physicalScale is false", () => {
    const r = getAtmosphereRadius(1.0, false);
    expect(r).toBeCloseTo(ATMOSPHERE_SCALE_AMPLIFIED, 5);
  });

  it("returns physical radius when physicalScale is true", () => {
    const r = getAtmosphereRadius(1.0, true);
    expect(r).toBeCloseTo(ATMOSPHERE_SCALE_PHYSICAL, 5);
  });

  it("scales with radius parameter", () => {
    const radius = 3.5;
    const r = getAtmosphereRadius(radius, false);
    expect(r).toBeCloseTo(radius * ATMOSPHERE_SCALE_AMPLIFIED, 5);
  });

  it("physical scale uses smaller radius than amplified", () => {
    const physical = getAtmosphereRadius(1.0, true);
    const amplified = getAtmosphereRadius(1.0, false);
    expect(physical).toBeLessThan(amplified);
  });
});

describe("ATMO_SEGMENTS", () => {
  it("is a reasonable segment count for a sphere", () => {
    expect(ATMO_SEGMENTS).toBeGreaterThanOrEqual(24);
    expect(ATMO_SEGMENTS).toBeLessThanOrEqual(96);
  });
});
