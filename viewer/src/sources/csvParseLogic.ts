/**
 * CSV parse logic extracted as pure functions.
 *
 * Used by both the Web Worker (csvParseWorker.ts) and tests.
 * No DOM, Worker, or React dependencies.
 */

import type { CSVMetadata, OrbitPoint } from "../orbit.js";
import { emptyMetadata, parseDataLine, parseMetadataLine } from "./parseCSVLine.js";

// ---------------------------------------------------------------------------
// Worker message protocol
// ---------------------------------------------------------------------------

export type CSVWorkerMessage =
  | { type: "metadata"; metadata: CSVMetadata }
  | { type: "chunk"; points: OrbitPoint[] }
  | { type: "complete"; totalPoints: number }
  | { type: "error"; message: string };

// ---------------------------------------------------------------------------
// Chunked parser (pure function)
// ---------------------------------------------------------------------------

/**
 * Parse a CSV string in chunks, emitting messages via the callback.
 *
 * Message order: metadata → chunk* → complete
 *
 * @param text Full CSV text
 * @param chunkSize Maximum points per chunk message
 * @param emit Callback to emit messages (in Worker: postMessage)
 */
export function parseCSVChunked(
  text: string,
  chunkSize: number,
  emit: (msg: CSVWorkerMessage) => void,
): void {
  const metadata = emptyMetadata();
  const lines = text.split("\n");
  let totalPoints = 0;
  let chunk: OrbitPoint[] = [];

  // First pass: extract metadata from comment lines at the top
  let dataStart = 0;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line === "") continue;
    if (line.startsWith("#")) {
      parseMetadataLine(line, metadata);
      continue;
    }
    dataStart = i;
    break;
  }

  // Emit metadata first
  emit({ type: "metadata", metadata });

  // Detect multi-satellite mode.
  // When `# satellites = ...` is present, the CSV always has a satellite_id
  // first column, even for single-satellite files (matches orts run output).
  const multiSat = metadata.satellites != null && metadata.satellites.length > 0;

  // Parse data lines in chunks
  for (let i = dataStart; i < lines.length; i++) {
    const line = lines[i].trim();
    if (line === "" || line.startsWith("#")) continue;

    const point = parseDataLine(line, multiSat);
    if (!point) continue;

    chunk.push(point);
    totalPoints++;

    if (chunk.length >= chunkSize) {
      emit({ type: "chunk", points: chunk });
      chunk = [];
    }
  }

  // Emit remaining points
  if (chunk.length > 0) {
    emit({ type: "chunk", points: chunk });
  }

  emit({ type: "complete", totalPoints });
}
