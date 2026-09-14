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

  /** `tsc` output, split into lines with the colours stripped. */
  function tsc(args: string[]): { failed: boolean; lines: string[] } {
    const strip = (text: string) =>
      // biome-ignore lint/suspicious/noControlCharactersInRegex: tsc colours its output
      text.replace(/\x1b\[[0-9;]*m/g, "").split("\n");
    try {
      const out = execFileSync("node_modules/.bin/tsc", ["-p", configName, ...args], {
        cwd: viewerDir,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      });
      return { failed: false, lines: strip(out) };
    } catch (e) {
      const err = e as { stdout?: string; stderr?: string };
      return { failed: true, lines: strip(`${err.stdout ?? ""}${err.stderr ?? ""}`) };
    }
  }

  it("compiles the fixture, and the fixture is what it compiles", () => {
    expect(existsSync(path.join(viewerDir, configName)), `${configName} should exist`).toBe(true);

    // The program's own file list, so an include that stops naming the fixture
    // cannot read as "no errors in the fixture". Without this the assertion
    // below passes on an empty program.
    const listed = tsc(["--listFilesOnly"]);
    expect(listed.failed, `--listFilesOnly failed:\n${listed.lines.join("\n")}`).toBe(false);
    expect(
      listed.lines.some((l) => l.includes(FIXTURE)),
      `${FIXTURE} should be in the compiled program`,
    ).toBe(true);

    const compiled = tsc([]);
    const errors = compiled.lines.filter((l) => / error TS\d+:/.test(l));

    // A diagnostic that names no file is the invocation or the config itself
    // (TS5xxx: a malformed option, no inputs). Those are failures of this check,
    // not results from it, so they are read before anything is filtered.
    const configErrors = errors.filter((l) => !/^[^ ].*\(\d+,\d+\)|^\S+:\d+:\d+/.test(l));
    expect(configErrors.join("\n"), "the config and invocation must be sound").toBe("");

    // Of what remains, only the fixture's own errors answer the question. The
    // internal modules the public types pull in are not written against this
    // flag — measured: 4 TS2412s in `src/orbit.ts`, where an optional component
    // is copied from one point to another — and whether the whole tree could be
    // is a separate question from what the declarations promise.
    //
    // Reported as the compiler's own lines, since a count says only that
    // something failed. Measured against the two ways this can regress:
    // dropping `| undefined` from the supplying arm gives TS2322 on the
    // fixture's first assignment, and letting a rotation through beside a
    // refusal gives TS2578 — the `@ts-expect-error` going unused.
    const onFixture = errors.filter((l) => l.includes(FIXTURE));
    expect(onFixture.join("\n"), "the fixture must compile with the flag on").toBe("");
  }, 120_000);
});
