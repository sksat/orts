import { describe, it, expect } from "vitest";
import type { Crate, Item } from "../src/types.js";
import { LinkResolver, collectApiItems, collectImplementors } from "../src/resolve.js";
import { generateCratePages, firstSentence } from "../src/markdown.js";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

function makeCrate(overrides: Partial<Crate> = {}): Crate {
  return {
    root: 0,
    crate_version: "0.0.0",
    includes_private: false,
    index: {},
    paths: {},
    external_crates: {},
    format_version: 57,
    ...overrides,
  };
}

function makeItem(id: number, name: string, inner: Record<string, unknown>): Item {
  return {
    id,
    name,
    visibility: "public" as const,
    docs: null,
    attrs: [],
    deprecation: null,
    inner,
    span: null,
  } as Item;
}

// ---------------------------------------------------------------------------
// Tuple struct rendering
// ---------------------------------------------------------------------------

describe("tuple struct rendering", () => {
  it("renders tuple struct with its field types in the code block", () => {
    // pub struct Eci(pub Vector3<f64>)
    const crate = makeCrate({
      root: 0,
      index: {
        "0": makeItem(0, "kaname", {
          module: { is_crate: true, items: [1] },
        }),
        "1": makeItem(1, "Eci", {
          struct: {
            kind: { tuple: [2] },
            generics: { params: [], where_predicates: [] },
            impls: [],
          },
        }),
        "2": makeItem(2, "0", {
          struct_field: {
            resolved_path: {
              path: "Vector3",
              id: 999,
              args: {
                angle_bracketed: {
                  args: [{ type: { primitive: "f64" } }],
                  constraints: [],
                },
              },
            },
          },
        }),
      },
    });

    const crates = new Map([["kaname", crate]]);
    const resolver = new LinkResolver(crates, "/orts");

    const items = collectApiItems(crate, "kaname");
    expect(items).toHaveLength(1);
    expect(items[0]!.displayName).toBe("Eci");

    const tmpDir = mkdtempSync(join(tmpdir(), "rustdoc-test-"));
    try {
      const pages = generateCratePages("kaname", items, crate, resolver, {
        contentDir: tmpDir,
        basePath: "/orts",
      });

      const eciPage = readFileSync(join(tmpDir, "kaname/api/structs/eci.md"), "utf-8");
      // Should show tuple struct syntax in code block
      expect(eciPage).toContain("pub struct Eci(");
      expect(eciPage).toContain("Vector3<f64>");
    } finally {
      rmSync(tmpDir, { recursive: true, force: true });
    }
  });
});

// ---------------------------------------------------------------------------
// Plain struct with fields in code block
// ---------------------------------------------------------------------------

describe("plain struct field rendering", () => {
  it("includes fields in the struct code block", () => {
    const crate = makeCrate({
      root: 0,
      index: {
        "0": makeItem(0, "tobari", {
          module: { is_crate: true, items: [1] },
        }),
        "1": makeItem(1, "SpaceWeather", {
          struct: {
            kind: { plain: { fields: [10, 11], has_stripped_fields: false } },
            generics: { params: [], where_predicates: [] },
            impls: [],
          },
        }),
        "10": {
          ...makeItem(10, "f107_daily", { struct_field: { primitive: "f64" } }),
          docs: "Daily F10.7 solar flux.",
        },
        "11": {
          ...makeItem(11, "ap_daily", { struct_field: { primitive: "f64" } }),
          docs: "Daily Ap index.",
        },
      },
    });

    const crates = new Map([["tobari", crate]]);
    const resolver = new LinkResolver(crates, "/orts");
    const items = collectApiItems(crate, "tobari");

    const tmpDir = mkdtempSync(join(tmpdir(), "rustdoc-test-"));
    try {
      generateCratePages("tobari", items, crate, resolver, {
        contentDir: tmpDir,
        basePath: "/orts",
      });

      const page = readFileSync(join(tmpDir, "tobari/api/structs/spaceweather.md"), "utf-8");
      // Code block should contain fields
      expect(page).toContain("pub f107_daily: f64,");
      expect(page).toContain("pub ap_daily: f64,");
      // Fields section should also exist
      expect(page).toContain("## Fields");
      expect(page).toContain("Daily F10.7 solar flux.");
    } finally {
      rmSync(tmpDir, { recursive: true, force: true });
    }
  });
});

// ---------------------------------------------------------------------------
// firstSentence
// ---------------------------------------------------------------------------

describe("firstSentence", () => {
  it("extracts first sentence ending with period + space", () => {
    expect(firstSentence("Hello world. More text here.")).toBe("Hello world.");
  });

  it("does not split on period inside numbers like F10.7", () => {
    expect(firstSentence("Constant space weather — returns the same F10.7 and Ap for all epochs."))
      .toBe("Constant space weather — returns the same F10.7 and Ap for all epochs.");
  });

  it("does not split on period inside version numbers like 0.25", () => {
    expect(firstSentence("Uses nalgebra 0.34 for vectors. Next sentence."))
      .toBe("Uses nalgebra 0.34 for vectors.");
  });

  it("falls back to first line when no period", () => {
    expect(firstSentence("No period here\nSecond line")).toBe("No period here");
  });

  it("returns empty string for null", () => {
    expect(firstSentence(null)).toBe("");
  });

  it("escapes pipe characters for Markdown tables", () => {
    expect(firstSentence("Parse Kp|Ap|SN data. More.")).toBe("Parse Kp\\|Ap\\|SN data.");
  });
});

// ---------------------------------------------------------------------------
// collectImplementors — crate:: prefix stripping
// ---------------------------------------------------------------------------

describe("collectImplementors", () => {
  it("strips crate:: prefix from implementor names", () => {
    const crate = makeCrate({
      index: {
        "10": makeItem(10, null as unknown as string, {
          impl: {
            is_unsafe: false,
            generics: { params: [], where_predicates: [] },
            provided_trait_methods: [],
            trait: { path: "HasOrbit", id: 50, args: null },
            for: { resolved_path: { path: "crate::OrbitalState", id: 100, args: null } },
            items: [],
            is_negative: false,
            blanket_impl: null,
          },
        }),
      },
    });

    const result = collectImplementors(crate, [10]);
    expect(result).toHaveLength(1);
    expect(result[0]!.name).toBe("OrbitalState");
  });
});
