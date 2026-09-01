import { describe, expect, it } from "vitest";
import type { BodyCatalog } from "./bodyCatalog.js";
import { DEFAULT_BODY_CATALOG } from "./bodyCatalog.js";
import { describeCentralBodyError, resolveCentralBody } from "./centralBody.js";

const EARTH = DEFAULT_BODY_CATALOG.earth;
const MARS = DEFAULT_BODY_CATALOG.mars;

/** The resolved body, or a failure reported as the test's own message. */
function resolved(...args: Parameters<typeof resolveCentralBody>) {
  const result = resolveCentralBody(...args);
  if (!result.ok) {
    throw new Error(`expected a resolved body, got ${describeCentralBodyError(result.error)}`);
  }
  return result.body;
}

describe("resolveCentralBody", () => {
  it("takes what the source declared", () => {
    const body = resolved({ bodyId: "mars", mu: 42828.37, bodyRadius: 3396.2 });
    expect(body).toEqual({ bodyId: "mars", mu: 42828.37, bodyRadius: 3396.2 });
  });

  it("fills a missing value from the body the source names", () => {
    // The point of naming the body: a recording need not repeat constants that
    // belong to it. What it must not get is another body's.
    const body = resolved({ bodyId: "mars", mu: null, bodyRadius: null });
    expect(body.mu).toBe(MARS.mu);
    expect(body.bodyRadius).toBe(MARS.radiusKm);
    expect(body.mu).not.toBe(EARTH.mu);
    expect(body.bodyRadius).not.toBe(EARTH.radiusKm);
  });

  it("keys a body on its name whatever case the source wrote it in", () => {
    // `orts run` writes `arika`'s display name into a recording, so this is
    // what the default path produces: a case-sensitive lookup read "Earth" as a
    // body with no constants, and refused a recording that left one out.
    const body = resolved({ bodyId: "Earth", mu: null, bodyRadius: null });
    expect(body).toEqual({ bodyId: "earth", mu: EARTH.mu, bodyRadius: EARTH.radiusKm });
    expect(resolved({ bodyId: " MARS ", mu: null, bodyRadius: null }).bodyId).toBe("mars");
  });

  it("reads a source that names no body as Earth", () => {
    // Recordings predate the field, and their orbits are Earth's.
    const body = resolved({ mu: null, bodyRadius: null });
    expect(body).toEqual({ bodyId: "earth", mu: EARTH.mu, bodyRadius: EARTH.radiusKm });
  });

  it("keeps a body it has no constants for, as long as the source has them", () => {
    // Nothing is invented here, so there is nothing to refuse.
    const body = resolved({ bodyId: "kerbin", mu: 3531600, bodyRadius: 600 });
    expect(body).toEqual({ bodyId: "kerbin", mu: 3531600, bodyRadius: 600 });
  });

  it("fails on a body it has no constants for when a value is missing", () => {
    const result = resolveCentralBody({ bodyId: "kerbin", mu: 3531600, bodyRadius: null });
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.error).toEqual({ kind: "unknown-body", bodyId: "kerbin", missing: "radius" });
    expect(describeCentralBodyError(result.error)).toContain("kerbin");
  });

  it("takes a consumer's own body from the catalog it supplies", () => {
    const catalog: BodyCatalog = { kerbin: { mu: 3531600, radiusKm: 600 } };
    const body = resolved({ bodyId: "kerbin", mu: null, bodyRadius: null }, catalog);
    expect(body).toEqual({ bodyId: "kerbin", mu: 3531600, bodyRadius: 600 });
  });

  it("fails on a catalog entry that does not carry the missing value", () => {
    const catalog: BodyCatalog = { kerbin: { mu: 3531600 } };
    const result = resolveCentralBody({ bodyId: "kerbin", bodyRadius: null }, catalog);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.error).toEqual({ kind: "missing-default", bodyId: "kerbin", missing: "radius" });
  });

  it("fails on a mu no orbit can be scaled by, rather than substituting one", () => {
    // Dividing by it would make every element non-finite. Reaching for Earth
    // instead would answer a question the file got wrong with a number that
    // looks right.
    for (const mu of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
      const result = resolveCentralBody({ bodyId: "earth", mu, bodyRadius: 6378.137 });
      expect(result.ok).toBe(false);
      if (result.ok) continue;
      expect(result.error).toEqual({
        kind: "unusable-value",
        bodyId: "earth",
        field: "mu",
        value: mu,
        origin: "source",
      });
    }
  });

  it("fails on a radius no altitude can be measured from", () => {
    // Altitude is `r - bodyRadius`: a negative one reads as a height above the
    // orbit, and zero is a point mass rather than a body.
    for (const bodyRadius of [0, -6378.137, Number.NaN, Number.POSITIVE_INFINITY]) {
      const result = resolveCentralBody({ bodyId: "earth", mu: 398600.4418, bodyRadius });
      expect(result.ok).toBe(false);
      if (result.ok) continue;
      expect(result.error).toMatchObject({ kind: "unusable-value", field: "radius" });
    }
  });

  it("keeps the constant a partial override leaves alone", () => {
    // Correcting Earth's `mu` should not cost Earth its radius: replacing the
    // whole entry left a recording that omits its radius refused for a body the
    // viewer ships.
    const catalog: BodyCatalog = { earth: { mu: 398600.44 } };
    const body = resolved({ bodyId: "earth", mu: null, bodyRadius: null }, catalog);
    expect(body).toEqual({ bodyId: "earth", mu: 398600.44, bodyRadius: EARTH.radiusKm });
  });

  it("holds a catalog's own constants to the same constraint", () => {
    // A catalog is the consumer's to write. A `mu` of -1 in one is as unusable
    // as a `mu` of -1 in a file, and it would otherwise reach every element
    // derived from it without passing the check the file's value passes.
    const catalog: BodyCatalog = { kerbin: { mu: -1, radiusKm: 600 } };
    const result = resolveCentralBody({ bodyId: "kerbin" }, catalog);
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.error).toEqual({
      kind: "unusable-value",
      bodyId: "kerbin",
      field: "mu",
      value: -1,
      origin: "catalog",
    });
    expect(describeCentralBodyError(result.error)).toContain("catalog's");
  });

  it("reports the first field that cannot be resolved", () => {
    const result = resolveCentralBody({ bodyId: "earth", mu: -1, bodyRadius: -1 });
    expect(result.ok).toBe(false);
    if (result.ok) return;
    expect(result.error).toMatchObject({ field: "mu" });
  });
});

describe("DEFAULT_BODY_CATALOG", () => {
  it("carries both constants for every body arika propagates around", () => {
    // `arika`'s `KnownBody` has ten. A body missing one of them here would fail
    // to resolve for a recording that leaves that value out.
    const ids = [
      "sun",
      "mercury",
      "venus",
      "earth",
      "moon",
      "mars",
      "jupiter",
      "saturn",
      "uranus",
      "neptune",
    ];
    expect(Object.keys(DEFAULT_BODY_CATALOG).sort()).toEqual([...ids].sort());
    for (const id of ids) {
      const entry = DEFAULT_BODY_CATALOG[id];
      expect(entry.mu, `${id} mu`).toBeGreaterThan(0);
      expect(entry.radiusKm, `${id} radius`).toBeGreaterThan(0);
    }
  });
});
