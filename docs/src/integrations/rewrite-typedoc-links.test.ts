import { describe, expect, it } from "vitest";
import {
  computeRelativeUrl,
  filePathToLogicalUrl,
  rewriteAbsoluteTypedocLinks,
} from "./rewrite-typedoc-links";

// Use forward slashes in fake filesystem paths — rewriteAbsoluteTypedocLinks
// normalises separators internally, so the tests stay portable.
const CONTENT_DIR = "/repo/docs/src/content/docs";
const OPTIONS = {
  contentDir: CONTENT_DIR,
  locale: "en",
  typedocRoot: "uneri/api",
  base: "/orts",
};

// ---------------------------------------------------------------------------
// filePathToLogicalUrl
// ---------------------------------------------------------------------------

describe("filePathToLogicalUrl", () => {
  it("turns a README.md at the typedoc root into a readme/ slug", () => {
    expect(filePathToLogicalUrl(`${CONTENT_DIR}/en/uneri/api/README.md`, OPTIONS)).toBe(
      "uneri/api/readme/",
    );
  });

  it("lowercases the final basename for PascalCase files", () => {
    expect(
      filePathToLogicalUrl(`${CONTENT_DIR}/en/uneri/api/classes/ChartBuffer.md`, OPTIONS),
    ).toBe("uneri/api/classes/chartbuffer/");
  });

  it("handles deeply nested files", () => {
    expect(
      filePathToLogicalUrl(`${CONTENT_DIR}/en/uneri/api/interfaces/TableSchema.md`, OPTIONS),
    ).toBe("uneri/api/interfaces/tableschema/");
  });

  it("handles camelCase function files", () => {
    expect(
      filePathToLogicalUrl(`${CONTENT_DIR}/en/uneri/api/functions/alignTimeSeries.md`, OPTIONS),
    ).toBe("uneri/api/functions/aligntimeseries/");
  });
});

// ---------------------------------------------------------------------------
// computeRelativeUrl — mirror of starlight-rustdoc's helper
// ---------------------------------------------------------------------------

describe("computeRelativeUrl", () => {
  it("computes a sibling link", () => {
    expect(
      computeRelativeUrl("uneri/api/classes/chartbuffer/", "uneri/api/classes/ingestbuffer/"),
    ).toBe("../ingestbuffer/");
  });

  it("computes a link from overview to an item page", () => {
    expect(computeRelativeUrl("uneri/api/readme/", "uneri/api/classes/chartbuffer/")).toBe(
      "../classes/chartbuffer/",
    );
  });

  it("computes a link across sibling categories", () => {
    expect(
      computeRelativeUrl("uneri/api/classes/chartbuffer/", "uneri/api/interfaces/tableschema/"),
    ).toBe("../../interfaces/tableschema/");
  });

  it("falls back to root-relative when the source is empty", () => {
    expect(computeRelativeUrl("", "uneri/api/readme/")).toBe("/uneri/api/readme/");
  });
});

// ---------------------------------------------------------------------------
// rewriteAbsoluteTypedocLinks — main integration helper
// ---------------------------------------------------------------------------

