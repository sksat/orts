import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { firstSentence, generateCratePages } from "../src/markdown.js";
import type { ApiItem } from "../src/resolve.js";
import { collectApiItems, collectImplementors, LinkResolver } from "../src/resolve.js";
import type { Crate, Item } from "../src/types.js";

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
// Enum variant rendering
// ---------------------------------------------------------------------------

describe("enum variant rendering", () => {
  it("renders all three variant kinds: unit, tuple, and struct", () => {
    // pub enum Outcome<Y, B> {
    //   Completed(Y),
    //   Terminated { state: Y, t: f64, reason: B },
    //   Unknown,
    // }
    const crate = makeCrate({
      root: 0,
      index: {
        "0": makeItem(0, "mycrate", {
          module: { is_crate: true, items: [1] },
        }),
        "1": makeItem(1, "Outcome", {
          enum: {
            generics: {
              params: [
                { name: "Y", kind: { type: { bounds: [], default: null, is_synthetic: false } } },
                { name: "B", kind: { type: { bounds: [], default: null, is_synthetic: false } } },
              ],
              where_predicates: [],
            },
            variants: [10, 11, 12],
            impls: [],
          },
        }),
        // Tuple variant: Completed(Y)
        "10": {
          ...makeItem(10, "Completed", {
            variant: {
              kind: { tuple: [20] },
              discriminant: null,
            },
          }),
          docs: "Completed successfully.",
        },
        "20": makeItem(20, "0", { struct_field: { generic: "Y" } }),
        // Struct variant: Terminated { state: Y, t: f64, reason: B }
        "11": {
          ...makeItem(11, "Terminated", {
            variant: {
              kind: { struct: { fields: [30, 31, 32], has_stripped_fields: false } },
              discriminant: null,
            },
          }),
          docs: "Terminated early.",
        },
        "30": makeItem(30, "state", { struct_field: { generic: "Y" } }),
        "31": makeItem(31, "t", { struct_field: { primitive: "f64" } }),
        "32": makeItem(32, "reason", { struct_field: { generic: "B" } }),
        // Unit variant: Unknown
        "12": {
          ...makeItem(12, "Unknown", {
            variant: { kind: "plain", discriminant: null },
          }),
          docs: "Unknown outcome.",
        },
      },
    });

    const crates = new Map([["mycrate", crate]]);
    const resolver = new LinkResolver(crates, "/orts");
    const items = collectApiItems(crate, "mycrate");

    const tmpDir = mkdtempSync(join(tmpdir(), "rustdoc-test-"));
    try {
      generateCratePages("mycrate", items, crate, resolver, {
        contentDir: tmpDir,
        basePath: "/orts",
      });

      const page = readFileSync(join(tmpDir, "mycrate/api/enums/outcome.md"), "utf-8");

      // Code block should show all variants
      expect(page).toContain("Completed(Y)");
      expect(page).toContain("Terminated {");
      expect(page).toContain("state: Y");
      expect(page).toContain("t: f64");
      expect(page).toContain("reason: B");
      expect(page).toContain("Unknown,");

      // Variants section should show docs
      expect(page).toContain("Completed successfully.");
      expect(page).toContain("Terminated early.");
      expect(page).toContain("Unknown outcome.");

      // Struct variant fields should be listed
      expect(page).toContain("**state**: Y");
      expect(page).toContain("**t**: f64");
      expect(page).toContain("**reason**: B");
    } finally {
      rmSync(tmpDir, { recursive: true, force: true });
    }
  });
});

// ---------------------------------------------------------------------------
// i18n locale prefix & locale-agnostic (fallback-safe) links
// ---------------------------------------------------------------------------

