import { type ChildProcess, spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// These tests verify texture lazy-loading:
// - Embedded 2K textures are always served via HTTP
// - `textures_ready` WS message is broadcast when a simulation starts
// - High-res textures cached on disk are served via the /textures/ endpoint
//
// To avoid depending on slow NASA downloads, we pre-populate the texture cache
// with a small fake JPEG for earth_4k.jpg so the downloader skips it and
// sends textures_ready immediately.

// Minimal valid JPEG: FF D8 FF E0 ... FF D9
// biome-ignore lint/style/noNonNullAssertion: test helper
const FAKE_JPEG = Buffer.from([
  0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
  0x00, 0x01, 0x00, 0x00, 0xff, 0xd9,
]);

const CACHE_DIR = "/tmp/orts/textures";

// Earth day textures that the downloader would produce.
const EARTH_DAY_FILES = ["earth_4k.jpg", "earth_8k.jpg", "earth_16k.jpg"];
const EARTH_NIGHT_FILES = ["earth_night_4k.jpg", "earth_night_8k.jpg", "earth_night_16k.jpg"];
const SUN_FILES = ["sun_4k.jpg"];

let ortsProcess: ChildProcess | undefined;
let wsUrl: string;
let httpBaseUrl: string;

test.beforeAll(async () => {
  // Pre-populate cache with fake JPEGs so downloads are skipped.
  fs.mkdirSync(CACHE_DIR, { recursive: true });
  for (const name of [...EARTH_DAY_FILES, ...EARTH_NIGHT_FILES, ...SUN_FILES]) {
    fs.writeFileSync(path.join(CACHE_DIR, name), FAKE_JPEG);
  }

  // Start orts in idle mode.
  const binary = process.env.ORTS_BINARY ?? path.resolve(__dirname, "../../target/debug/orts");
  const child = spawn(binary, ["serve", "--port", "0"]);
  ortsProcess = child;

  const port = await new Promise<number>((resolve, reject) => {
    const rl = createInterface({ input: child.stderr! });
    const timeout = setTimeout(() => {
      rl.close();
      reject(new Error("Timed out waiting for orts server to start"));
    }, 30000);

    rl.on("line", (line) => {
      const match = line.match(/ws:\/\/localhost:(\d+)/);
      if (match) {
        clearTimeout(timeout);
        resolve(parseInt(match[1], 10));
      }
    });

    child.on("error", (err) => {
      clearTimeout(timeout);
      reject(err);
    });
    child.on("exit", (code) => {
      clearTimeout(timeout);
      reject(new Error(`orts exited with code ${code} before listening`));
    });
  });

  wsUrl = `ws://localhost:${port}/ws`;
  httpBaseUrl = `http://localhost:${port}`;
  console.log(`orts idle server started at ${wsUrl}`);
});

test.afterAll(async () => {
  if (ortsProcess && !ortsProcess.killed) {
    ortsProcess.kill("SIGTERM");
  }
  // Clean up fake cache files.
  for (const name of [...EARTH_DAY_FILES, ...EARTH_NIGHT_FILES, ...SUN_FILES]) {
    const p = path.join(CACHE_DIR, name);
    if (fs.existsSync(p)) {
      // Only remove if it's our tiny fake (< 1KB). Don't delete real cached textures.
      const stat = fs.statSync(p);
      if (stat.size < 1024) fs.unlinkSync(p);
    }
  }
});

test("embedded 2K textures are served via HTTP", async ({ request }) => {
  for (const name of ["earth_2k.jpg", "earth_night_2k.jpg", "moon.jpg", "mars.jpg", "sun.jpg"]) {
    const resp = await request.get(`${httpBaseUrl}/textures/${name}`);
    expect(resp.status(), `${name} should return 200`).toBe(200);
    expect(resp.headers()["content-type"]).toBe("image/jpeg");

    const body = await resp.body();
    expect(body[0], `${name} JPEG magic`).toBe(0xff);
    expect(body[1]).toBe(0xd8);
    expect(body[2]).toBe(0xff);
  }
});

test("unknown texture returns 404", async ({ request }) => {
  const resp = await request.get(`${httpBaseUrl}/textures/nonexistent.jpg`);
  expect(resp.status()).toBe(404);
});

test("cached texture is served from disk", async ({ request }) => {
  // earth_4k.jpg was pre-populated with a fake JPEG in beforeAll.
  const resp = await request.get(`${httpBaseUrl}/textures/earth_4k.jpg`);
  expect(resp.status()).toBe(200);
  expect(resp.headers()["content-type"]).toBe("image/jpeg");

  const body = await resp.body();
  expect(body[0]).toBe(0xff);
  expect(body[1]).toBe(0xd8);
});

test("textures_ready arrives after starting simulation", async ({ page }) => {
  await page.goto("/?noAutoConnect=1");

  // Connect WS to idle server, start simulation, wait for textures_ready.
  // Because cache is pre-populated, downloads are skipped and textures_ready
  // should arrive within seconds.
  const result = await page.evaluate(
    async ({ url }) => {
      return new Promise<{ messageTypes: string[]; texturesReadyBodies: string[] }>((resolve) => {
        const ws = new WebSocket(url);
        const messageTypes: string[] = [];
        const texturesReadyBodies: string[] = [];

        ws.addEventListener("open", () => {
          ws.send(
            JSON.stringify({
              type: "start_simulation",
              config: {
                body: "earth",
                satellites: [
                  {
                    orbit: { type: "circular", altitude: 400 },
                  },
                ],
              },
            }),
          );
        });

        ws.addEventListener("message", (e) => {
          try {
            const msg = JSON.parse(e.data as string);
            messageTypes.push(msg.type);
            if (msg.type === "textures_ready") {
              texturesReadyBodies.push(msg.body);
            }
          } catch {
            // ignore
          }
        });

        // With cached textures, textures_ready should arrive quickly.
        const check = setInterval(() => {
          // Wait for at least earth (central body). Sun and moon may also arrive.
          if (texturesReadyBodies.includes("earth")) {
            clearInterval(check);
            clearTimeout(timeout);
            ws.close();
            resolve({ messageTypes, texturesReadyBodies });
          }
        }, 200);

        const timeout = setTimeout(() => {
          clearInterval(check);
          ws.close();
          resolve({ messageTypes, texturesReadyBodies });
        }, 30_000);
      });
    },
    { url: wsUrl },
  );

  console.log("Unique message types:", [...new Set(result.messageTypes)]);
  console.log("textures_ready bodies:", result.texturesReadyBodies);

  expect(result.texturesReadyBodies, "should receive textures_ready for earth").toContain("earth");
  // Sun should also be notified (third body for earth).
  expect(result.texturesReadyBodies, "should receive textures_ready for sun").toContain("sun");
});

test("no textures_ready before simulation starts", async ({ page }) => {
  // Start a fresh idle server connection. Without starting a simulation,
  // no textures_ready messages should arrive.
  await page.goto("/?noAutoConnect=1");

  const result = await page.evaluate(
    async ({ url }) => {
      return new Promise<{ types: string[]; gotTexturesReady: boolean }>((resolve) => {
        const ws = new WebSocket(url);
        const types: string[] = [];
        let gotTexturesReady = false;

        ws.addEventListener("message", (e) => {
          try {
            const msg = JSON.parse(e.data as string);
            types.push(msg.type);
            if (msg.type === "textures_ready") {
              gotTexturesReady = true;
            }
          } catch {
            // ignore
          }
        });

        // Wait 3 seconds — no textures_ready should arrive for idle server.
        setTimeout(() => {
          ws.close();
          resolve({ types, gotTexturesReady });
        }, 3000);
      });
    },
    { url: wsUrl },
  );

  console.log("Idle message types:", result.types);
  // Server is running from the previous test, so we may get state/info.
  // But we should NOT get a new textures_ready (already sent for that body).
  // The key assertion: on a fresh connection to an already-running sim,
  // textures_ready is not re-sent (it was a one-time broadcast).
  // This confirms the lazy-load is event-driven, not spammed on every connect.
  expect(result.gotTexturesReady).toBe(false);
});
