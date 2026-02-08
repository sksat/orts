import { describe, it, expect } from "vitest";
import { computeRetention } from "./RetentionPolicy.js";

describe("computeRetention", () => {
  it("returns shouldDownsample=false when under maxRows", () => {
    const result = computeRetention(5000, 100000);
    expect(result.shouldDownsample).toBe(false);
    expect(result.keepEveryN).toBe(1);
  });

  it("returns shouldDownsample=false when exactly at maxRows", () => {
    const result = computeRetention(100000, 100000);
    expect(result.shouldDownsample).toBe(false);
    expect(result.keepEveryN).toBe(1);
  });

  it("returns shouldDownsample=true when over maxRows", () => {
    const result = computeRetention(150000, 100000);
    expect(result.shouldDownsample).toBe(true);
    expect(result.keepEveryN).toBeGreaterThanOrEqual(2);
  });

  it("keepEveryN increases as totalRows grows beyond maxRows", () => {
    const r1 = computeRetention(120000, 100000);
    const r2 = computeRetention(200000, 100000);
    expect(r2.keepEveryN).toBeGreaterThanOrEqual(r1.keepEveryN);
  });

  it("handles edge case: 0 rows", () => {
    const result = computeRetention(0, 100000);
    expect(result.shouldDownsample).toBe(false);
  });

  it("handles edge case: 1 row", () => {
    const result = computeRetention(1, 100000);
    expect(result.shouldDownsample).toBe(false);
  });

  it("handles edge case: maxRows = 0", () => {
    const result = computeRetention(100, 0);
    expect(result.shouldDownsample).toBe(false);
  });
});
