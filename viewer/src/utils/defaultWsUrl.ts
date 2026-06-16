/**
 * The WebSocket endpoint `orts serve` advertises by default
 * (`ws://localhost:9001/ws`, matching the CLI's `--port 9001` default).
 *
 * Used as the connection default for static deployments (e.g. GitHub Pages)
 * where the page host is not the telemetry server — the user runs `orts serve`
 * locally and the viewer dials back to it.
 */
export const CLI_DEFAULT_WS_URL = "ws://localhost:9001/ws";

export interface ResolveDefaultWsUrlInput {
  /** Build-time override (`import.meta.env.VITE_WS_URL`). Wins when non-empty. */
  explicitWsUrl?: string;
  /** Vite base path (`import.meta.env.BASE_URL`); `"/"` unless a sub-path build. */
  baseUrl: string;
  /** Page protocol (`window.location.protocol`), e.g. `"https:"`. */
  protocol: string;
  /** Page host incl. port (`window.location.host`), e.g. `"localhost:9001"`. */
  host: string;
  /** Override for the static-deploy fallback; defaults to {@link CLI_DEFAULT_WS_URL}. */
  localCliWsUrl?: string;
}

/**
 * Resolve the default WebSocket URL the viewer connects to on startup.
 *
 * Precedence:
 * 1. An explicit `VITE_WS_URL` build override always wins.
 * 2. A non-root `baseUrl` marks a static sub-path deploy (only the GitHub Pages
 *    build sets `VITE_BASE_PATH`). There the page host is *not* the WS server,
 *    so dial the CLI default — the user's local `orts serve`. This is a
 *    project-local build contract, not a general "base path ⇒ WS location"
 *    rule: were `orts serve` ever to host the viewer under a sub-path, this
 *    would need revisiting (set `VITE_WS_URL` for such deploys).
 * 3. Otherwise the viewer is co-served by `orts serve` at the origin root
 *    (locally or on a remote box, since serve binds `0.0.0.0`), so derive the
 *    URL from the page origin — preserving a non-default `--port`.
 */
export function resolveDefaultWsUrl({
  explicitWsUrl,
  baseUrl,
  protocol,
  host,
  localCliWsUrl = CLI_DEFAULT_WS_URL,
}: ResolveDefaultWsUrlInput): string {
  if (explicitWsUrl) return explicitWsUrl;
  if (baseUrl !== "/") return localCliWsUrl;
  const wsProtocol = protocol === "https:" ? "wss:" : "ws:";
  return `${wsProtocol}//${host}/ws`;
}
