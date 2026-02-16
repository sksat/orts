/**
 * Atmospheric scattering shader for rendering Earth's atmosphere from space.
 *
 * Based on single-pass ray-marching with Rayleigh + Mie scattering,
 * incorporating Hillaire (EGSR 2020) multi-scattering approximation.
 *
 * References:
 * - glsl-atmosphere (wwwtyro): https://github.com/wwwtyro/glsl-atmosphere
 * - atmospheric-scattering-explained (Dimev): https://github.com/Dimev/atmospheric-scattering-explained
 * - Hillaire, "A Scalable and Production Ready Sky and Atmosphere Rendering Technique" (EGSR 2020)
 */

// ── Physical constants ──────────────────────────────────────────────

/** Amplified atmosphere scale for body-centered (distant) view. */
export const ATMOSPHERE_SCALE_AMPLIFIED = 1.06;

/** Physical atmosphere scale (~100km Kármán line / 6371km Earth radius). */
export const ATMOSPHERE_SCALE_PHYSICAL = 1.015;

/**
 * Rayleigh scattering coefficients [m⁻¹] for R, G, B channels.
 * Blue scatters most (λ⁻⁴ law).
 */
export const RAYLEIGH_COEFFICIENTS: [number, number, number] = [5.5e-6, 13.0e-6, 22.4e-6];

/** Mie scattering coefficient [m⁻¹] (wavelength-independent). */
export const MIE_COEFFICIENT = 21e-6;

/** Rayleigh scale height in Earth-radius units (8.0 km / 6371 km). */
export const RAYLEIGH_SCALE_HEIGHT = 8.0 / 6371.0;

/** Mie scale height in Earth-radius units (1.2 km / 6371 km). */
export const MIE_SCALE_HEIGHT = 1.2 / 6371.0;

/** Mie asymmetry parameter g (forward-scattering dominant). */
export const MIE_ANISOTROPY = 0.76;

/** Multi-scattering factor f_ms (Hillaire approximation). */
export const MULTI_SCATTERING_FACTOR = 0.25;

/** Airglow emission color (blue-dominant chemiluminescence at ~90-300 km). */
export const AIRGLOW_COLOR: [number, number, number] = [0.1, 0.25, 0.5];

/** Airglow emission strength factor. */
export const AIRGLOW_STRENGTH = 2.0;

// ── Math utility functions (also used in GLSL) ─────────────────────

/**
 * Ray-sphere intersection for a sphere centered at the origin.
 * Returns [near, far] distances along the ray, or null if no intersection.
 * near < 0 means the ray origin is inside the sphere.
 */
export function raySphereIntersection(
  origin: [number, number, number],
  dir: [number, number, number],
  radius: number,
): [number, number] | null {
  // Solve: |origin + t*dir|² = radius²
  // → (dir·dir)t² + 2(origin·dir)t + (origin·origin - radius²) = 0
  const a = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
  const b = 2 * (origin[0] * dir[0] + origin[1] * dir[1] + origin[2] * dir[2]);
  const c = origin[0] * origin[0] + origin[1] * origin[1] + origin[2] * origin[2] - radius * radius;

  const discriminant = b * b - 4 * a * c;
  if (discriminant < 0) return null;

  const sqrtD = Math.sqrt(discriminant);
  const near = (-b - sqrtD) / (2 * a);
  const far = (-b + sqrtD) / (2 * a);
  return [near, far];
}

/**
 * Rayleigh phase function: (3/16π)(1 + cos²θ).
 * Symmetric for forward and backward scattering.
 */
export function rayleighPhase(cosTheta: number): number {
  return (3 / (16 * Math.PI)) * (1 + cosTheta * cosTheta);
}

/**
 * Cornette-Shanks phase function (improved Henyey-Greenstein for Mie scattering).
 * (3/8π) × (1 - g²)(1 + cos²θ) / ((2 + g²)(1 + g² - 2g·cosθ)^(3/2))
 */
export function miePhase(cosTheta: number, g: number): number {
  const gg = g * g;
  const num = (3 / (8 * Math.PI)) * (1 - gg) * (1 + cosTheta * cosTheta);
  const denom = (2 + gg) * Math.pow(1 + gg - 2 * g * cosTheta, 1.5);
  return num / denom;
}

/**
 * Atmospheric density at a given altitude: exp(-altitude / scaleHeight).
 */
export function atmosphericDensity(altitude: number, scaleHeight: number): number {
  return Math.exp(-altitude / scaleHeight);
}

// ── GLSL Shaders ────────────────────────────────────────────────────

