/**
 * Shader for overlaying data values on a globe via a DataTexture.
 *
 * The data texture stores scalar values in the R channel.
 * The shader maps these to a viridis-like color scale.
 */

export const overlayVert = /* glsl */ `
varying vec2 vUv;
void main() {
  vUv = uv;
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
}
`;

export const overlayFrag = /* glsl */ `
uniform sampler2D dataMap;
uniform float dataMin;
uniform float dataMax;
uniform float opacity;
uniform bool useLogScale;

varying vec2 vUv;

// Blue → Cyan → Green → Yellow → Red (high contrast, wide hue range)
vec3 viridis(float t) {
  const vec3 c0 = vec3(0.10, 0.20, 0.65);
  const vec3 c1 = vec3(0.10, 0.45, 0.80);
  const vec3 c2 = vec3(0.10, 0.70, 0.70);
  const vec3 c3 = vec3(0.15, 0.82, 0.45);
  const vec3 c4 = vec3(0.50, 0.88, 0.25);
  const vec3 c5 = vec3(0.85, 0.85, 0.15);
  const vec3 c6 = vec3(1.00, 0.60, 0.10);
  const vec3 c7 = vec3(1.00, 0.25, 0.10);

  float s = clamp(t, 0.0, 1.0) * 7.0;
  int idx = int(floor(s));
  float f = fract(s);

  if (idx == 0) return mix(c0, c1, f);
  if (idx == 1) return mix(c1, c2, f);
  if (idx == 2) return mix(c2, c3, f);
  if (idx == 3) return mix(c3, c4, f);
  if (idx == 4) return mix(c4, c5, f);
  if (idx == 5) return mix(c5, c6, f);
  return mix(c6, c7, f);
}

void main() {
  float raw = texture2D(dataMap, vUv).r;
  float norm;
  if (useLogScale) {
    float logMin = log(max(dataMin, 1e-30));
    float logMax = log(max(dataMax, 1e-29));
    norm = (log(max(raw, 1e-30)) - logMin) / max(logMax - logMin, 1e-10);
  } else {
    norm = (raw - dataMin) / max(dataMax - dataMin, 1e-10);
  }
  norm = clamp(norm, 0.0, 1.0);
  vec3 color = viridis(norm);
  gl_FragColor = vec4(color, opacity);
}
`;
