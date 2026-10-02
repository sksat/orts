import { describe, expect, it } from "vitest";
import { playbackModeLabel } from "./playbackModeLabel.js";

describe("playbackModeLabel", () => {
  it("names the server's pace while Live, when the server says it", () => {
    // Live follows the server, so its speed is the server's, not the replay
    // speed next to it.
    expect(playbackModeLabel(true, true, false, "realtime")).toBe("Live · realtime");
    expect(playbackModeLabel(true, true, false, "accelerated")).toBe("Live · accelerated");
    expect(playbackModeLabel(true, true, false, undefined)).toBe("Live");
  });

  it("says Playing or Paused once the view has left Live", () => {
    expect(playbackModeLabel(true, false, true, "realtime")).toBe("Playing");
    expect(playbackModeLabel(true, false, false, "realtime")).toBe("Paused");
  });

  it("says Replay without a live source", () => {
    expect(playbackModeLabel(false, undefined, true, undefined)).toBe("Replay");
  });
});