/** Vertex shader for the atmosphere shell. */
export const atmosphereVert = /* glsl */ `
#include <common>
#include <logdepthbuf_pars_vertex>

varying vec3 vWorldPosition;
varying vec3 vSphereCenter;

void main() {
  vec4 worldPos = modelMatrix * vec4(position, 1.0);
  vWorldPosition = worldPos.xyz;
  vSphereCenter = (modelMatrix * vec4(0.0, 0.0, 0.0, 1.0)).xyz;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);

  #include <logdepthbuf_vertex>
}
`;

/** Fragment shader for atmospheric scattering with ray-marching. */
export const atmosphereFrag = /* glsl */ `
#include <logdepthbuf_pars_fragment>

uniform vec3 sunDirection;      // normalized, world space
uniform float sunIntensity;     // inverse-square factor: (1 AU / distance)²
uniform vec3 cameraWorldPos;    // camera position in world space
uniform float earthRadius;      // scene units (1.0 for body-centered)
uniform float atmosphereRadius; // scene units (earthRadius * scale)

varying vec3 vWorldPosition;
varying vec3 vSphereCenter;

// ── Physical constants (all in Earth-radius units, earthRadius = 1) ──
//
// The shader normalizes all distances by earthRadius at the start of main(),
// so these constants work correctly regardless of scene scale.

// Rayleigh scattering coefficients per Earth-radius [R_E⁻¹].
// Physical [m⁻¹] × 6.371e6 [m/R_E] → [R_E⁻¹].
const vec3 RAY_BETA = vec3(5.5e-6, 13.0e-6, 22.4e-6) * 6.371e6;

// Mie scattering coefficient per Earth-radius [R_E⁻¹].
const vec3 MIE_BETA = vec3(21e-6) * 6.371e6;

// Mie asymmetry (forward-scattering dominant).
const float G_MIE = 0.76;

// Multi-scattering approximation factor (Hillaire 2020).
const float F_MS = 0.25;

// Scale heights in Earth-radius units.
const float HEIGHT_RAY = 0.001256;  // Rayleigh: 8.0 km / 6371 km
const float HEIGHT_MIE = 0.000188;  // Mie: 1.2 km / 6371 km
const float HEIGHT_GLOW = 0.01;     // Airglow: ~64 km (broad emission layer)

// Number of ray-march steps.
const int I_STEPS = 16;  // primary ray (view direction)
const int J_STEPS = 8;   // secondary ray (toward sun)

// Sun intensity multiplier for visual tuning.
const float SUN_INTENSITY_SCALE = 20.0;

// Airglow: faint atmospheric self-emission (chemiluminescence at ~90-300 km).
// Visible from orbit as thin blue band on night-side limb.
const vec3 AIRGLOW_COLOR = vec3(0.1, 0.25, 0.5);
const float AIRGLOW_STRENGTH = 2.0;

// ── Ray-sphere intersection ──

// Returns (near, far) distances. near > far means no intersection.
vec2 rsi(vec3 r0, vec3 rd, float sr) {
  float a = dot(rd, rd);
  float b = 2.0 * dot(rd, r0);
  float c = dot(r0, r0) - sr * sr;
  float d = b * b - 4.0 * a * c;
  if (d < 0.0) return vec2(1e5, -1e5);
  float sd = sqrt(d);
  return vec2((-b - sd) / (2.0 * a), (-b + sd) / (2.0 * a));
}

// ── Phase functions ──

// Rayleigh phase: (3/16π)(1 + cos²θ)
float phaseRayleigh(float cosTheta) {
  return 3.0 / (50.2654824574) * (1.0 + cosTheta * cosTheta);
  // 50.2654824574 = 16π
}

// Cornette-Shanks (Mie) phase function.
float phaseMie(float cosTheta, float g) {
  float gg = g * g;
  float num = 3.0 * (1.0 - gg) * (1.0 + cosTheta * cosTheta);
  float denom = (8.0 * 3.14159265) * (2.0 + gg) * pow(1.0 + gg - 2.0 * g * cosTheta, 1.5);
  return num / denom;
}

void main() {
  // ── Normalize to Earth-radius units ──
  // All constants (RAY_BETA, HEIGHT_RAY, etc.) assume earthRadius = 1.
  // Dividing by earthRadius makes the shader scale-invariant.
  vec3 center = vSphereCenter;
  float R = earthRadius;   // scene-unit Earth radius (may be 1 or ~200)
  float invR = 1.0 / R;

  // Ray origin and direction in normalized units (planet center = origin, radius = 1).
  vec3 r0 = (cameraWorldPos - center) * invR;
  vec3 rd = normalize(vWorldPosition - cameraWorldPos);  // direction is scale-invariant

  float nAtmoRadius = atmosphereRadius * invR;  // e.g. 1.06

  // Intersect view ray with atmosphere and planet spheres (normalized).
  vec2 atmoHit = rsi(r0, rd, nAtmoRadius);
  vec2 planetHit = rsi(r0, rd, 1.0);

  // No atmosphere intersection → fully transparent.
  if (atmoHit.x > atmoHit.y) {
    gl_FragColor = vec4(0.0);
    #include <logdepthbuf_fragment>
    return;
  }

  // Clamp ray start to atmosphere entry (max with 0 for camera inside atmo).
  float tStart = max(atmoHit.x, 0.0);
  float tEnd = atmoHit.y;

  // If ray hits planet, end at planet surface.
  if (planetHit.x < planetHit.y && planetHit.x > 0.0) {
    tEnd = min(tEnd, planetHit.x);
  }

  // Step size along view ray (in normalized units).
  float iStepSize = (tEnd - tStart) / float(I_STEPS);

  // Accumulated scattering and optical depth.
  vec3 totalRay = vec3(0.0);
  vec3 totalMie = vec3(0.0);
  float iOptRay = 0.0;
  float iOptMie = 0.0;
  float totalGlow = 0.0;

  // Phase function values (constant along the ray).
  float mu = dot(rd, sunDirection);
  float pRay = phaseRayleigh(mu);
  float pMie = phaseMie(mu, G_MIE);

  // ── Primary ray-march (all in normalized Earth-radius units) ──
  for (int i = 0; i < I_STEPS; i++) {
    // Sample point at center of step.
    vec3 iPos = r0 + rd * (tStart + iStepSize * (float(i) + 0.5));
    float iHeight = length(iPos) - 1.0;  // altitude above surface in R_E

    // Density at this sample point.
    float dRay = exp(-iHeight / HEIGHT_RAY) * iStepSize;
    float dMie = exp(-iHeight / HEIGHT_MIE) * iStepSize;

    // Accumulate optical depth along view ray.
    iOptRay += dRay;
    iOptMie += dMie;
    // Airglow: broad emission layer (~90-300 km altitude range).
    totalGlow += exp(-iHeight / HEIGHT_GLOW) * iStepSize;

    // ── Secondary ray-march toward sun ──
    float jStepSize = rsi(iPos, sunDirection, nAtmoRadius).y / float(J_STEPS);
    float jOptRay = 0.0;
    float jOptMie = 0.0;

    for (int j = 0; j < J_STEPS; j++) {
      vec3 jPos = iPos + sunDirection * (jStepSize * (float(j) + 0.5));
      float jHeight = length(jPos) - 1.0;
      jOptRay += exp(-jHeight / HEIGHT_RAY) * jStepSize;
      jOptMie += exp(-jHeight / HEIGHT_MIE) * jStepSize;
    }

    // Transmittance from sun through atmosphere to this point, then to camera.
    vec3 attn = exp(-(RAY_BETA * (iOptRay + jOptRay) + MIE_BETA * (iOptMie + jOptMie)));

    // Single-scattering contribution.
    totalRay += dRay * attn;
    totalMie += dMie * attn;
  }

  // Combine single-scattering with phase functions.
  vec3 singleScatter = (pRay * RAY_BETA * totalRay + pMie * MIE_BETA * totalMie);

  // Multi-scattering approximation (Hillaire 2020):
  // Approximate higher-order scattering as isotropic, scaled by geometric series.
  // L_ms ≈ L_isotropic / (1 - f_ms)
  float isoPhase = 1.0 / (4.0 * 3.14159265);
  vec3 multiScatter = (isoPhase * RAY_BETA * totalRay + isoPhase * MIE_BETA * totalMie)
                      * F_MS / (1.0 - F_MS);

  // Airglow: sun-independent emission visible on night-side limb.
  vec3 airglow = AIRGLOW_COLOR * AIRGLOW_STRENGTH * totalGlow;
  vec3 color = (singleScatter + multiScatter) * sunIntensity * SUN_INTENSITY_SCALE + airglow;

  // With additive blending, color itself controls contribution intensity.
  // Using alpha=1.0 avoids quadratic dimming (faint scatter × faint alpha)
  // that would make thin physical-scale atmospheres invisible.
  float alpha = 1.0;

  gl_FragColor = vec4(color, alpha);

  #include <tonemapping_fragment>
  #include <colorspace_fragment>
  #include <logdepthbuf_fragment>
}
`;
