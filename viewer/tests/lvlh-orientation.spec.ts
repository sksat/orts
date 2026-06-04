/**
 * E2E regression test for the satellite-centered (LVLH) central-body orientation.
 *
 * Guards against the bug in #50, where the Earth's pole-alignment rotation
 * `R_x(π/2)` was applied twice in the LVLH path, tilting the central body 90°
 * about the in-track axis. The visible symptom: from an *equatorial* orbit the
 * pole/ice cap appears directly below the satellite instead of the equator.
 *
 * Rather than reading pixels (texture/lighting/AA make that flaky), this reads
 * the rendered Earth mesh world quaternion from the live Three.js scene graph
 * (exposed via `window.__debug_get_earth_world_quat` in dev/E2E builds) and
 * checks a frame-invariant: for an equatorial orbit the Earth's north pole must
 * lie along the LVLH cross-track axis (scene +Y), i.e. `|pole.y| ≈ 1`. The
 * double-rotation bug puts the pole in the in-track/radial plane → `pole.y ≈ 0`.
 *
 * The check is independent of orbital phase and of the ERA value (R_z(ERA)
 * leaves the pole on the cross-track axis), so it does not depend on the
 * arika WASM rotation model being correct — only on the viewer's compositing.
 */
import { writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";

/** Equatorial circular orbit with position + velocity and a J2000 epoch. */
function generateEquatorialCSV(numPoints: number, dt: number): string {
  const mu = 398600.4418;
  const r = 7378; // ~1000 km altitude
  const v = Math.sqrt(mu / r);
  const omega = v / r;
  const lines = [
    "# orts 2-body orbit propagation",
    `# mu = ${mu} km^3/s^2`,
    "# epoch_jd = 2451545.0",
    "# central_body = earth",
    "# central_body_radius = 6378.137 km",
  ];
  for (let i = 0; i < numPoints; i++) {
    const t = i * dt;
    const a = omega * t;
    const x = r * Math.cos(a);
    const y = r * Math.sin(a);
    const vx = -v * Math.sin(a);
    const vy = v * Math.cos(a);
    lines.push(`${t},${x},${y},0,${vx},${vy},0,${r},0,0,0,0,${a}`);
  }
  return lines.join("\n");
}

test("equatorial orbit: LVLH central body keeps the pole on cross-track (not nadir)", async ({
  page,
}) => {
  const csvPath = join(tmpdir(), `orts-lvlh-orientation-${Date.now()}.csv`);
  writeFileSync(csvPath, generateEquatorialCSV(120, 30));

  await page.goto("/?noAutoConnect=1");
  await page.locator('input[type="file"]').setInputFiles(csvPath);
  await expect(page.locator('[data-testid="orbit-info-file"]')).toContainText("points", {
    timeout: 10000,
  });

  await expect(page.locator("canvas").first()).toBeVisible();

  // Deterministic state: pause playback and seek to a fixed phase.
  const playPause = page.locator('[data-testid="play-pause-btn"]');
  if ((await playPause.textContent())?.includes("Pause")) {
    await playPause.click();
  }
  await page.locator('[data-testid="time-slider"]').fill("500");

  // Center the view on the satellite → activates the LVLH (body-fixed) frame.
  await page.locator('[data-testid="frame-selector-select"]').selectOption("satellite:default");

  // Wait until the Earth's orientation is published from the scene graph.
  await page.waitForFunction(
    () =>
      typeof (window as unknown as Record<string, unknown>).__debug_get_earth_world_quat ===
      "function",
    { timeout: 10000 },
  );
  await page.waitForTimeout(1500); // let the LVLH frame switch apply + render a frame

  const quat = (await page.evaluate(() => {
    const get = (window as unknown as Record<string, unknown>).__debug_get_earth_world_quat as
      | (() => [number, number, number, number] | null)
      | undefined;
    return get ? get() : null;
  })) as [number, number, number, number] | null;

  expect(quat, "Earth world quaternion should be exposed once LVLH is active").not.toBeNull();
  if (quat == null) return; // narrows type; unreachable — expect() throws above on null
  const [qx, , qz] = quat;

  // World direction of the geographic north pole = quat · (0,1,0); its Y (cross-track)
  // component is `1 - 2(qx² + qz²)`. Equatorial orbit ⇒ pole is the orbit normal ⇒
  // it must lie on cross-track (scene +Y), so |pole.y| ≈ 1.
  const poleY = 1 - 2 * (qx * qx + qz * qz);
  expect(
    Math.abs(poleY),
    `north pole should lie on the LVLH cross-track axis (|pole.y|≈1), got pole.y=${poleY.toFixed(
      3,
    )} — a value near 0 means the central body is tilted 90° (pole at nadir)`,
  ).toBeGreaterThan(0.9);
});
