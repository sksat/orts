import { describe, expect, it } from "vitest";

import { resolveAttitude } from "../attitude.js";
import { toOrbitPoint } from "./adapt.js";
import type { Quat, SatelliteState } from "./types.js";

/** The three things a caller can say about a satellite's orientation. */
const USABLE: SatelliteState = {
  id: "sat-a",
  position: [6778, 0, 0],
  attitude: [1, 0, 0, 0],
};
const REFUSED: SatelliteState = {
  id: "sat-b",
  position: [6778, 0, 0],
  attitudeRefused: true,
};
const ABSENT: SatelliteState = { id: "sat-c", position: [6778, 0, 0] };

describe("SatelliteState's attitude", () => {
  // The form every embedder already writes, and the one the viewer's own app
  // writes: a value computed as `Quat | undefined`, passed without narrowing.
  //
  // This file is compiled with this repo's settings, where an optional property
  // accepts `undefined` on its own. An embedder with `exactOptionalPropertyTypes`
  // on is stricter, and the arms in `types.ts` say `| undefined` for it —
  // measured against that flag, since no assertion here can observe it.
  it("accepts a quaternion, an undefined one, and neither", () => {
    const computed: Quat | undefined = Math.random() < 2 ? [1, 0, 0, 0] : undefined;
    const states: SatelliteState[] = [
      USABLE,
      ABSENT,
      REFUSED,
      { id: "sat-d", position: [6778, 0, 0], attitude: computed },
    ];

    expect(states).toHaveLength(4);
  });

  it("refuses a claim and a quaternion at once", () => {
    const both = {
      id: "sat-e",
      position: [6778, 0, 0] as Quat extends never ? never : [number, number, number],
      attitude: [1, 0, 0, 0] as Quat,
      attitudeRefused: true as const,
    };
    // @ts-expect-error — the two states are exclusive: a satellite cannot both
    // supply a rotation and say its claim was refused.
    const state: SatelliteState = both;
    expect(state.id).toBe("sat-e");
  });
});

describe("toOrbitPoint", () => {
  // The point is a position carrier. A refusal has no spelling in it, and
  // inventing one — four NaN components, as the rrd path once did — is what
  // the boundary's third state exists to avoid.
  it("does not encode a refusal into the point's quaternion", () => {
    const point = toOrbitPoint(REFUSED, 0);

    expect([point.qw, point.qx, point.qy, point.qz]).toEqual([
      undefined,
      undefined,
      undefined,
      undefined,
    ]);
    // Read on its own, the point says the satellite claimed nothing — which is
    // why the refusal travels beside it rather than inside it.
    expect(resolveAttitude(point).kind).toBe("absent");
  });

  it("carries a usable quaternion into the point", () => {
    const point = toOrbitPoint(USABLE, 0);

    expect(resolveAttitude(point).kind).toBe("usable");
  });
});
