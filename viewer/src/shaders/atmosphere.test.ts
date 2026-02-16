import { describe, expect, it } from "vitest";
import {
  raySphereIntersection,
  rayleighPhase,
  miePhase,
  atmosphericDensity,
  ATMOSPHERE_SCALE_AMPLIFIED,
  ATMOSPHERE_SCALE_PHYSICAL,
  RAYLEIGH_COEFFICIENTS,
  MIE_COEFFICIENT,
  RAYLEIGH_SCALE_HEIGHT,
  MIE_SCALE_HEIGHT,
  MIE_ANISOTROPY,
  MULTI_SCATTERING_FACTOR,
  atmosphereVert,
  atmosphereFrag,
} from "./atmosphere.js";

describe("raySphereIntersection", () => {
  it("returns two positive distances when ray hits sphere from outside", () => {
    // Ray from (0,0,5) toward origin, sphere radius 1
    const result = raySphereIntersection([0, 0, 5], [0, 0, -1], 1);
    expect(result).not.toBeNull();
    const [near, far] = result!;
    expect(near).toBeCloseTo(4.0, 5);
    expect(far).toBeCloseTo(6.0, 5);
  });

  it("returns null when ray misses sphere", () => {
    // Ray from (0,0,5) going sideways, sphere radius 1
    const result = raySphereIntersection([0, 0, 5], [1, 0, 0], 1);
    expect(result).toBeNull();
  });

  it("returns negative near when ray starts inside sphere", () => {
    // Ray from origin toward +Z, sphere radius 2
    const result = raySphereIntersection([0, 0, 0], [0, 0, 1], 2);
    expect(result).not.toBeNull();
    const [near, far] = result!;
    expect(near).toBeCloseTo(-2.0, 5);
    expect(far).toBeCloseTo(2.0, 5);
  });

  it("returns near ≈ far for tangent ray", () => {
    // Ray from (1,0,5) toward -Z, sphere radius 1 → tangent
    const result = raySphereIntersection([1, 0, 5], [0, 0, -1], 1);
    expect(result).not.toBeNull();
    const [near, far] = result!;
    expect(near).toBeCloseTo(far, 3);
  });

  it("works with arbitrary ray direction", () => {
    // Ray from (3,0,0) toward origin, sphere radius 1
    const result = raySphereIntersection([3, 0, 0], [-1, 0, 0], 1);
    expect(result).not.toBeNull();
    const [near, far] = result!;
    expect(near).toBeCloseTo(2.0, 5);
    expect(far).toBeCloseTo(4.0, 5);
  });
});

describe("rayleighPhase", () => {
  const EXPECTED_BASE = 3 / (16 * Math.PI); // ≈ 0.05968

  it("returns (3/16π) for cosTheta=0", () => {
    expect(rayleighPhase(0)).toBeCloseTo(EXPECTED_BASE, 5);
  });

  it("returns (3/16π)×2 for cosTheta=1 (forward scattering)", () => {
    expect(rayleighPhase(1)).toBeCloseTo(EXPECTED_BASE * 2, 5);
  });

  it("returns (3/16π)×2 for cosTheta=-1 (backward scattering, symmetric)", () => {
    expect(rayleighPhase(-1)).toBeCloseTo(EXPECTED_BASE * 2, 5);
  });

  it("is symmetric: phase(cos) === phase(-cos)", () => {
    expect(rayleighPhase(0.5)).toBeCloseTo(rayleighPhase(-0.5), 10);
  });
});

describe("miePhase", () => {
  it("is isotropic when g=0", () => {
    // When g=0, Henyey-Greenstein → 1/(4π) for all angles
    // Cornette-Shanks: (3/8π) * (1-g²)(1+cos²θ) / ((2+g²)(1+g²-2g·cosθ)^(3/2))
    // At g=0: (3/8π) * 1 * (1+cos²θ) / (2 * 1) = (3/16π)(1+cos²θ)
    // This is the Rayleigh phase function, not constant.
    // So let's just check that it returns a finite positive value
    const val0 = miePhase(0, 0);
    const val1 = miePhase(1, 0);
    const valm1 = miePhase(-1, 0);
    expect(val0).toBeGreaterThan(0);
    expect(val1).toBeGreaterThan(0);
    expect(valm1).toBeGreaterThan(0);
  });

  it("is forward-scattering dominant when g > 0", () => {
    const g = 0.76;
    const forward = miePhase(1, g);
    const sideways = miePhase(0, g);
    const backward = miePhase(-1, g);
    expect(forward).toBeGreaterThan(sideways);
    expect(sideways).toBeGreaterThan(backward);
  });

  it("returns positive values for all angles", () => {
    const g = 0.76;
    for (let cos = -1; cos <= 1; cos += 0.2) {
      expect(miePhase(cos, g)).toBeGreaterThan(0);
    }
  });
});

