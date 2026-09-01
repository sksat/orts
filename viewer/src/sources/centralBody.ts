/**
 * The central body constants a source is read against.
 *
 * One module because two readers have to agree: the WASM batch that derives the
 * orbital values, and the `SimInfo` / DuckDB schema that the charts are built
 * from. A `mu` that differs between them shows the same recording two ways.
 *
 * Nothing here invents a constant. A source says which body it is around and may
 * carry that body's `mu` and radius; where it carries neither, the value comes
 * from the body's catalog entry, and where there is no entry to take it from,
 * resolving fails rather than reading the orbit against Earth. A body that is
 * Mars by name and Earth by radius is not a body, and the altitude measured
 * against it (`r - bodyRadius`) is wrong by thousands of kilometres with nothing
 * on the chart to say so.
 */

import { type BodyCatalog, IMPLICIT_BODY_ID, resolveBodyCatalog } from "./bodyCatalog.js";

/** Resolved central body constants. */
export interface CentralBody {
  /** The body these came from, as the source named it. */
  bodyId: string;
  mu: number;
  bodyRadius: number;
}

/** Which field of a central body is at issue. */
export type CentralBodyField = "mu" | "radius";

/** Why a source's central body could not be resolved. */
export type CentralBodyError =
  | {
      /** The source named a body the catalog does not carry, and left a value out. */
      kind: "unknown-body";
      bodyId: string;
      missing: CentralBodyField;
    }
  | {
      /** The catalog carries the body but not the value the source left out. */
      kind: "missing-default";
      bodyId: string;
      missing: CentralBodyField;
    }
  | {
      /** The source carried the value, and it cannot be one. */
      kind: "unusable-value";
      bodyId: string;
      field: CentralBodyField;
      value: number;
    };

export type CentralBodyResult =
  | { ok: true; body: CentralBody }
  | { ok: false; error: CentralBodyError };

/** What a source declared about the body it is around. */
export interface DeclaredCentralBody {
  /**
   * The body's id, as the source named it. A source that names none is read as
   * Earth: recordings predate the field, and their orbits are Earth's.
   */
  bodyId?: string | null;
  mu?: number | null;
  bodyRadius?: number | null;
}

/** A one-line account of why resolving failed, for an error event. */
export function describeCentralBodyError(error: CentralBodyError): string {
  switch (error.kind) {
    case "unknown-body":
      return `the recording is around "${error.bodyId}", which this viewer has no constants for, and carries no ${error.missing} of its own`;
    case "missing-default":
      return `the recording carries no ${error.missing}, and none is known for "${error.bodyId}"`;
    case "unusable-value":
      return `the recording's ${error.field} for "${error.bodyId}" is ${error.value}, which no orbit can be measured against`;
  }
}

/**
 * Resolve the constants a source's orbits are measured against.
 *
 * A `mu` must be positive and finite: it scales the orbit, and every element
 * derived from a zero, negative or non-finite one comes out non-finite. A radius
 * must be positive and finite too — altitude is `r - bodyRadius`, so a negative
 * one reads as a height above the orbit, and zero is a point mass rather than a
 * body. Where the source carries such a value, that is an error and not a reason
 * to reach for the catalog: the file says something about itself that cannot be
 * true, and quietly substituting a default hides it.
 *
 * A body the catalog does not carry is fine as long as the source carries both
 * of its constants — nothing is being invented then. It fails only where a value
 * is missing and there is nothing to fill it from.
 */
export function resolveCentralBody(
  declared: DeclaredCentralBody,
  catalog?: BodyCatalog,
): CentralBodyResult {
  const bodyId = declared.bodyId ?? IMPLICIT_BODY_ID;
  const entry = resolveBodyCatalog(catalog)[bodyId];

  const resolve = (
    declaredValue: number | null | undefined,
    fromCatalog: number | undefined,
    field: CentralBodyField,
  ): { ok: true; value: number } | { ok: false; error: CentralBodyError } => {
    if (declaredValue != null) {
      return Number.isFinite(declaredValue) && declaredValue > 0
        ? { ok: true, value: declaredValue }
        : { ok: false, error: { kind: "unusable-value", bodyId, field, value: declaredValue } };
    }
    if (fromCatalog != null) {
      return { ok: true, value: fromCatalog };
    }
    return {
      ok: false,
      error: entry
        ? { kind: "missing-default", bodyId, missing: field }
        : { kind: "unknown-body", bodyId, missing: field },
    };
  };

  const mu = resolve(declared.mu, entry?.mu, "mu");
  if (!mu.ok) return { ok: false, error: mu.error };
  const radius = resolve(declared.bodyRadius, entry?.radiusKm, "radius");
  if (!radius.ok) return { ok: false, error: radius.error };

  return { ok: true, body: { bodyId, mu: mu.value, bodyRadius: radius.value } };
}
