/**
 * Orbit trail vertex/fragment shaders with high/low split precision.
 *
 * Uses Cesium-style 2^16 chunk encoding to preserve f64-level precision
 * in float32 vertex attributes. The shader subtracts origin and applies
 * frame rotation entirely on the GPU, making mode switches (ECI, ECEF,
 * LVLH, etc.) a uniform-only update — O(1) per frame regardless of
 * point count.
 */

/** Vertex shader for orbit trail with high/low split precision. */
export const orbitTrailVert = /* glsl */ `
#include <common>
#include <logdepthbuf_pars_vertex>

// High/low split position attributes (km, pre-split on CPU)
attribute vec3 positionHigh;
attribute vec3 positionLow;

// Origin in high/low split (satellite ECI km, or [0,0,0] for central-body)
uniform vec3 uOriginHigh;
uniform vec3 uOriginLow;

// Frame rotation matrix (identity for ECI/ECEF, LVLH rotation for body-frame)
uniform mat3 uFrameRotation;

// Inverse of scaleRadius: 1.0 / centralBodyRadius [1/km]
uniform float uInvScaleRadius;

void main() {
  // Reconstruct relative position in km with high precision.
  // The high-part subtraction cancels most magnitude, leaving a small
  // value that the low-part subtraction refines.
  vec3 relativeKm = (positionHigh - uOriginHigh) + (positionLow - uOriginLow);

  // Apply frame rotation (identity for ECI/ECEF, LVLH for body-frame)
  vec3 frameKm = uFrameRotation * relativeKm;

  // Convert from km to scene units
  vec3 scenePosition = frameKm * uInvScaleRadius;

  vec4 mvPosition = modelViewMatrix * vec4(scenePosition, 1.0);
  gl_Position = projectionMatrix * mvPosition;

  #include <logdepthbuf_vertex>
}
`;

/** Fragment shader for orbit trail. */
export const orbitTrailFrag = /* glsl */ `
#include <logdepthbuf_pars_fragment>

uniform vec3 uColor;
uniform float uOpacity;

void main() {
  gl_FragColor = vec4(uColor, uOpacity);

  #include <tonemapping_fragment>
  #include <colorspace_fragment>
  #include <logdepthbuf_fragment>
}
`;
