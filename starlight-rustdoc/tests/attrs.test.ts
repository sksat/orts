import { describe, expect, it } from "vitest";
import { extractCfgConditions, renderCfgBadge } from "../src/attrs.js";
import type { Attribute } from "../src/types.js";

// ---------------------------------------------------------------------------
// extractCfgConditions
// ---------------------------------------------------------------------------

describe("extractCfgConditions", () => {
  it("returns empty array for empty attrs", () => {
    expect(extractCfgConditions([])).toEqual([]);
  });

  it("skips simple string attributes (automatically_derived, etc.)", () => {
    const attrs: Attribute[] = ["automatically_derived", "non_exhaustive"];
    expect(extractCfgConditions(attrs)).toEqual([]);
  });

  it("skips non-CfgTrace object attributes", () => {
    const attrs: Attribute[] = [
      { other: "#[allow(clippy::excessive_precision)]" },
      { other: "#[attr = Inline(Hint)]" },
      { repr: { kind: "transparent", align: null, packed: null, int: null } },
    ];
    expect(extractCfgConditions(attrs)).toEqual([]);
  });

  it("extracts single feature condition from CfgTrace", () => {
    const attrs: Attribute[] = [
      {
        other:
          '#[attr = CfgTrace([NameValue { name: "feature", value: Some("alloc"), span: arika/src/lib.rs:13:7: 13:24 (#0) }])]',
      },
    ];
    const result = extractCfgConditions(attrs);
    expect(result).toEqual([{ label: "alloc", kind: "feature", negated: false }]);
  });

  it("extracts std feature condition", () => {
    const attrs: Attribute[] = [
      {
        other:
          '#[attr = CfgTrace([NameValue { name: "feature", value: Some("std"), span: arika/src/epoch.rs:412:11: 412:26 (#0) }])]',
      },
    ];
    const result = extractCfgConditions(attrs);
    expect(result).toEqual([{ label: "std", kind: "feature", negated: false }]);
  });

  it("extracts platform condition (target_arch)", () => {
    const attrs: Attribute[] = [
      {
        other:
          '#[attr = CfgTrace([NameValue { name: "target_arch", value: Some("wasm32"), span: foo.rs:1:1: 1:10 (#0) }])]',
      },
    ];
    const result = extractCfgConditions(attrs);
    expect(result).toEqual([{ label: "wasm32", kind: "platform", negated: false }]);
  });

  it("extracts platform condition (target_os)", () => {
    const attrs: Attribute[] = [
      {
        other:
          '#[attr = CfgTrace([NameValue { name: "target_os", value: Some("linux"), span: foo.rs:1:1: 1:10 (#0) }])]',
      },
    ];
    const result = extractCfgConditions(attrs);
    expect(result).toEqual([{ label: "linux", kind: "platform", negated: false }]);
  });

  it("deduplicates identical conditions", () => {
    const attrs: Attribute[] = [
      {
        other:
          '#[attr = CfgTrace([NameValue { name: "feature", value: Some("alloc"), span: foo.rs:1:1: 1:10 (#0) }])]',
      },
      {
        other:
          '#[attr = CfgTrace([NameValue { name: "feature", value: Some("alloc"), span: bar.rs:2:1: 2:10 (#0) }])]',
      },
    ];
    const result = extractCfgConditions(attrs);
    expect(result).toEqual([{ label: "alloc", kind: "feature", negated: false }]);
  });

  it("extracts multiple different conditions from mixed attrs", () => {
    const attrs: Attribute[] = [
      "automatically_derived",
      {
        other:
          '#[attr = CfgTrace([NameValue { name: "feature", value: Some("alloc"), span: foo.rs:1:1: 1:10 (#0) }])]',
      },
      { other: "#[allow(dead_code)]" },
      {
        other:
          '#[attr = CfgTrace([NameValue { name: "feature", value: Some("std"), span: bar.rs:2:1: 2:10 (#0) }])]',
      },
    ];
    const result = extractCfgConditions(attrs);
    expect(result).toEqual([
      { label: "alloc", kind: "feature", negated: false },
      { label: "std", kind: "feature", negated: false },
    ]);
  });

  it("handles unknown name as 'other' kind", () => {
    const attrs: Attribute[] = [
      {
        other:
          '#[attr = CfgTrace([NameValue { name: "custom_flag", value: Some("yes"), span: foo.rs:1:1: 1:10 (#0) }])]',
      },
    ];
    const result = extractCfgConditions(attrs);
    expect(result).toEqual([{ label: 'custom_flag = "yes"', kind: "other", negated: false }]);
  });

  it("detects negated conditions from Not(...) wrapper", () => {
    const attrs: Attribute[] = [
      {
        other:
          '#[attr = CfgTrace([All([NameValue { name: "feature", value: Some("fetch-horizons"), span: foo.rs:1:1: 1:10 (#0) }, Not(NameValue { name: "target_arch", value: Some("wasm32"), span: foo.rs:1:1: 1:10 (#0) }, foo.rs:1:1: 1:10 (#0))], foo.rs:1:1: 1:10 (#0))])]',
      },
    ];
    const result = extractCfgConditions(attrs);
    expect(result).toEqual([
      { label: "fetch-horizons", kind: "feature", negated: false },
      { label: "wasm32", kind: "platform", negated: true },
    ]);
  });

  it("deduplicates negated and non-negated as separate conditions", () => {
    const attrs: Attribute[] = [
      {
        other:
          '#[attr = CfgTrace([NameValue { name: "target_arch", value: Some("wasm32"), span: foo.rs:1:1: 1:10 (#0) }])]',
      },
      {
        other:
          '#[attr = CfgTrace([Not(NameValue { name: "target_arch", value: Some("wasm32"), span: bar.rs:1:1: 1:10 (#0) })])]',
      },
    ];
    const result = extractCfgConditions(attrs);
    expect(result).toEqual([
      { label: "wasm32", kind: "platform", negated: false },
      { label: "wasm32", kind: "platform", negated: true },
    ]);
  });
});

