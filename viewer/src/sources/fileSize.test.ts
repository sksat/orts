import { describe, expect, it } from "vitest";
import { LARGE_FILE_POINTS, largeFileWarning } from "./fileSize.js";

describe("largeFileWarning", () => {
  it("says nothing about a file below the threshold", () => {
    expect(largeFileWarning(LARGE_FILE_POINTS - 1)).toBeNull();
  });

  it("names the point count from the threshold on", () => {
    expect(largeFileWarning(LARGE_FILE_POINTS)).toContain("1,000,000 points");
    expect(largeFileWarning(1_234_567)).toContain("1,234,567 points");
  });
});
