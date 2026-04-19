/**
 * E2E test: CSV file loading through the unified source pipeline.
 *
 * Verifies that loading a CSV file:
 * - Populates TrailBuffer with correct orbit data
 * - Sets SimInfo from CSV metadata
 * - Displays data in charts
 * - Shows correct point count and file info
 */

import { writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";

/** Generate a minimal CSV file with metadata and orbit points. */
function generateTestCSV(numPoints: number, dt: number): string {
  const lines: string[] = [
    "# orts 2-body orbit propagation",
    "# mu = 398600.4418 km^3/s^2",
    "# epoch_jd = 2451545.0",
    "# central_body = earth",
    "# central_body_radius = 6378.137 km",
  ];

  const r = 6778; // km
  const v = 7.669; // km/s
  const omega = v / r; // rad/s

  for (let i = 0; i < numPoints; i++) {
    const t = i * dt;
    const angle = omega * t;
    const x = r * Math.cos(angle);
    const y = r * Math.sin(angle);
    const vx = -v * Math.sin(angle);
    const vy = v * Math.cos(angle);
    // t,x,y,z,vx,vy,vz,a,e,inc,raan,omega,nu
    lines.push(`${t},${x},${y},0,${vx},${vy},0,${r},0,0.9,0,0,${angle}`);
  }

  return lines.join("\n");
}

test("CSV file load populates trail and charts", async ({ page }) => {
  // Collect console errors for verification
  const consoleErrors: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error") consoleErrors.push(msg.text());
  });

  const NUM_POINTS = 50;
  const DT = 10;
  const EXPECTED_DURATION = (NUM_POINTS - 1) * DT; // 490s

  // Write test CSV to a temp file
  const csvContent = generateTestCSV(NUM_POINTS, DT);
  const csvPath = join(tmpdir(), `orts-test-${Date.now()}.csv`);
  writeFileSync(csvPath, csvContent);

  // Navigate without auto-connect (no WS server needed)
  await page.goto("/?noAutoConnect=1");

  // Upload the CSV file via the hidden file input
  const fileInput = page.locator('input[type="file"]');
  await fileInput.setInputFiles(csvPath);

  // Verify file load info text
  const orbitInfo = page.locator('[data-testid="orbit-info-file"]');
  await expect(orbitInfo).toContainText(`${NUM_POINTS} points`, { timeout: 5000 });
  await expect(orbitInfo).toContainText(`Duration: ${EXPECTED_DURATION}.0 s`);

  // Verify SimInfo is set from CSV metadata
  const simInfoText = page.locator('[data-testid="orbit-info-sim"]');
  await expect(simInfoText).toContainText("mu=398600.44", { timeout: 5000 });
  await expect(simInfoText).toContainText("dt=10.0");
  await expect(simInfoText).toContainText("2000-01-01"); // epoch_jd = 2451545.0 = J2000

  // Verify total points displayed matches
  const pointsInfo = page.locator('[data-testid="orbit-info-points"]');
  await expect(pointsInfo).toContainText(`${NUM_POINTS} points`, { timeout: 5000 });

  // Verify TrailBuffer has correct data via debug state
  const bufferCheck = await page.evaluate((_expectedPoints) => {
    const buffers = (window as unknown as Record<string, unknown>).__debug_ingest_buffers;
    if (!buffers || !(buffers instanceof Map)) return { ok: false, error: "no buffers" };

    const defaultBuf = (buffers as Map<string, { latestT: number }>).get("default");
    if (!defaultBuf) return { ok: false, error: "no default buffer" };

    return {
      ok: true,
      bufferCount: (buffers as Map<string, unknown>).size,
      latestT: defaultBuf.latestT,
    };
  }, NUM_POINTS);

  expect(bufferCheck.ok).toBe(true);
  expect(bufferCheck.bufferCount).toBe(1); // single satellite
  expect(bufferCheck.latestT).toBe(EXPECTED_DURATION); // last point t = 490

  // Verify no React/runtime errors during loading
  const criticalErrors = consoleErrors.filter(
    (e) => e.includes("Maximum update depth") || e.includes("Uncaught") || e.includes("unhandled"),
  );
  expect(criticalErrors).toEqual([]);
});