// ---------------------------------------------------------------------------
// renderCfgBadge
// ---------------------------------------------------------------------------

describe("renderCfgBadge", () => {
  it("returns empty string for no conditions", () => {
    expect(renderCfgBadge([])).toBe("");
  });

  it("renders single feature badge", () => {
    const result = renderCfgBadge([{ label: "alloc", kind: "feature", negated: false }]);
    expect(result).toBe("> **Available on crate feature `alloc` only.**\n");
  });

  it("renders two feature badges with 'and'", () => {
    const result = renderCfgBadge([
      { label: "std", kind: "feature", negated: false },
      { label: "fetch-horizons", kind: "feature", negated: false },
    ]);
    expect(result).toBe("> **Available on crate features `std` and `fetch-horizons` only.**\n");
  });

  it("renders three feature badges with commas and 'and'", () => {
    const result = renderCfgBadge([
      { label: "a", kind: "feature", negated: false },
      { label: "b", kind: "feature", negated: false },
      { label: "c", kind: "feature", negated: false },
    ]);
    expect(result).toBe("> **Available on crate features `a`, `b` and `c` only.**\n");
  });

  it("renders platform badge", () => {
    const result = renderCfgBadge([{ label: "wasm32", kind: "platform", negated: false }]);
    expect(result).toBe("> **Available on `wasm32` only.**\n");
  });

  it("renders negated platform badge with non- prefix", () => {
    const result = renderCfgBadge([{ label: "wasm32", kind: "platform", negated: true }]);
    expect(result).toBe("> **Available on non-`wasm32` only.**\n");
  });

  it("renders combined feature and negated platform badges", () => {
    const result = renderCfgBadge([
      { label: "fetch-horizons", kind: "feature", negated: false },
      { label: "wasm32", kind: "platform", negated: true },
    ]);
    expect(result).toBe(
      "> **Available on crate feature `fetch-horizons` only. Available on non-`wasm32` only.**\n",
    );
  });

  it("excludes negated features from feature list", () => {
    const result = renderCfgBadge([{ label: "broken", kind: "feature", negated: true }]);
    // Negated features are not shown in the feature section
    expect(result).toBe("");
  });

  it("renders 'other' kind badges", () => {
    const result = renderCfgBadge([{ label: 'docsrs = "true"', kind: "other", negated: false }]);
    expect(result).toBe('> **Available when `docsrs = "true"`.**\n');
  });
});
