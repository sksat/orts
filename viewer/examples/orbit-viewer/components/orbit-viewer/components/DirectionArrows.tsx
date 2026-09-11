import { useEffect, useMemo, useRef } from "react";
import type * as THREE from "three";
import { type DrawnArrow, registerDirectionVectors } from "../debug/directionVectors.js";
import { DIRECTION_VECTOR_LABELS, type DirectionVector } from "../directionVectors.js";
import type { Vec3 } from "../displayFrame.js";
import {
  type ArrowGeometry,
  arrowGeometryForSpan,
  arrowLabelDistance,
  arrowLabelHeight,
} from "../spacecraftScale.js";
import { Arrow } from "./Arrow.js";
import { LABEL_RENDER_ORDER, labelTexture } from "./labelTexture.js";

interface DirectionArrowsProps {
  /** Spacecraft position in scene units — where the arrows start. */
  position: Vec3;
  /** Directions in scene axes, as resolved by `resolveDirectionVectors`. */
  vectors: readonly DirectionVector[];
  /** The spacecraft's apparent size in scene units, which sets the proportions. */
  visualSpan: number;
  /**
   * Where an arrow's tail sits, in scene units — the radius of what is drawn, so
   * the arrow does not begin inside it. Defaults to the sphere marker's.
   */
  startRadius?: number;
  /**
   * Optional spacecraft id. When set, the *rendered* directions are published
   * (dev/E2E only) via `window.__debug_get_direction_vectors(id)`.
   */
  debugId?: string;
}

/**
 * Reference-direction arrows drawn at a spacecraft.
 *
 * Drawn by the attitude view, at the origin. Proportions come from the
 * spacecraft's apparent size rather than from a scene length, so the component
 * also works in a scene measured in central-body radii — which is how the orbit
 * view draws them at a centred satellite in a follow-up.
 */
export function DirectionArrows({
  position,
  vectors,
  visualSpan,
  startRadius,
  debugId,
}: DirectionArrowsProps) {
  const geometry = useMemo(
    () => arrowGeometryForSpan(visualSpan, startRadius),
    [visualSpan, startRadius],
  );
  const groupRef = useRef<THREE.Group>(null);
  const headsRef = useRef(new Map<string, THREE.Object3D>());

  // The getter reads the live scene objects, so it only has to be re-registered
  // when the *set* of arrows changes — not when the array holding them is rebuilt,
  // which a scene does on every sample. Keying the effect on the kinds (and
  // reading the array through a ref) keeps it to one registration.
  const vectorsRef = useRef(vectors);
  vectorsRef.current = vectors;
  const kinds = vectors.map((v) => v.kind).join(",");
  useEffect(() => {
    if (debugId == null) return;
    const heads = headsRef.current;
    return registerDirectionVectors(debugId, () =>
      vectorsRef.current.map<DrawnArrow>((v) => ({
        kind: v.kind,
        origin: groupRef.current,
        head: heads.get(v.kind) ?? null,
      })),
    );
  }, [debugId, kinds]);

  if (vectors.length === 0) return null;

  return (
    <group ref={groupRef} position={position}>
      {vectors.map((v) => (
        <Arrow
          key={v.kind}
          direction={v.direction}
          color={v.color}
          headRef={(mesh) => {
            if (mesh == null) headsRef.current.delete(v.kind);
            else headsRef.current.set(v.kind, mesh);
          }}
          {...geometry}
        />
      ))}
      <ArrowLabels vectors={vectors} geometry={geometry} span={visualSpan} />
    </group>
  );
}

/**
 * Each arrow's name, at its tip.
 *
 * The alternative is a legend in the overlay, which asks the reader to hold a
 * colour in mind and match it against the picture — and leaves a still image
 * with nothing to go on. The axis triads name themselves for the same reason,
 * and these read the same way: a sprite past the tip, coloured like the arrow it
 * belongs to, drawn last and without a depth test so a name on an arrow pointing
 * away stays legible.
 */
function ArrowLabels({
  vectors,
  geometry,
  span,
}: {
  vectors: readonly DirectionVector[];
  geometry: ArrowGeometry;
  span: number;
}) {
  const labels = useMemo(() => {
    const distance = arrowLabelDistance(geometry);
    const height = arrowLabelHeight(span);
    return vectors.map((v) => {
      const label = labelTexture(DIRECTION_VECTOR_LABELS[v.kind], v.color);
      return {
        kind: v.kind,
        texture: label.texture,
        position: [
          v.direction[0] * distance,
          v.direction[1] * distance,
          v.direction[2] * distance,
        ] as [number, number, number],
        scale: [height * label.aspect, height, height] as [number, number, number],
      };
    });
  }, [vectors, geometry, span]);

  return (
    <>
      {labels.map((label) => (
        <sprite
          key={label.kind}
          position={label.position}
          scale={label.scale}
          renderOrder={LABEL_RENDER_ORDER}
        >
          <spriteMaterial
            map={label.texture}
            transparent
            depthTest={false}
            depthWrite={false}
            toneMapped={false}
          />
        </sprite>
      ))}
    </>
  );
}
