import { describe, expect, it } from "vitest";
import type { OrbitPoint } from "../orbit.js";
import type { TrailBuffer } from "../utils/TrailBuffer.js";
import { buildRenderEntries } from "./renderEntries.js";

const buf = (length: number) => ({ length }) as TrailBuffer;
const pt = () => ({}) as OrbitPoint;

describe("buildRenderEntries", () => {
  it("renders a satellite that has a position but no trail", () => {
    const entries = buildRenderEntries(new Map(), new Map([["a", pt()]]));
    expect(entries.map((e) => e.satId)).toEqual(["a"]);
    expect(entries[0].buf).toBeUndefined();
    expect(entries[0].pos).toBeDefined();
  });

  it("includes trail-having satellites and skips empty trails", () => {
    const tb = new Map([
      ["a", buf(5)],
      ["b", buf(0)], // empty trail, no position → not rendered
    ]);
    expect(buildRenderEntries(tb, undefined).map((e) => e.satId)).toEqual(["a"]);
  });

  it("unions trails and positions (trail-having first, no duplicates)", () => {
    const tb = new Map([["a", buf(3)]]);
    const sp = new Map<string, OrbitPoint | null>([
      ["a", pt()],
      ["b", pt()],
    ]);
    expect(buildRenderEntries(tb, sp).map((e) => e.satId)).toEqual(["a", "b"]);
  });

  it("excludes null positions", () => {
    const sp = new Map<string, OrbitPoint | null>([["a", null]]);
    expect(buildRenderEntries(new Map(), sp)).toEqual([]);
  });

  it("exposes both the buffer and the position for a satellite that has both", () => {
    const tb = new Map([["a", buf(2)]]);
    const sp = new Map<string, OrbitPoint | null>([["a", pt()]]);
    const [entry] = buildRenderEntries(tb, sp);
    expect(entry.buf?.length).toBe(2);
    expect(entry.pos).toBeDefined();
  });
});
