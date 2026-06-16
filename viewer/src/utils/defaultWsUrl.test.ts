import { describe, expect, it } from "vitest";
import { CLI_DEFAULT_WS_URL, resolveDefaultWsUrl } from "./defaultWsUrl.js";

describe("resolveDefaultWsUrl", () => {
  it("returns the explicit VITE_WS_URL override when set", () => {
    expect(
      resolveDefaultWsUrl({
        explicitWsUrl: "ws://example.test:1234/ws",
        baseUrl: "/",
        protocol: "https:",
        host: "sksat.github.io",
      }),
    ).toBe("ws://example.test:1234/ws");
  });

  it("lets the explicit override win even on a static sub-path deploy", () => {
    expect(
      resolveDefaultWsUrl({
        explicitWsUrl: "wss://relay.example/ws",
        baseUrl: "/orts/viewer/",
        protocol: "https:",
        host: "sksat.github.io",
      }),
    ).toBe("wss://relay.example/ws");
  });

  it("treats an empty explicit override as unset", () => {
    expect(
      resolveDefaultWsUrl({
        explicitWsUrl: "",
        baseUrl: "/",
        protocol: "http:",
        host: "localhost:9001",
      }),
    ).toBe("ws://localhost:9001/ws");
  });

  it("derives from the page origin when co-served from the root (orts serve)", () => {
    expect(
      resolveDefaultWsUrl({
        baseUrl: "/",
        protocol: "http:",
        host: "localhost:9001",
      }),
    ).toBe("ws://localhost:9001/ws");
  });

  it("preserves the served port when co-served on a non-default port", () => {
    expect(
      resolveDefaultWsUrl({
        baseUrl: "/",
        protocol: "http:",
        host: "localhost:8080",
      }),
    ).toBe("ws://localhost:8080/ws");
  });

  it("uses wss when co-served over https (remote orts serve)", () => {
    expect(
      resolveDefaultWsUrl({
        baseUrl: "/",
        protocol: "https:",
        host: "orts.example.com",
      }),
    ).toBe("wss://orts.example.com/ws");
  });

  it("falls back to the CLI default on a static sub-path deploy (GitHub Pages)", () => {
    // GitHub Pages serves the viewer at a non-root base; the WS server is the
    // user's local `orts serve`, not the Pages host.
    expect(
      resolveDefaultWsUrl({
        baseUrl: "/orts/viewer/",
        protocol: "https:",
        host: "sksat.github.io",
      }),
    ).toBe(CLI_DEFAULT_WS_URL);
  });

  it("allows overriding the local CLI default", () => {
    expect(
      resolveDefaultWsUrl({
        baseUrl: "/orts/viewer/",
        protocol: "https:",
        host: "sksat.github.io",
        localCliWsUrl: "ws://127.0.0.1:7777/ws",
      }),
    ).toBe("ws://127.0.0.1:7777/ws");
  });

  it("exposes the CLI default matching `orts serve` (port 9001)", () => {
    expect(CLI_DEFAULT_WS_URL).toBe("ws://localhost:9001/ws");
  });
});
