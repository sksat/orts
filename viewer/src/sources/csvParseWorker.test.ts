import { describe, expect, it } from "vitest";
import { type CSVWorkerMessage, parseCSVChunked } from "./csvParseLogic.js";

describe("parseCSVChunked", () => {
  it("parses metadata from comment headers", () => {
    const csv = `# epoch_jd = 2451545.0
# mu = 398600.4418 km^3/s^2
# central_body = earth
# central_body_radius = 6378.137 km
0,7000,0,0,0,7.5,0
10,7000,100,50,-0.1,7.5,0.01`;

    const messages: CSVWorkerMessage[] = [];
    parseCSVChunked(csv, 5000, (msg) => messages.push(msg));

    const meta = messages.find((m) => m.type === "metadata");
    expect(meta).toBeDefined();
    if (meta?.type === "metadata") {
      expect(meta.metadata.epochJd).toBe(2451545.0);
      expect(meta.metadata.mu).toBe(398600.4418);
      expect(meta.metadata.centralBody).toBe("earth");
    }
  });

  it("emits chunks of points", () => {
    const lines = ["# comment"];
    for (let i = 0; i < 100; i++) {
      lines.push(`${i * 10},${7000 + i},0,0,0,7.5,0`);
    }
    const csv = lines.join("\n");

    const messages: CSVWorkerMessage[] = [];
    parseCSVChunked(csv, 30, (msg) => messages.push(msg));

    const chunks = messages.filter((m) => m.type === "chunk");
    expect(chunks.length).toBeGreaterThan(1); // 100 points / 30 per chunk

    let totalPoints = 0;
    for (const chunk of chunks) {
      if (chunk.type === "chunk") totalPoints += chunk.points.length;
    }
    expect(totalPoints).toBe(100);
  });

  it("emits complete message with total count", () => {
    const csv = `0,7000,0,0,0,7.5,0
10,7000,100,50,-0.1,7.5,0.01`;

    const messages: CSVWorkerMessage[] = [];
    parseCSVChunked(csv, 5000, (msg) => messages.push(msg));

    const complete = messages.find((m) => m.type === "complete");
    expect(complete).toBeDefined();
    if (complete?.type === "complete") {
      expect(complete.totalPoints).toBe(2);
    }
  });

  it("emits metadata before chunks", () => {
    const csv = `# epoch_jd = 2451545.0
0,7000,0,0,0,7.5,0`;

    const messages: CSVWorkerMessage[] = [];
    parseCSVChunked(csv, 5000, (msg) => messages.push(msg));

    expect(messages[0].type).toBe("metadata");
    expect(messages[1].type).toBe("chunk");
    expect(messages[2].type).toBe("complete");
  });

  it("skips invalid lines without error", () => {
    const csv = `0,7000,0,0,0,7.5,0
invalid,data
10,7000,100,50,-0.1,7.5,0.01`;

    const messages: CSVWorkerMessage[] = [];
    parseCSVChunked(csv, 5000, (msg) => messages.push(msg));

    const complete = messages.find((m) => m.type === "complete");
    if (complete?.type === "complete") {
      expect(complete.totalPoints).toBe(2); // invalid line skipped
    }
  });

  it("handles empty file", () => {
    const messages: CSVWorkerMessage[] = [];
    parseCSVChunked("", 5000, (msg) => messages.push(msg));

    const complete = messages.find((m) => m.type === "complete");
    expect(complete).toBeDefined();
    if (complete?.type === "complete") {
      expect(complete.totalPoints).toBe(0);
    }
  });
});
