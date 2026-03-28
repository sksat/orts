import { type ChildProcess, spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

let ortsProcess: ChildProcess | undefined;
let wsUrl: string;
let configPath: string;

test.beforeAll(async () => {
  // Always spawn a dedicated server with attitude config — the CI shared
  // server runs orbit-only and cannot satisfy attitude tests.

  // Write a temp config with attitude enabled
  const config = {
    central_body: "earth",
    dt: 1.0,
    output_interval: 10,
    satellites: [
      {
        id: "att-test",
        name: "Attitude Test",
        orbit: { type: "circular", altitude: 400 },
        attitude: { mass: 500, inertia_diag: [100, 100, 50] },
      },
    ],
  };
  configPath = path.join(os.tmpdir(), `orts-attitude-e2e-${Date.now()}.json`);
  fs.writeFileSync(configPath, JSON.stringify(config));

  const binary = process.env.ORTS_BINARY ?? path.resolve(__dirname, "../../target/debug/orts");
  const child = spawn(binary, ["serve", "--port", "0", "--config", configPath]);
  ortsProcess = child;

  const port = await new Promise<number>((resolve, reject) => {
    const rl = createInterface({ input: child.stderr ?? process.stdin });
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
  console.log(`orts attitude server started at ${wsUrl}`);
});

test.afterAll(async () => {
  if (ortsProcess && !ortsProcess.killed) {
    ortsProcess.kill("SIGTERM");
  }
  if (configPath) {
    try {
      fs.unlinkSync(configPath);
    } catch {
      // ignore
    }
  }
});

test("state messages include attitude payload when attitude config is set", async ({ page }) => {
  await page.goto("/?noAutoConnect=1");

  const attitude = await page.evaluate(async (url) => {
    return new Promise<Record<string, unknown>>((resolve) => {
      const ws = new WebSocket(url);
      ws.addEventListener("message", (e) => {
        try {
          const msg = JSON.parse(e.data as string);
          if (msg.type === "state" && msg.attitude != null) {
            ws.close();
            const att = msg.attitude;
            resolve({
              has_quaternion:
                Array.isArray(att.quaternion_wxyz) && att.quaternion_wxyz.length === 4,
              has_angular_velocity:
                Array.isArray(att.angular_velocity_body) && att.angular_velocity_body.length === 3,
              has_source: typeof att.source === "string",
              source: att.source,
              quaternion_is_unit:
                Math.abs(
                  Math.sqrt(
                    att.quaternion_wxyz[0] ** 2 +
                      att.quaternion_wxyz[1] ** 2 +
                      att.quaternion_wxyz[2] ** 2 +
                      att.quaternion_wxyz[3] ** 2,
                  ) - 1.0,
                ) < 0.01,
            });
          }
        } catch {
          // ignore
        }
      });
      setTimeout(() => {
        ws.close();
        resolve({ timeout: true });
      }, 15000);
    });
  }, wsUrl);

  console.log("Attitude payload:", attitude);
  expect(attitude.has_quaternion, "attitude must include quaternion_wxyz[4]").toBe(true);
  expect(attitude.has_angular_velocity, "attitude must include angular_velocity_body[3]").toBe(
    true,
  );
  expect(attitude.has_source, "attitude must include source").toBe(true);
  expect(attitude.source, "source should be propagated").toBe("propagated");
  expect(attitude.quaternion_is_unit, "quaternion must be unit length").toBe(true);
});

test("state messages without attitude config omit attitude field", async ({ page }) => {
  // Connect to a separate server without attitude config
  await page.goto("/?noAutoConnect=1");

  // Start a no-attitude server inline
  const binary = process.env.ORTS_BINARY ?? path.resolve(__dirname, "../../target/debug/orts");
  const child = spawn(binary, ["serve", "--port", "0", "--sat", "altitude=400,id=no-att"]);

  const port = await new Promise<number>((resolve, reject) => {
    const rl = createInterface({ input: child.stderr ?? process.stdin });
    const timeout = setTimeout(() => {
      rl.close();
      reject(new Error("Timed out waiting for no-attitude server"));
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

  const noAttUrl = `ws://localhost:${port}/ws`;

  try {
    const result = await page.evaluate(async (url) => {
      return new Promise<Record<string, unknown>>((resolve) => {
        const ws = new WebSocket(url);
        ws.addEventListener("message", (e) => {
          try {
            const msg = JSON.parse(e.data as string);
            if (msg.type === "state") {
              ws.close();
              resolve({
                attitude_is_null: msg.attitude == null,
              });
            }
          } catch {
            // ignore
          }
        });
        setTimeout(() => {
          ws.close();
          resolve({ timeout: true });
        }, 15000);
      });
    }, noAttUrl);

    expect(result.attitude_is_null, "orbital-only state should not include attitude").toBe(true);
  } finally {
    child.kill("SIGTERM");
  }
});
