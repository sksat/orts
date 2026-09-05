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

      // Attitude data (optional). A column that is present but not four long
      // would leave the missing components undefined, and a sample carrying only
      // some of them reads downstream as no attitude at all — which lets a
      // registered model stand at its own orientation, and scales the scene to
      // that model. The claim arrives whole instead: `NaN` is what those
      // components are, the display frame refuses them, and the spacecraft gets
      // the marker that shows no orientation.
      if (row.quaternion) {
        const complete = row.quaternion.length === 4;
        point.qw = complete ? row.quaternion[0] : Number.NaN;
        point.qx = complete ? row.quaternion[1] : Number.NaN;
        point.qy = complete ? row.quaternion[2] : Number.NaN;
        point.qz = complete ? row.quaternion[3] : Number.NaN;
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
