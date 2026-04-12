import { describe, expect, it } from "vitest";
import {
  renderGenericParams,
  renderType,
  renderWhereClause,
} from "../src/render.js";
import { LinkResolver } from "../src/resolve.js";
import type { Crate, Type } from "../src/types.js";

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

function makeResolver(): LinkResolver {
  return new LinkResolver(new Map(), "/base");
}

// ---------------------------------------------------------------------------
// renderType
// ---------------------------------------------------------------------------

describe("renderType", () => {
  const crate = makeCrate();
  const resolver = makeResolver();

  it("renders primitives", () => {
    expect(renderType({ primitive: "f64" }, crate, resolver)).toBe("f64");
    expect(renderType({ primitive: "bool" }, crate, resolver)).toBe("bool");
    expect(renderType({ primitive: "usize" }, crate, resolver)).toBe("usize");
  });

  it("renders generics", () => {
    expect(renderType({ generic: "T" }, crate, resolver)).toBe("T");
    expect(renderType({ generic: "Self" }, crate, resolver)).toBe("Self");
  });

  it("renders borrowed references", () => {
    expect(
      renderType(
        { borrowed_ref: { lifetime: null, is_mutable: false, type: { generic: "T" } } },
        crate,
        resolver,
      ),
    ).toBe("&T");

    expect(
      renderType(
        { borrowed_ref: { lifetime: null, is_mutable: true, type: { primitive: "str" } } },
        crate,
        resolver,
      ),
    ).toBe("&mut str");

    expect(
      renderType(
        { borrowed_ref: { lifetime: "'a", is_mutable: false, type: { generic: "T" } } },
        crate,
        resolver,
      ),
    ).toBe("&'a T");
  });

  it("renders tuples", () => {
    expect(renderType({ tuple: [] }, crate, resolver)).toBe("()");
    expect(
      renderType({ tuple: [{ primitive: "f64" }, { primitive: "f64" }] }, crate, resolver),
    ).toBe("(f64, f64)");
  });

  it("renders slices and arrays", () => {
    expect(renderType({ slice: { primitive: "u8" } }, crate, resolver)).toBe("[u8]");
    expect(renderType({ array: { type: { primitive: "f64" }, len: "7" } }, crate, resolver)).toBe(
      "[f64; 7]",
    );
  });

  it("renders raw pointers", () => {
    expect(
      renderType(
        { raw_pointer: { is_mutable: false, type: { primitive: "u8" } } },
        crate,
        resolver,
      ),
    ).toBe("*const u8");
    expect(
      renderType({ raw_pointer: { is_mutable: true, type: { primitive: "u8" } } }, crate, resolver),
    ).toBe("*mut u8");
  });

  it("strips crate:: prefix from resolved paths", () => {
    const type: Type = {
      resolved_path: { path: "crate::OrbitalState", id: 1, args: null },
    };
    expect(renderType(type, crate, resolver)).toBe("OrbitalState");
  });

  it("strips nested crate:: prefix from resolved paths", () => {
    const type: Type = {
      resolved_path: { path: "crate::record::recording::Recording", id: 1, args: null },
    };
    expect(renderType(type, crate, resolver)).toBe("Recording");
  });

  it("renders qualified paths with trait", () => {
    // <S as DynamicalSystem>::State
    const crateWithTrait = makeCrate({
      index: {
        "143": {
          id: 143,
          name: "DynamicalSystem",
          visibility: "public",
          docs: null,
          attrs: [],
          deprecation: null,
          inner: { trait: {} },
          span: null,
        },
      },
    });
    const type: Type = {
      qualified_path: {
        name: "State",
        args: null,
        self_type: { generic: "S" },
        trait: { path: "", id: 143, args: null },
      },
    };
    const rendered = renderType(type, crateWithTrait, resolver);
    expect(rendered).toContain("<S as DynamicalSystem>::State");
  });

  it("renders in plain mode without Markdown links or escaped angle brackets", () => {
    const type: Type = {
      resolved_path: {
        path: "Vec",
        id: 1,
        args: { angle_bracketed: { args: [{ type: { primitive: "f64" } }], constraints: [] } },
      },
    };
    const plain = renderType(type, crate, resolver, true);
    expect(plain).toBe("Vec<f64>");
    expect(plain).not.toContain("\\<");
    expect(plain).not.toContain("[");
  });
});

// ---------------------------------------------------------------------------
// renderGenericParams — const generics
// ---------------------------------------------------------------------------

describe("renderGenericParams", () => {
  const crate = makeCrate();
  const resolver = makeResolver();

  it("renders const generics", () => {
    const params = [
      { name: "DIM", kind: { const: { type: { primitive: "usize" as const }, default: null } } },
      { name: "ORDER", kind: { const: { type: { primitive: "usize" as const }, default: null } } },
    ];
    const result = renderGenericParams(params, crate, resolver);
    expect(result).toContain("const DIM: usize");
    expect(result).toContain("const ORDER: usize");
  });

  it("renders type params with bounds", () => {
    const params = [
      {
        name: "S",
        kind: {
          type: {
            bounds: [
              {
                trait_bound: {
                  trait: { path: "Debug", id: 1, args: null },
                  generic_params: [],
                  modifier: "none" as const,
                },
              },
            ],
            default: null,
            is_synthetic: false,
          },
        },
      },
    ];
    const result = renderGenericParams(params, crate, resolver);
    expect(result).toContain("S: Debug");
  });

  it("skips synthetic params", () => {
    const params = [
      {
        name: "impl Trait",
        kind: {
          type: {
            bounds: [],
            default: null,
            is_synthetic: true,
          },
        },
      },
    ];
    const result = renderGenericParams(params, crate, resolver);
    expect(result).toBe("");
  });
});

// ---------------------------------------------------------------------------
// renderWhereClause — type binding equality (State = State<DIM, 2>)
// ---------------------------------------------------------------------------

describe("renderWhereClause", () => {
  const crate = makeCrate();
  const resolver = makeResolver();

  it("renders where clause with type binding equality containing const args", () => {
    // where S: DynamicalSystem<State = State<DIM, 2>>
    const predicates = [
      {
        bound_predicate: {
          type: { generic: "S" } as Type,
          bounds: [
            {
              trait_bound: {
                trait: {
                  path: "DynamicalSystem",
                  id: 143,
                  args: {
                    angle_bracketed: {
                      args: [],
                      constraints: [
                        {
                          name: "State",
                          args: null,
                          binding: {
                            equality: {
                              type: {
                                resolved_path: {
                                  path: "State",
                                  id: 323,
                                  args: {
                                    angle_bracketed: {
                                      args: [
                                        { const: { expr: "DIM", value: null, is_literal: false } },
                                        { const: { expr: "2", value: null, is_literal: true } },
                                      ],
                                      constraints: [],
                                    },
                                  },
                                },
                              },
                            },
                          },
                        },
                      ],
                    },
                  },
                },
                generic_params: [],
                modifier: "none" as const,
              },
            },
          ],
          generic_params: [],
        },
      },
    ];
    const result = renderWhereClause(predicates, crate, resolver);
    expect(result).toContain("State = State");
    expect(result).toContain("DIM");
    expect(result).toContain("2");
    expect(result).not.toContain("unknown type");
  });
});