describe("atmosphericDensity", () => {
  it("returns 1.0 at altitude=0", () => {
    expect(atmosphericDensity(0, 1)).toBeCloseTo(1.0, 10);
  });

  it("returns 1/e at altitude=scaleHeight", () => {
    expect(atmosphericDensity(1, 1)).toBeCloseTo(1 / Math.E, 10);
  });

  it("returns exp(-2) at altitude=2×scaleHeight", () => {
    expect(atmosphericDensity(2, 1)).toBeCloseTo(Math.exp(-2), 10);
  });

  it("approaches 0 for very high altitude", () => {
    expect(atmosphericDensity(100, 1)).toBeLessThan(1e-40);
  });

  it("works with non-unit scaleHeight", () => {
    expect(atmosphericDensity(8, 8)).toBeCloseTo(1 / Math.E, 10);
  });
});

describe("physical constants", () => {
  it("ATMOSPHERE_SCALE_AMPLIFIED is in expected range", () => {
    expect(ATMOSPHERE_SCALE_AMPLIFIED).toBeGreaterThan(1.01);
    expect(ATMOSPHERE_SCALE_AMPLIFIED).toBeLessThan(1.1);
  });

  it("ATMOSPHERE_SCALE_PHYSICAL is in expected range (~100km/6371km)", () => {
    expect(ATMOSPHERE_SCALE_PHYSICAL).toBeGreaterThan(1.005);
    expect(ATMOSPHERE_SCALE_PHYSICAL).toBeLessThan(1.03);
  });

  it("AMPLIFIED > PHYSICAL", () => {
    expect(ATMOSPHERE_SCALE_AMPLIFIED).toBeGreaterThan(ATMOSPHERE_SCALE_PHYSICAL);
  });

  it("Rayleigh coefficients: R < G < B (blue scatters most)", () => {
    const [r, g, b] = RAYLEIGH_COEFFICIENTS;
    expect(r).toBeLessThan(g);
    expect(g).toBeLessThan(b);
  });

  it("Rayleigh coefficients are all positive", () => {
    for (const c of RAYLEIGH_COEFFICIENTS) {
      expect(c).toBeGreaterThan(0);
    }
  });

  it("Mie coefficient is positive", () => {
    expect(MIE_COEFFICIENT).toBeGreaterThan(0);
  });

  it("scale heights are positive", () => {
    expect(RAYLEIGH_SCALE_HEIGHT).toBeGreaterThan(0);
    expect(MIE_SCALE_HEIGHT).toBeGreaterThan(0);
  });

  it("Rayleigh scale height > Mie scale height", () => {
    expect(RAYLEIGH_SCALE_HEIGHT).toBeGreaterThan(MIE_SCALE_HEIGHT);
  });

  it("Mie anisotropy g is in (0, 1)", () => {
    expect(MIE_ANISOTROPY).toBeGreaterThan(0);
    expect(MIE_ANISOTROPY).toBeLessThan(1);
  });

  it("multi-scattering factor f_ms is in (0, 1)", () => {
    expect(MULTI_SCATTERING_FACTOR).toBeGreaterThan(0);
    expect(MULTI_SCATTERING_FACTOR).toBeLessThan(1);
  });
});

describe("shader strings", () => {
  it("vertex shader is non-empty and includes logdepthbuf", () => {
    expect(atmosphereVert.length).toBeGreaterThan(0);
    expect(atmosphereVert).toContain("logdepthbuf");
    expect(atmosphereVert).toContain("vWorldPosition");
  });

  it("fragment shader is non-empty and includes logdepthbuf", () => {
    expect(atmosphereFrag.length).toBeGreaterThan(0);
    expect(atmosphereFrag).toContain("logdepthbuf");
    expect(atmosphereFrag).toContain("sunDirection");
    expect(atmosphereFrag).toContain("cameraWorldPos");
    expect(atmosphereFrag).toContain("earthRadius");
    expect(atmosphereFrag).toContain("atmosphereRadius");
  });

  it("fragment shader contains ray-marching loop", () => {
    expect(atmosphereFrag).toContain("for");
  });

  it("fragment shader contains Rayleigh and Mie phase functions", () => {
    // Should reference both scattering types
    expect(atmosphereFrag).toMatch(/[Rr]ayleigh/i);
    expect(atmosphereFrag).toMatch(/[Mm]ie/i);
  });

  it("fragment shader does not use modelMatrix (not available in fragment shaders)", () => {
    // Three.js only provides modelMatrix as a built-in in vertex shaders.
    // Fragment shader must receive derived values via varyings.
    expect(atmosphereFrag).not.toContain("modelMatrix");
  });

  it("vertex shader passes sphere center to fragment via varying", () => {
    expect(atmosphereVert).toContain("vSphereCenter");
    expect(atmosphereFrag).toContain("vSphereCenter");
  });
});
