/**
 * Resolve the base URL for texture fetches.
 *
 * Priority:
 *  1. WS server origin (when connected) — server resizes on demand.
 *  2. Build-time VITE_TEXTURE_BASE_URL (static deployments that ship
 *     high-res textures alongside the viewer bundle).
 *  3. undefined — keep bundled 2K textures; do not probe unknown paths.
 */
export function resolveTextureBaseUrl(
  isConnected: boolean,
  wsUrl: string,
  envBaseUrl: string | undefined,
): string | undefined {
  if (isConnected) {
    try {
      const u = new URL(wsUrl.replace(/^ws/, "http"));
      return `${u.origin}/textures/`;
    } catch {
      return undefined;
    }
  }
  const raw = envBaseUrl?.trim();
  if (!raw) return undefined;
  return raw.endsWith("/") ? raw : `${raw}/`;
}
