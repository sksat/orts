import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { rewriteUrl } from "./remark-rewrite-root-md-links";

const REPO_ROOT = resolve(fileURLToPath(import.meta.url), "../../../..");

describe("rewriteUrl", () => {
  it("rewrites an existing top-level file to a blob URL", () => {
    expect(rewriteUrl("DESIGN.md", REPO_ROOT)).toBe(
      "https://github.com/sksat/orts/blob/main/DESIGN.md",
    );
  });

  it("rewrites an existing nested file to a blob URL", () => {
    expect(rewriteUrl("orts/wit/v0/orts.wit", REPO_ROOT)).toBe(
      "https://github.com/sksat/orts/blob/main/orts/wit/v0/orts.wit",
    );
  });

  it("rewrites an existing directory (trailing slash) to a tree URL", () => {
    expect(rewriteUrl("orts/", REPO_ROOT)).toBe("https://github.com/sksat/orts/tree/main/orts");
  });

  it("preserves fragments on the rewritten URL", () => {
    expect(rewriteUrl("DESIGN.md#section", REPO_ROOT)).toBe(
      "https://github.com/sksat/orts/blob/main/DESIGN.md#section",
    );
  });

  it("returns null for external URLs", () => {
    expect(rewriteUrl("https://example.com", REPO_ROOT)).toBeNull();
    expect(rewriteUrl("mailto:x@example.com", REPO_ROOT)).toBeNull();
  });

  it("returns null for fragments and absolute paths", () => {
    expect(rewriteUrl("#anchor", REPO_ROOT)).toBeNull();
    expect(rewriteUrl("/some/path", REPO_ROOT)).toBeNull();
  });

  it("returns null for paths that do not exist on disk", () => {
    expect(rewriteUrl("does-not-exist.md", REPO_ROOT)).toBeNull();
    expect(rewriteUrl("orts/does-not-exist/", REPO_ROOT)).toBeNull();
  });

  it("returns null for paths that escape the mdDir", () => {
    expect(rewriteUrl("../outside.md", REPO_ROOT)).toBeNull();
  });
});
