import { beforeEach, describe, expect, it, vi } from "vitest";
import { readTimeRangeParam, writeTimeRangeParam } from "./urlParams.js";

describe("readTimeRangeParam", () => {
  beforeEach(() => {
    // Reset to clean URL
    history.replaceState(null, "", "/");
  });

  it("returns null when no timeRange param", () => {
    expect(readTimeRangeParam()).toBeNull();
  });

  it("returns number when timeRange is a valid number", () => {
    history.replaceState(null, "", "/?timeRange=300");
    expect(readTimeRangeParam()).toBe(300);
  });

  it("returns null when timeRange is 'all'", () => {
    history.replaceState(null, "", "/?timeRange=all");
    expect(readTimeRangeParam()).toBeNull();
  });

  it("returns null when timeRange is non-numeric", () => {
    history.replaceState(null, "", "/?timeRange=abc");
    expect(readTimeRangeParam()).toBeNull();
  });

  it("returns null when timeRange is empty string", () => {
    history.replaceState(null, "", "/?timeRange=");
    expect(readTimeRangeParam()).toBeNull();
  });

  it("parses floating point values", () => {
    history.replaceState(null, "", "/?timeRange=1800");
    expect(readTimeRangeParam()).toBe(1800);
  });

  it("returns null for negative values", () => {
    history.replaceState(null, "", "/?timeRange=-100");
    expect(readTimeRangeParam()).toBeNull();
  });

  it("returns null for zero", () => {
    history.replaceState(null, "", "/?timeRange=0");
    expect(readTimeRangeParam()).toBeNull();
  });
});

describe("writeTimeRangeParam", () => {
  beforeEach(() => {
    history.replaceState(null, "", "/");
  });

  it("sets timeRange param for a number", () => {
    writeTimeRangeParam(300);
    expect(window.location.search).toBe("?timeRange=300");
  });

  it("removes timeRange param for null", () => {
    history.replaceState(null, "", "/?timeRange=300");
    writeTimeRangeParam(null);
    expect(window.location.search).toBe("");
  });

  it("preserves other query params when setting", () => {
    history.replaceState(null, "", "/?foo=bar");
    writeTimeRangeParam(300);
    const params = new URLSearchParams(window.location.search);
    expect(params.get("foo")).toBe("bar");
    expect(params.get("timeRange")).toBe("300");
  });

  it("preserves other query params when removing", () => {
    history.replaceState(null, "", "/?foo=bar&timeRange=300");
    writeTimeRangeParam(null);
    const params = new URLSearchParams(window.location.search);
    expect(params.get("foo")).toBe("bar");
    expect(params.has("timeRange")).toBe(false);
  });

  it("overwrites existing timeRange value", () => {
    history.replaceState(null, "", "/?timeRange=300");
    writeTimeRangeParam(1800);
    expect(new URLSearchParams(window.location.search).get("timeRange")).toBe("1800");
  });

  it("uses replaceState (no history entry)", () => {
    const spy = vi.spyOn(history, "replaceState");
    writeTimeRangeParam(300);
    expect(spy).toHaveBeenCalled();
    spy.mockRestore();
  });
});
