import { useMemo } from "react";
import { AXIS_COLORS, AXIS_LETTERS, axisLabelPositions, axisLabelScale } from "../axisTriad.js";
import { LABEL_RENDER_ORDER, labelTexture } from "./labelTexture.js";

interface AxisLabelsProps {
  /** Axis length in scene units — the labels sit just past each tip. */
  length: number;
  /** Matches the axes' own opacity, so a subordinate triad gets fainter letters. */
  opacity?: number;
}

/**
 * X / Y / Z at the tips of an axis triad.
 *
 * Sprites rather than 3D text: they face the camera from every angle, need no
 * font asset (the letters are drawn with the browser's own canvas text), and
 * carry no dependency. Placed inside the triad's group, so a label follows the
 * axis it names when the group rotates — a sprite ignores inherited *rotation*
 * but not inherited position.
 *
 * The colours alone leave a reader to recall the convention and work out which
 * line is which; on a still image of two triads there is nothing to work from.
 * The letters say it outright.
 */
export function AxisLabels({ length, opacity = 1 }: AxisLabelsProps) {
  const labels = useMemo(() => {
    const positions = axisLabelPositions(length);
    return AXIS_LETTERS.map((letter, i) => {
      const label = labelTexture(letter, AXIS_COLORS[i]);
      return { letter, label, position: positions[i] };
    });
  }, [length]);
  const scale = axisLabelScale(length);

  return (
    <>
      {labels.map((label) => (
        <sprite
          key={label.letter}
          position={label.position}
          // The texture is as wide as the letter needs, so a square sprite would
          // stretch it: a bold "X" measures narrower than the texture is tall.
          scale={[scale * label.label.aspect, scale, scale]}
          // Drawn last and without a depth test, so a letter on an axis pointing
          // away from the camera stays readable instead of hiding inside the
          // spacecraft. The axis *arrow* is still depth-tested, so it disappearing
          // into the body is what tells the reader that axis points away.
          renderOrder={LABEL_RENDER_ORDER}
        >
          <spriteMaterial
            map={label.label.texture}
            transparent
            opacity={opacity}
            depthTest={false}
            depthWrite={false}
            toneMapped={false}
          />
        </sprite>
      ))}
    </>
  );
}
