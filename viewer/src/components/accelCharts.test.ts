import { describe, expect, it } from "vitest";

import { isAccelChartActive } from "./accelCharts.js";

describe("isAccelChartActive", () => {
  it("shows nothing when no perturbation is active", () => {
    expect(isAccelChartActive(undefined, [])).toBe(false);
    expect(isAccelChartActive("srp", [])).toBe(false);
    expect(isAccelChartActive("srp", undefined)).toBe(false);
  });

  it("shows gravity and the total whenever anything is active", () => {
    expect(isAccelChartActive(undefined, ["drag"])).toBe(true);
    expect(isAccelChartActive("_any", ["drag"])).toBe(true);
  });

  it("matches a perturbation by its own name", () => {
    expect(isAccelChartActive("srp", ["gravity", "srp"])).toBe(true);
    expect(isAccelChartActive("drag", ["gravity", "srp"])).toBe(false);
  });

  // A panel model reports its acceleration under the force's name, so its
  // chart has to appear even though `perturbations` names the model.
  it("matches the panel model of the same force", () => {
    expect(isAccelChartActive("srp", ["gravity", "panel_srp"])).toBe(true);
    expect(isAccelChartActive("drag", ["gravity", "panel_drag"])).toBe(true);
  });

  it("does not match an unrelated panel model", () => {
    expect(isAccelChartActive("srp", ["gravity", "panel_drag"])).toBe(false);
  });
});
