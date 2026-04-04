import { describe, it, expect, beforeEach } from "vitest";
import type { Crate, Id } from "../src/types.js";
import { LinkResolver, collectApiItems, collectTraitImpls, resolveTraitImplUrl } from "../src/resolve.js";

// ---------------------------------------------------------------------------
// Helpers to build minimal Crate fixtures
// ---------------------------------------------------------------------------

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

function makeItem(id: number, name: string, inner: Record<string, unknown>) {
  return {
    id,
    name,
    visibility: "public" as const,
    docs: null,
    attrs: [],
    deprecation: null,
    inner,
    span: null,
  };
}

// ---------------------------------------------------------------------------
// LinkResolver.resolveId
// ---------------------------------------------------------------------------

describe("LinkResolver.resolveId", () => {
  it("resolves registered page IDs", () => {
    const crates = new Map<string, Crate>();
    const crate = makeCrate();
    crates.set("mycrate", crate);
    const resolver = new LinkResolver(crates, "/base");
    resolver.registerPage(42, "/base/mycrate/api/structs/foo/", "Foo", "mycrate");

    expect(resolver.resolveId(42, crate)).toBe("/base/mycrate/api/structs/foo/");
  });

  it("does not confuse items with same ID from different crates", () => {
    // utsuroi ID 158 = Rk4, kaname ID 158 = MU_EARTH
    // When resolving from utsuroi context, ID 158 should be Rk4
    const utsuroi = makeCrate();
    const kaname = makeCrate();
    const crates = new Map([["utsuroi", utsuroi], ["kaname", kaname]]);
    const resolver = new LinkResolver(crates, "/base");

    // Register both pages — same numeric ID but from different crates
    resolver.registerPage(158, "/base/utsuroi/api/structs/rk4/", "Rk4", "utsuroi");
    resolver.registerPage(158, "/base/kaname/api/constants/mu_earth/", "MU_EARTH", "kaname");

    // Resolving ID 158 from utsuroi context should give Rk4
    expect(resolver.resolveId(158, utsuroi, "utsuroi")).toBe("/base/utsuroi/api/structs/rk4/");
    // Resolving ID 158 from kaname context should give MU_EARTH
    expect(resolver.resolveId(158, kaname, "kaname")).toBe("/base/kaname/api/constants/mu_earth/");
  });

  it("resolves std traits to doc.rust-lang.org", () => {
    const crate = makeCrate({
      paths: {
        "100": { crate_id: 2, path: ["core", "marker", "Send"], kind: "trait" },
      },
      external_crates: {
        "2": { name: "core", html_root_url: "https://doc.rust-lang.org/nightly/" },
      },
    });
    const crates = new Map([["mycrate", crate]]);
    const resolver = new LinkResolver(crates, "/base");

    const url = resolver.resolveId(100, crate);
    expect(url).toContain("doc.rust-lang.org");
    expect(url).toContain("trait.Send.html");
  });

  it("does NOT use global external_crates — uses per-crate table", () => {
    // crate A has external_crate 5 = "nalgebra"
    // crate B has external_crate 5 = "serde"
    // resolveId from crate A should use nalgebra, not serde
    const crateA = makeCrate({
      paths: {
        "200": { crate_id: 5, path: ["nalgebra", "SVector"], kind: "struct" },
      },
      external_crates: {
        "5": { name: "nalgebra", html_root_url: null },
      },
    });
    const crateB = makeCrate({
      external_crates: {
        "5": { name: "serde", html_root_url: null },
      },
    });
    const crates = new Map([["crateA", crateA], ["crateB", crateB]]);
    const resolver = new LinkResolver(crates, "/base");

    const url = resolver.resolveId(200, crateA);
    expect(url).toContain("nalgebra");
    expect(url).not.toContain("serde");
  });

  it("resolves cross-crate local references to internal pages", () => {
    // orts crate references kaname::Epoch — should resolve to internal page
    const kanameCrate = makeCrate();
    const ortsCrate = makeCrate({
      paths: {
        "300": { crate_id: 10, path: ["kaname", "epoch", "Epoch"], kind: "struct" },
      },
      external_crates: {
        "10": { name: "kaname", html_root_url: null },
      },
    });
    const crates = new Map([["kaname", kanameCrate], ["orts", ortsCrate]]);
    const resolver = new LinkResolver(crates, "/base");
    resolver.registerPage(999, "/base/kaname/api/structs/epoch/", "Epoch", "kaname");

    const url = resolver.resolveId(300, ortsCrate);
    expect(url).toBe("/base/kaname/api/structs/epoch/");
  });

  it("falls back to docs.rs for unknown external crates", () => {
    const crate = makeCrate({
      paths: {
        "400": { crate_id: 20, path: ["serde", "Serialize"], kind: "trait" },
      },
      external_crates: {
        "20": { name: "serde", html_root_url: null },
      },
    });
    const crates = new Map([["mycrate", crate]]);
    const resolver = new LinkResolver(crates, "/base");

    const url = resolver.resolveId(400, crate);
    expect(url).toBe("https://docs.rs/serde/latest/serde/trait.Serialize.html");
  });

  it("does not double the crate name in docs.rs URLs", () => {
    const crate = makeCrate({
      paths: {
        "500": { crate_id: 20, path: ["nalgebra", "base", "SVector"], kind: "struct" },
      },
      external_crates: {
        "20": { name: "nalgebra", html_root_url: null },
      },
    });
    const crates = new Map([["mycrate", crate]]);
    const resolver = new LinkResolver(crates, "/base");

    const url = resolver.resolveId(500, crate);
    expect(url).toBe("https://docs.rs/nalgebra/latest/nalgebra/base/struct.SVector.html");
    // Should NOT be .../nalgebra/nalgebra/base/...
  });
});

