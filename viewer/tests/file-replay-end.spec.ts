/**
 * E2E test: replaying a loaded file to its end leaves the bar paused there.
 *
 * A loaded file rests paused at its end. Playing it from the start runs until
 * the end and pauses again rather than going live, and the playback bar has
 * to show that: the frame that reaches the end changes the mode without any
 * new point arriving.
 */

import { writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";

test("replaying a loaded file to its end leaves the bar paused there", async ({ page }) => {
  const lines = ["# mu = 398600.4418 km^3/s^2", "# epoch_jd = 2451545.0"];
  for (let i = 0; i < 50; i++) lines.push(`${i * 10},6778,0,0,0,7.669,0,6778,0,0.9,0,0,0`);
  const csvPath = join(tmpdir(), `orts-replay-end-${Date.now()}.csv`);
  writeFileSync(csvPath, lines.join("\n"));

  await page.goto("/?noAutoConnect=1");
  await page.locator('input[type="file"]').setInputFiles(csvPath);
  await expect(page.locator('[data-testid="orbit-info-file"]')).toContainText("50 points", {
    timeout: 10_000,
  });

  const playPause = page.locator('[data-testid="play-pause-btn"]');
  const time = page.locator('[data-testid="playback-time"]');
  await expect(playPause).toHaveText("Play");

  // 490 s of sim time at 100x: about five seconds to the end.
  await page.locator('[data-testid="replay-speed-select"]').selectOption("100");
  await page.locator('[data-testid="time-slider"]').fill("0");
  await playPause.click();
  await expect(playPause).toHaveText("Pause");

  await expect(playPause).toHaveText("Play", { timeout: 20_000 });
  await expect(time).toContainText("T+8m 10.0s / 8m 10.0s");
  await expect(page.locator('[data-testid="playback-mode"]')).toHaveText("Replay");
});
