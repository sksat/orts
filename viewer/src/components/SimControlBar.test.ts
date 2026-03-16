import { describe, expect, it } from "vitest";
import type { SimControlBarProps } from "./SimControlBar.js";

describe("SimControlBar props", () => {
  it("running state expects onPause and onTerminate", () => {
    const props: SimControlBarProps = {
      serverState: "running",
      onPause: () => {},
      onResume: () => {},
      onTerminate: () => {},
    };
    expect(props.serverState).toBe("running");
  });

  it("paused state expects onResume and onTerminate", () => {
    const props: SimControlBarProps = {
      serverState: "paused",
      onPause: () => {},
      onResume: () => {},
      onTerminate: () => {},
    };
    expect(props.serverState).toBe("paused");
  });
});
