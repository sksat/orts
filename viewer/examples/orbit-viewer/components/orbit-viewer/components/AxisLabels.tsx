import { useMemo } from "react";
import * as THREE from "three";
import {
  AXIS_COLORS,
  AXIS_LETTERS,
  axisColorCss,
  axisLabelPositions,
  axisLabelScale,
} from "../axisTriad.js";

/**
 * One canvas texture per letter, made on first use and kept.
 *
 * Lazily, not at module scope: the module is imported by tests running without a
 * DOM, and a canvas at import time would throw there.
 */
const textureCache = new Map<string, THREE.Texture>();

/** Above the scene's meshes, which use the default 0. */
const LABEL_RENDER_ORDER = 10;

const TEXTURE_SIZE = 128;

function letterTexture(letter: string, color: number): THREE.Texture {
  const key = `${letter}:${color}`;
  const cached = textureCache.get(key);
  if (cached) return cached;

  const canvas = document.createElement("canvas");
  canvas.width = TEXTURE_SIZE;
  canvas.height = TEXTURE_SIZE;
  const ctx = canvas.getContext("2d");
  if (ctx) {
    ctx.font = `bold ${TEXTURE_SIZE * 0.76}px ui-sans-serif, system-ui, sans-serif`;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    // Outline first: a letter over a bright 3D model needs its own contrast, and
    // the scene's background cannot be relied on behind it.
    ctx.lineWidth = TEXTURE_SIZE * 0.1;
    // Hex with alpha rather than `rgba(0, 0, 0, 0.85)`: `shadcn add` rewrites
    // colour functions, and it drops a component from this one, leaving
    // `rgba(0, 0, 0.85)` in a consumer's copy. Canvas ignores an invalid
    // `strokeStyle` and keeps the previous one, so the outline the installed
    // registry item draws would not be the one written here. The registry item
    // itself carries the source unchanged; the mangling is in the install step.
    ctx.strokeStyle = "#000000d9";
    ctx.strokeText(letter, TEXTURE_SIZE / 2, TEXTURE_SIZE * 0.54);
    ctx.fillStyle = axisColorCss(color);
    ctx.fillText(letter, TEXTURE_SIZE / 2, TEXTURE_SIZE * 0.54);
  }

  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  textureCache.set(key, texture);
  return texture;
}

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
    return AXIS_LETTERS.map((letter, i) => ({
      letter,
      texture: letterTexture(letter, AXIS_COLORS[i]),
      position: positions[i],
    }));
  }, [length]);
  const scale = axisLabelScale(length);

  return (
    <>
      {labels.map((label) => (
        <sprite
          key={label.letter}
          position={label.position}
          scale={[scale, scale, scale]}
          // Drawn last and without a depth test, so a letter on an axis pointing
          // away from the camera stays readable instead of hiding inside the
          // spacecraft. The axis *arrow* is still depth-tested, so it disappearing
          // into the body is what tells the reader that axis points away.
          renderOrder={LABEL_RENDER_ORDER}
        >
          <spriteMaterial
            map={label.texture}
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
