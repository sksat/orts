/**
 * The public attitude contract, written the way an embedder writes it.
 *
 * Compiled twice: with this repo's settings (by the ordinary typecheck, where an
 * optional property accepts `undefined` on its own) and with
 * `exactOptionalPropertyTypes` on (by tsconfig.exactOptional.json, which
 * `exactOptionalContract.test.ts` runs). The second is the one that holds the
 * promise: a consumer's setting applies to the declarations, so the looser one
 * cannot stand in for it.
 */
import type { Quat, SatelliteState } from "./types.js";

declare const computed: Quat | undefined;

/** A value computed as `Quat | undefined`, passed without narrowing first. */
export const supplied: SatelliteState = {
  id: "sat-a",
  position: [6778, 0, 0],
  attitude: computed,
};

/** A caller that already knows its claim cannot be used. */
export const refused: SatelliteState = {
  id: "sat-b",
  position: [6778, 0, 0],
  attitudeRefused: true,
};

/** Nothing said about the orientation. */
export const absent: SatelliteState = { id: "sat-c", position: [6778, 0, 0] };

/** A refusal beside an explicit `undefined`, which says the same thing twice. */
export const refusedWithUndefined: SatelliteState = {
  id: "sat-d",
  position: [6778, 0, 0],
  attitude: undefined,
  attitudeRefused: true,
};

/** Not refused, said out loud. It means what an absent flag means. */
export const notRefused: SatelliteState = {
  id: "sat-e",
  position: [6778, 0, 0],
  attitude: computed,
  attitudeRefused: false,
};

declare const rotation: Quat;

export const contradictory = {
  id: "sat-f",
  position: [6778, 0, 0] as [number, number, number],
  attitude: rotation,
  attitudeRefused: true as const,
};

// @ts-expect-error the two states are exclusive: a rotation cannot arrive beside
// a refusal, under either setting.
export const both: SatelliteState = contradictory;
