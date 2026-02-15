/** Vertex shader for Earth day/night blending. */
export const earthDayNightVert = /* glsl */ `
#include <common>
#include <logdepthbuf_pars_vertex>

varying vec2 vUv;
varying vec3 vWorldNormal;

void main() {
  vUv = uv;
  vWorldNormal = normalize((modelMatrix * vec4(normal, 0.0)).xyz);
  gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);

  #include <logdepthbuf_vertex>
}
`;

/** Fragment shader for Earth day/night blending with smooth terminator. */
export const earthDayNightFrag = /* glsl */ `
#include <logdepthbuf_pars_fragment>

uniform sampler2D dayMap;
uniform sampler2D nightMap;
uniform vec3 sunDirection;
uniform float ambientIntensity;

varying vec2 vUv;
varying vec3 vWorldNormal;

void main() {
  vec4 dayColor = texture2D(dayMap, vUv);
  vec4 nightColor = texture2D(nightMap, vUv);

  // Cosine of angle between surface normal and sun direction
  float cosAngle = dot(vWorldNormal, sunDirection);

  // Smooth terminator transition (~18 degree twilight zone)
  float blend = smoothstep(-0.1, 0.2, cosAngle);

  // Day side: Lambertian diffuse with configurable ambient floor
  float diffuse = max(cosAngle, 0.0);
  vec3 litDay = dayColor.rgb * (ambientIntensity + (1.0 - ambientIntensity) * diffuse);

  // Night side: city lights (emissive)
  vec3 litNight = nightColor.rgb;

  vec3 finalColor = mix(litNight, litDay, blend);
  gl_FragColor = vec4(finalColor, 1.0);

  #include <logdepthbuf_fragment>
}
`;
