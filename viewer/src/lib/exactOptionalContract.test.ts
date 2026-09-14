import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

/**
 * `exactOptionalPropertyTypes` is a compiler-wide setting, so no assertion
 * written in this suite can observe it — the suite is compiled with it off. The
 * contract is checked by compiling the fixture again with it on.
 */
describe("the public attitude contract under exactOptionalPropertyTypes", () => {
  // Vitest's root is the viewer package, and the config being compiled lives
  // there. Asserted rather than assumed: a wrong directory would make `tsc`
  // fail for its own reason and read as the contract breaking.
  const viewerDir = process.cwd();
  const configName = "tsconfig.exactOptional.json";
  const FIXTURE = "src/lib/exactOptionalContract.fixture.ts";

  /** Every error `tsc` reported, one line each, with the colours stripped. */
  function compileWithExactOptional(): string[] {
    try {
      execFileSync("node_modules/.bin/tsc", ["-p", configName], {
        cwd: viewerDir,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      });
      return [];
    } catch (e) {
      const err = e as { stdout?: string; stderr?: string };
      // biome-ignore lint/suspicious/noControlCharactersInRegex: tsc colours its output
      const plain = `${err.stdout ?? ""}${err.stderr ?? ""}`.replace(/\x1b\[[0-9;]*m/g, "");
      return plain.split("\n").filter((l) => / error TS\d+:/.test(l));
    }
  }

  it("accepts a Quat | undefined and still rejects a rotation beside a refusal", () => {
    expect(existsSync(path.join(viewerDir, configName)), `${configName} should exist`).toBe(true);

    // Only the fixture's own errors answer the question. The internal modules
    // the public types pull in are not written against this flag — measured: 4
    // TS2412s in `src/orbit.ts`, where an optional component is copied from one
    // point to another — and whether the whole tree could be is a separate
    // question from what the declarations promise.
    //
    // Reported as the compiler's own lines, since a count says only that
    // something failed. Measured against the two ways this can regress:
    // dropping `| undefined` from the supplying arm gives TS2322 on the
    // fixture's first assignment, and letting a rotation through beside a
    // refusal gives TS2578 — the `@ts-expect-error` below going unused.
    const onFixture = compileWithExactOptional().filter((l) => l.includes(FIXTURE));

    expect(onFixture.join("\n"), "the fixture must compile with the flag on").toBe("");
  }, 120_000);
});
