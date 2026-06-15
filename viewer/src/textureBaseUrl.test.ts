import { describe, expect, it } from "vitest";
import { resolveTextureBaseUrl } from "./textureBaseUrl.js";

describe("resolveTextureBaseUrl", () => {
  it("returns WS server origin when connected", () => {
    expect(resolveTextureBaseUrl(true, "ws://example.com:9001/ws", undefined)).toBe(
      "http://example.com:9001/textures/",
    );
  });

  it("WS takes priority over env var when connected", () => {
    expect(resolveTextureBaseUrl(true, "ws://example.com/ws", "/orts/viewer/textures/")).toBe(
      "http://example.com/textures/",
    );
  });

  it("falls back to envBaseUrl when disconnected", () => {
    expect(resolveTextureBaseUrl(false, "", "/orts/viewer/textures/")).toBe(
      "/orts/viewer/textures/",
    );
  });

  it("adds trailing slash when envBaseUrl is missing it", () => {
    expect(resolveTextureBaseUrl(false, "", "/orts/viewer/textures")).toBe(
      "/orts/viewer/textures/",
    );
  });

  it("trims whitespace from envBaseUrl", () => {
    expect(resolveTextureBaseUrl(false, "", "  /orts/viewer/textures/  ")).toBe(
      "/orts/viewer/textures/",
    );
  });

  it("returns undefined when disconnected and envBaseUrl is absent", () => {
    expect(resolveTextureBaseUrl(false, "", undefined)).toBeUndefined();
  });

  it("returns undefined when disconnected and envBaseUrl is whitespace-only", () => {
    expect(resolveTextureBaseUrl(false, "", "   ")).toBeUndefined();
  });
});