describe("i18n locale prefix & locale-agnostic links", () => {
  function makeKanameCrate(): Crate {
    // Minimal two-item crate so the overview page table has something to
    // link between, allowing us to verify cross-reference URLs.
    return makeCrate({
      root: 0,
      index: {
        "0": makeItem(0, "kaname", {
          module: { is_crate: true, items: [1, 3] },
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
        "3": makeItem(3, "Epoch", {
          struct: {
            kind: "unit",
            generics: { params: [], where_predicates: [] },
            impls: [],
          },
        }),
      },
    });
  }

  function registerLogical(resolver: LinkResolver, items: ApiItem[]): void {
    for (const item of items) {
      const slug = item.displayName.toLowerCase();
      const category = item.category === "type_alias" ? "type-aliases" : `${item.category}s`;
      resolver.registerPage(
        item.item.id,
        `${item.crateName}/api/${category}/${slug}/`,
        item.displayName,
        item.crateName,
      );
    }
  }

  it("writes item pages under the locale subdirectory when locale is set", () => {
    const crate = makeKanameCrate();
    const crates = new Map([["kaname", crate]]);
    const resolver = new LinkResolver(crates, "/orts");
    const items = collectApiItems(crate, "kaname");
    registerLogical(resolver, items);

    const tmpDir = mkdtempSync(join(tmpdir(), "rustdoc-test-"));
    try {
      generateCratePages("kaname", items, crate, resolver, {
        contentDir: tmpDir,
        basePath: "/orts",
        locale: "en",
      });

      // File should be written under the locale directory
      const localized = readFileSync(join(tmpDir, "en/kaname/api/structs/eci.md"), "utf-8");
      expect(localized).toContain("pub struct Eci(");
    } finally {
      rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it("overview links are relative (no locale or base baked in) when locale=en", () => {
    const crate = makeKanameCrate();
    const crates = new Map([["kaname", crate]]);
    const resolver = new LinkResolver(crates, "/orts");
    const items = collectApiItems(crate, "kaname");
    registerLogical(resolver, items);

    const tmpDir = mkdtempSync(join(tmpdir(), "rustdoc-test-"));
    try {
      generateCratePages("kaname", items, crate, resolver, {
        contentDir: tmpDir,
        basePath: "/orts",
        locale: "en",
      });

      const overview = readFileSync(join(tmpDir, "en/kaname/api/overview.md"), "utf-8");
      // Cross-reference links in the overview table must be relative so
      // that Starlight i18n fallback pages keep users in their chosen
      // locale. No `/orts/`, `/en/`, or `/ja/` segment should appear in
      // the URL of internal links.
      expect(overview).toMatch(/\]\(\.\.\/structs\/eci\/\)/);
      expect(overview).toMatch(/\]\(\.\.\/structs\/epoch\/\)/);
      expect(overview).not.toContain("(/orts/");
      expect(overview).not.toContain("(/en/");
      expect(overview).not.toContain("(/ja/");
    } finally {
      rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it("overview links stay identical when locale=ja (fallback-safe)", () => {
    // This test encodes the key fallback guarantee: whichever locale the
    // plugin writes to, generated cross-references are locale-agnostic so
    // that the same content works when served via the opposite locale's
    // fallback route.
    const crate = makeKanameCrate();
    const crates = new Map([["kaname", crate]]);
    const resolver = new LinkResolver(crates, "/orts");
    const items = collectApiItems(crate, "kaname");
    registerLogical(resolver, items);

    const tmpDir = mkdtempSync(join(tmpdir(), "rustdoc-test-"));
    try {
      generateCratePages("kaname", items, crate, resolver, {
        contentDir: tmpDir,
        basePath: "/orts",
        locale: "ja",
      });

      // Files go to ja/... but link content is identical to the en case.
      const overview = readFileSync(join(tmpDir, "ja/kaname/api/overview.md"), "utf-8");
      expect(overview).toMatch(/\]\(\.\.\/structs\/eci\/\)/);
      expect(overview).toMatch(/\]\(\.\.\/structs\/epoch\/\)/);
      expect(overview).not.toContain("/ja/");
      expect(overview).not.toContain("/en/");
    } finally {
      rmSync(tmpDir, { recursive: true, force: true });
    }
  });

  it("overview and item file contents are byte-identical for locale=en vs locale=ja", () => {
    // Direct proof of the "fallback-safe" guarantee: regardless of locale,
    // the generated file bytes are the same — so linking works correctly
    // both when the file is served from its native locale and when it is
    // served via Starlight i18n fallback as the other locale.
    const makeCrates = () => {
      const crate = makeKanameCrate();
      const crates = new Map([["kaname", crate]]);
      return { crate, crates };
    };

    const { crate: enCrate, crates: enCrates } = makeCrates();
    const enResolver = new LinkResolver(enCrates, "/orts");
    const enItems = collectApiItems(enCrate, "kaname");
    registerLogical(enResolver, enItems);

    const { crate: jaCrate, crates: jaCrates } = makeCrates();
    const jaResolver = new LinkResolver(jaCrates, "/orts");
    const jaItems = collectApiItems(jaCrate, "kaname");
    registerLogical(jaResolver, jaItems);

    const enDir = mkdtempSync(join(tmpdir(), "rustdoc-en-"));
    const jaDir = mkdtempSync(join(tmpdir(), "rustdoc-ja-"));
    try {
      generateCratePages("kaname", enItems, enCrate, enResolver, {
        contentDir: enDir,
        basePath: "/orts",
        locale: "en",
      });
      generateCratePages("kaname", jaItems, jaCrate, jaResolver, {
        contentDir: jaDir,
        basePath: "/orts",
        locale: "ja",
      });

      const enOverview = readFileSync(join(enDir, "en/kaname/api/overview.md"), "utf-8");
      const jaOverview = readFileSync(join(jaDir, "ja/kaname/api/overview.md"), "utf-8");
      expect(jaOverview).toBe(enOverview);

      const enEci = readFileSync(join(enDir, "en/kaname/api/structs/eci.md"), "utf-8");
      const jaEci = readFileSync(join(jaDir, "ja/kaname/api/structs/eci.md"), "utf-8");
      expect(jaEci).toBe(enEci);
    } finally {
      rmSync(enDir, { recursive: true, force: true });
      rmSync(jaDir, { recursive: true, force: true });
    }
  });

  it("preserves root-level file layout when locale is not set", () => {
    const crate = makeKanameCrate();
    const crates = new Map([["kaname", crate]]);
    const resolver = new LinkResolver(crates, "/orts");
    const items = collectApiItems(crate, "kaname");
    registerLogical(resolver, items);

    const tmpDir = mkdtempSync(join(tmpdir(), "rustdoc-test-"));
    try {
      generateCratePages("kaname", items, crate, resolver, {
        contentDir: tmpDir,
        basePath: "/orts",
      });

      // Default behaviour: no locale directory
      const root = readFileSync(join(tmpDir, "kaname/api/structs/eci.md"), "utf-8");
      expect(root).toContain("pub struct Eci(");
      const overview = readFileSync(join(tmpDir, "kaname/api/overview.md"), "utf-8");
      // Links remain relative even with no locale — they were relative all
      // along; the locale option only controls file placement.
      expect(overview).toMatch(/\]\(\.\.\/structs\/eci\/\)/);
      expect(overview).not.toContain("/orts/");
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
    expect(
      firstSentence("Constant space weather — returns the same F10.7 and Ap for all epochs."),
    ).toBe("Constant space weather — returns the same F10.7 and Ap for all epochs.");
  });

  it("does not split on period inside version numbers like 0.25", () => {
    expect(firstSentence("Uses nalgebra 0.34 for vectors. Next sentence.")).toBe(
      "Uses nalgebra 0.34 for vectors.",
    );
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
