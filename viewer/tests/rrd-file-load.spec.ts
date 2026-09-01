/**
 * E2E: opening a `.rrd` file fills in the orbital values the recording does
 * not carry.
 *
 * The decoder recovers position and velocity only. The Keplerian elements and
 * the chart scalars used to arrive as hardcoded zeros, so a recording of a
 * 400 km orbit charted a semi-major axis of 0 km and an altitude of 0 km. This
 * exercises the wiring the unit tests cannot: the adapter really calls
 * `arika-wasm` and the values really reach the ingest buffer.
 */
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

/** The recording committed for `rrd-wasm`'s own decoder tests. */
const FIXTURE = fileURLToPath(
  new URL("../../rrd-wasm/tests/fixtures/test_orbit.rrd", import.meta.url),
);

test("opening an .rrd derives its Keplerian elements and chart scalars", async ({ page }) => {
  const logs: string[] = [];
  page.on("console", (m) => logs.push(`[${m.type()}] ${m.text()}`));
  page.on("pageerror", (e) => logs.push(`[pageerror] ${e.message}`));

  await page.goto("/?noAutoConnect=1");

  const fileInput = page.locator('input[type="file"]');
  await fileInput.setInputFiles(FIXTURE);

  // Wait on the decoded point rather than the DOM: the info bar depends on
  // playback state, while this is the value under test.
  await page
    .waitForFunction(
      () => (window as unknown as Record<string, unknown>).__debug_rrd_first_point != null,
      { timeout: 20000 },
    )
    .catch(() => {
      throw new Error(`no point was decoded from the .rrd. console:\n${logs.join("\n")}`);
    });

  const sample = await page.evaluate(() => {
    const p = (window as unknown as Record<string, unknown>).__debug_rrd_first_point as
      | Record<string, number>
      | undefined;
    if (!p) return { ok: false as const, error: "no decoded point was exposed" };
    return {
      ok: true as const,
      a: p.a,
      e: p.e,
      altitude: p.altitude,
      specific_energy: p.specific_energy,
      angular_momentum: p.angular_momentum,
      velocity_mag: p.velocity_mag,
      r: Math.hypot(p.x, p.y, p.z),
    };
  });

  expect(sample.ok, `${sample.ok ? "" : sample.error}`).toBe(true);
  if (!sample.ok) return; // narrows the type; unreachable

  // The fixture is one satellite on a circular orbit 400 km up. Pinning the
  // measured values, not just "non-zero": a wrong field order or a unit slip
  // would still clear a range check.
  expect(sample.a, "semi-major axis (was hardcoded 0)").toBeCloseTo(6778.137, 3);
  expect(sample.e, "a circular orbit's eccentricity").toBeCloseTo(0, 6);
  expect(sample.altitude, "altitude (was hardcoded 0)").toBeCloseTo(400, 3);
  expect(sample.altitude).toBeCloseTo(sample.r - 6378.137, 6);
  expect(sample.specific_energy, "bound orbit, so negative (was 0)").toBeCloseTo(-29.4034, 3);
  expect(sample.angular_momentum, "|r x v| (was 0)").toBeCloseTo(51978.5379, 2);
  expect(sample.velocity_mag).toBeCloseTo(7.668558, 5);
});
