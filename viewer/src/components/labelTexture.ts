import * as THREE from "three";

/**
 * Canvas textures for the short labels drawn in the scene: the letters on an
 * axis triad, and the names at the tips of the direction arrows.
 *
 * Sprites rather than 3D text — they face the camera from every angle, need no
 * font asset, and add no dependency. The texture is as wide as the text needs, so
 * a caller scales the sprite by {@link SceneLabel.aspect} to keep the glyphs from
 * stretching.
 */

/** One texture per text and colour, made on first use and kept. */
const cache = new Map<string, SceneLabel>();

/** Texture height in pixels. The width follows the text. */
const TEXTURE_HEIGHT = 128;

/** Outline width, as a fraction of the height. */
const OUTLINE_RATIO = 0.1;

/** Padding around the text, as a fraction of the height. */
const PADDING_RATIO = 0.12;

export interface SceneLabel {
  texture: THREE.Texture;
  /** Width / height of the texture, for scaling the sprite. */
  aspect: number;
}

/** CSS colour for a canvas context, from a Three.js hex colour. */
export function labelColorCss(color: number): string {
  return `#${color.toString(16).padStart(6, "0")}`;
}

/**
 * A texture with `text` drawn on it in `color`, outlined so it reads over a lit
 * 3D model as well as over the scene's background.
 *
 * Built lazily rather than at module scope: this module is imported by tests
 * that run without a DOM, where a canvas at import time would throw.
 */
export function labelTexture(text: string, color: number): SceneLabel {
  const key = `${text}:${color}`;
  const cached = cache.get(key);
  if (cached) return cached;

  const font = `bold ${TEXTURE_HEIGHT * 0.76}px ui-sans-serif, system-ui, sans-serif`;
  const canvas = document.createElement("canvas");
  const ctx = canvas.getContext("2d");
  let aspect = 1;
  if (ctx) {
    // Measure with the final font before sizing the canvas: setting the width
    // resets the context, so the font is applied twice on purpose.
    ctx.font = font;
    const width = ctx.measureText(text).width + TEXTURE_HEIGHT * PADDING_RATIO * 2;
    canvas.width = Math.max(1, Math.ceil(width));
    canvas.height = TEXTURE_HEIGHT;
    aspect = canvas.width / canvas.height;

    ctx.font = font;
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    // Outline first: the text over a bright 3D model needs its own contrast, and
    // the scene's background cannot be relied on behind it.
    ctx.lineWidth = TEXTURE_HEIGHT * OUTLINE_RATIO;
    // Hex with alpha rather than `rgba(0, 0, 0, 0.85)`: `shadcn add` rewrites
    // colour functions, and it drops a component from this one, leaving
    // `rgba(0, 0, 0.85)` in a consumer's copy. Canvas ignores an invalid
    // `strokeStyle` and keeps the previous one, so the outline the installed
    // registry item draws would not be the one written here.
    ctx.strokeStyle = "#000000d9";
    ctx.strokeText(text, canvas.width / 2, TEXTURE_HEIGHT * 0.54);
    ctx.fillStyle = labelColorCss(color);
    ctx.fillText(text, canvas.width / 2, TEXTURE_HEIGHT * 0.54);
  } else {
    canvas.width = TEXTURE_HEIGHT;
    canvas.height = TEXTURE_HEIGHT;
  }

  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  const label = { texture, aspect };
  cache.set(key, label);
  return label;
}

/** Above the scene's meshes, which use the default 0. */
export const LABEL_RENDER_ORDER = 10;
