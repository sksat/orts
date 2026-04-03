/**
 * Web Worker for parsing RRD files via WASM.
 *
 * Receives an ArrayBuffer, decodes it with rrd-wasm, and sends back
 * metadata + chunked point data to the main thread.
 */

import { initRrdWasm, parseRrd } from "../wasm/rrdWasmInit.js";
import type { RrdPointOut, RrdWorkerInput, RrdWorkerMessage } from "./rrdParseLogic.js";

const CHUNK_SIZE = 5000;

function post(msg: RrdWorkerMessage) {
  self.postMessage(msg);
}

self.onmessage = async (e: MessageEvent<RrdWorkerInput>) => {
  if (e.data.type !== "parse") return;

  try {
    await initRrdWasm();

    const bytes = new Uint8Array(e.data.buffer);
    const data = parseRrd(bytes);

    // Send metadata first
    post({ type: "metadata", metadata: data.metadata });

    // Convert rows to points and send in chunks
    let chunk: RrdPointOut[] = [];

    for (const row of data.rows) {
      const point: RrdPointOut = {
        t: row.t,
        x: row.x,
        y: row.y,
        z: row.z,
        vx: row.vx,
        vy: row.vy,
        vz: row.vz,
        entityPath: row.entity_path,
      };

      // Attitude data (optional)
      if (row.quaternion) {
        point.qw = row.quaternion[0];
        point.qx = row.quaternion[1];
        point.qy = row.quaternion[2];
        point.qz = row.quaternion[3];
      }
      if (row.angular_velocity) {
        point.wx = row.angular_velocity[0];
        point.wy = row.angular_velocity[1];
        point.wz = row.angular_velocity[2];
      }

      chunk.push(point);

      if (chunk.length >= CHUNK_SIZE) {
        post({ type: "chunk", points: chunk, done: false });
        chunk = [];
      }
    }

    // Final chunk
    post({ type: "chunk", points: chunk, done: true });
  } catch (err) {
    post({ type: "error", message: String(err) });
  }
};
