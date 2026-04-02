/**
 * Web Worker for CSV parsing.
 *
 * Receives a CSV text string, parses it in chunks, and posts
 * CSVWorkerMessage back to the main thread.
 *
 * Usage: new Worker(new URL("./csvParseWorker.ts", import.meta.url), { type: "module" })
 */

import { parseCSVChunked } from "./csvParseLogic.js";

const CHUNK_SIZE = 5000;

self.onmessage = (e: MessageEvent<{ type: "parse"; text: string }>) => {
  if (e.data.type !== "parse") return;

  try {
    parseCSVChunked(e.data.text, CHUNK_SIZE, (msg) => {
      self.postMessage(msg);
    });
  } catch (err) {
    self.postMessage({
      type: "error",
      message: err instanceof Error ? err.message : String(err),
    });
  }
};