// ---------------------------------------------------------------------------
// LinkResolver.resolvePath
// ---------------------------------------------------------------------------

describe("LinkResolver.resolvePath", () => {
  it("resolves exact name match", () => {
    const crates = new Map<string, Crate>();
    const resolver = new LinkResolver(crates, "/base");
    resolver.registerPage(1, "/base/crate/api/structs/nrlmsise00/", "Nrlmsise00", "crate");
    resolver.registerPage(2, "/base/crate/api/structs/nrlmsise00output/", "Nrlmsise00Output", "crate");

    // Should match Nrlmsise00 exactly, NOT Nrlmsise00Output
    expect(resolver.resolvePath("Nrlmsise00")).toBe("/base/crate/api/structs/nrlmsise00/");
  });

  it("resolves qualified path by last segment", () => {
    const crates = new Map<string, Crate>();
    const resolver = new LinkResolver(crates, "/base");
    resolver.registerPage(1, "/base/crate/api/traits/integrator/", "Integrator", "crate");

    // "module::Type" → resolves "Type"
    expect(resolver.resolvePath("crate::Integrator")).toBe("/base/crate/api/traits/integrator/");
    // "Type::method" → extracts "method" which is not a registered page
    expect(resolver.resolvePath("Integrator::step")).toBeUndefined();
    // Direct name works
    expect(resolver.resolvePath("Integrator")).toBe("/base/crate/api/traits/integrator/");
  });

  it("returns undefined for unknown paths", () => {
    const crates = new Map<string, Crate>();
    const resolver = new LinkResolver(crates, "/base");

    expect(resolver.resolvePath("NonExistent")).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// collectTraitImpls — auto-trait filtering
// ---------------------------------------------------------------------------

describe("collectTraitImpls", () => {
  it("separates auto-traits from user traits", () => {
    const crate = makeCrate({
      index: {
        "10": makeItem(10, null as unknown as string, {
          impl: {
            is_unsafe: false,
            generics: { params: [], where_predicates: [] },
            provided_trait_methods: [],
            trait: { path: "Send", id: 100, args: null },
            for: { generic: "Self" },
            items: [],
            is_negative: false,
            blanket_impl: null,
          },
        }),
        "11": makeItem(11, null as unknown as string, {
          impl: {
            is_unsafe: false,
            generics: { params: [], where_predicates: [] },
            provided_trait_methods: [],
            trait: { path: "Debug", id: 101, args: null },
            for: { generic: "Self" },
            items: [],
            is_negative: false,
            blanket_impl: null,
          },
        }),
        "12": makeItem(12, null as unknown as string, {
          impl: {
            is_unsafe: false,
            generics: { params: [], where_predicates: [] },
            provided_trait_methods: [],
            trait: { path: "Clone", id: 102, args: null },
            for: { generic: "Self" },
            items: [],
            is_negative: false,
            blanket_impl: null,
          },
        }),
        "13": makeItem(13, null as unknown as string, {
          impl: {
            is_unsafe: false,
            generics: { params: [], where_predicates: [] },
            provided_trait_methods: [],
            trait: { path: "Freeze", id: 103, args: null },
            for: { generic: "Self" },
            items: [],
            is_negative: false,
            blanket_impl: null,
          },
        }),
      },
    });

    const { userTraits, autoTraits } = collectTraitImpls(crate, [10, 11, 12, 13]);

    expect(userTraits.map((t) => t.traitName)).toEqual(["Debug", "Clone"]);
    expect(autoTraits.map((t) => t.traitName)).toEqual(["Send", "Freeze"]);
  });

  it("skips blanket impls", () => {
    const crate = makeCrate({
      index: {
        "10": makeItem(10, null as unknown as string, {
          impl: {
            is_unsafe: false,
            generics: { params: [], where_predicates: [] },
            provided_trait_methods: [],
            trait: { path: "Into", id: 100, args: null },
            for: { generic: "T" },
            items: [],
            is_negative: false,
            blanket_impl: { generic: "T" },
          },
        }),
      },
    });

    const { userTraits, autoTraits } = collectTraitImpls(crate, [10]);
    expect(userTraits).toHaveLength(0);
    expect(autoTraits).toHaveLength(0);
  });
});

// ---------------------------------------------------------------------------
// resolveTraitImplUrl
// ---------------------------------------------------------------------------

describe("resolveTraitImplUrl", () => {
  it("resolves std traits via paths table", () => {
    const crate = makeCrate({
      paths: {
        "100": { crate_id: 2, path: ["core", "fmt", "Debug"], kind: "trait" },
      },
      external_crates: {
        "2": { name: "core", html_root_url: null },
      },
    });
    const crates = new Map([["mycrate", crate]]);
    const resolver = new LinkResolver(crates, "/base");

    const url = resolveTraitImplUrl(
      { traitName: "Debug", traitId: 100, fullPath: ["core", "fmt", "Debug"], crateId: 2 },
      crate,
      resolver,
    );
    expect(url).toContain("doc.rust-lang.org");
    expect(url).toContain("trait.Debug.html");
  });

  it("resolves local traits to internal pages", () => {
    const crate = makeCrate();
    const crates = new Map([["mycrate", crate]]);
    const resolver = new LinkResolver(crates, "/base");
    resolver.registerPage(50, "/base/mycrate/api/traits/odestate/", "OdeState", "mycrate");

    const url = resolveTraitImplUrl(
      { traitName: "OdeState", traitId: 50, fullPath: ["utsuroi", "state", "OdeState"], crateId: 0 },
      crate,
      resolver,
    );
    expect(url).toBe("/base/mycrate/api/traits/odestate/");
  });
});
