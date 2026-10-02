import type { Pacing } from "../protocol/generated/Pacing.js";

/**
 * The PlaybackBar mode indicator's text. Live follows the server, so it names
 * the server's pace; the replay speed only applies once the view leaves Live.
 */
export function playbackModeLabel(
  isRealtimeMode: boolean,
  isLive: boolean | undefined,
  isPlaying: boolean,
  serverPacing: Pacing | undefined,
): string {
  if (!isRealtimeMode) return "Replay";
  if (isLive) return serverPacing === undefined ? "Live" : `Live · ${serverPacing}`;
  return isPlaying ? "Playing" : "Paused";
}