describe("rewriteAbsoluteTypedocLinks", () => {
  const README_PATH = `${CONTENT_DIR}/en/uneri/api/README.md`;
  const CHARTBUFFER_PATH = `${CONTENT_DIR}/en/uneri/api/classes/ChartBuffer.md`;

  it("rewrites a single absolute link in the README to a relative path", () => {
    const input = "See [ChartBuffer](/orts/en/uneri/api/classes/chartbuffer/) for details.";
    const output = rewriteAbsoluteTypedocLinks(input, {
      ...OPTIONS,
      filePath: README_PATH,
    });
    expect(output).toBe("See [ChartBuffer](../classes/chartbuffer/) for details.");
  });

  it("rewrites multiple absolute links in one document", () => {
    const input = [
      "- [ChartBuffer](/orts/en/uneri/api/classes/chartbuffer/)",
      "- [IngestBuffer](/orts/en/uneri/api/classes/ingestbuffer/)",
      "- [TableSchema](/orts/en/uneri/api/interfaces/tableschema/)",
    ].join("\n");
    const output = rewriteAbsoluteTypedocLinks(input, {
      ...OPTIONS,
      filePath: README_PATH,
    });
    expect(output).toBe(
      [
        "- [ChartBuffer](../classes/chartbuffer/)",
        "- [IngestBuffer](../classes/ingestbuffer/)",
        "- [TableSchema](../interfaces/tableschema/)",
      ].join("\n"),
    );
  });

  it("rewrites cross-category links from an item page", () => {
    // ChartBuffer.md is at /orts/en/uneri/api/classes/chartbuffer/ — links
    // into interfaces/ or type-aliases/ should back out one extra segment.
    const input = "Produces [ChartDataMap](/orts/en/uneri/api/type-aliases/chartdatamap/).";
    const output = rewriteAbsoluteTypedocLinks(input, {
      ...OPTIONS,
      filePath: CHARTBUFFER_PATH,
    });
    expect(output).toBe("Produces [ChartDataMap](../../type-aliases/chartdatamap/).");
  });

  it("leaves already-relative links untouched", () => {
    const input = "See [ChartBuffer](../classes/chartbuffer/) and [Home](./readme/).";
    const output = rewriteAbsoluteTypedocLinks(input, {
      ...OPTIONS,
      filePath: README_PATH,
    });
    expect(output).toBe(input);
  });

  it("leaves unrelated absolute links untouched", () => {
    // Links into user-written pages outside the typedoc tree are left alone
    // (the rewrite targets only the `uneri/api` subtree).
    const input = "See the [Getting Started](/orts/en/getting-started/) guide.";
    const output = rewriteAbsoluteTypedocLinks(input, {
      ...OPTIONS,
      filePath: README_PATH,
    });
    expect(output).toBe(input);
  });

  it("leaves external URLs untouched", () => {
    const input = "See [docs.rs](https://docs.rs/foo) and [MDN](https://developer.mozilla.org/).";
    const output = rewriteAbsoluteTypedocLinks(input, {
      ...OPTIONS,
      filePath: README_PATH,
    });
    expect(output).toBe(input);
  });

  it("is idempotent under repeated application", () => {
    const input = "See [ChartBuffer](/orts/en/uneri/api/classes/chartbuffer/).";
    const once = rewriteAbsoluteTypedocLinks(input, {
      ...OPTIONS,
      filePath: README_PATH,
    });
    const twice = rewriteAbsoluteTypedocLinks(once, {
      ...OPTIONS,
      filePath: README_PATH,
    });
    expect(twice).toBe(once);
  });

  it("is locale-agnostic: the result does not contain /en/ or /ja/", () => {
    // This is the whole point of the rewriter — no matter which locale the
    // page is eventually served as, its links should not embed any locale
    // segment, so fallback pages keep users in their chosen language.
    const input = "[ChartBuffer](/orts/en/uneri/api/classes/chartbuffer/)";
    const output = rewriteAbsoluteTypedocLinks(input, {
      ...OPTIONS,
      filePath: README_PATH,
    });
    expect(output).not.toContain("/en/");
    expect(output).not.toContain("/ja/");
    expect(output).not.toContain("/orts/");
  });

  it("handles pathological inputs without catastrophic backtracking", () => {
    // Defend against regex regressions — a long line with many candidate
    // patterns should complete quickly and without explosion.
    const parts: string[] = [];
    for (let i = 0; i < 200; i++) {
      parts.push(`[item${i}](/orts/en/uneri/api/classes/item${i}/)`);
    }
    const input = parts.join(" ");
    const start = Date.now();
    const output = rewriteAbsoluteTypedocLinks(input, {
      ...OPTIONS,
      filePath: README_PATH,
    });
    const elapsed = Date.now() - start;
    expect(elapsed).toBeLessThan(500);
    expect(output).toContain("../classes/item0/");
    expect(output).toContain("../classes/item199/");
    expect(output).not.toContain("/orts/en/");
  });
});
