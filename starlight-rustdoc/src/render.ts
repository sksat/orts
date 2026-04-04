/**
 * Renders rustdoc JSON types, generics, and function signatures as
 * human-readable Rust-like strings.
 */

import type {
  Crate,
  FunctionItem,
  FunctionSignature,
  FunctionHeader,
  GenericArgs,
  GenericArg,
  GenericBound,
  GenericParam,
  Generics,
  QualifiedPath,
  TraitBound,
  Type,
  TypePath,
  WherePredicate,
  TypeBindingConstraint,
} from "./types.js";
import type { LinkResolver } from "./resolve.js";

// ---------------------------------------------------------------------------
// Type rendering
// ---------------------------------------------------------------------------

export function renderType(type: Type, crate: Crate, resolver: LinkResolver, plain = false): string {
  if ("primitive" in type) {
    return type.primitive;
  }

  if ("generic" in type) {
    return type.generic;
  }

  if ("resolved_path" in type) {
    return renderTypePath(type.resolved_path, crate, resolver, plain);
  }

  if ("qualified_path" in type) {
    return renderQualifiedPath(type.qualified_path, crate, resolver, plain);
  }

  if ("borrowed_ref" in type) {
    const ref = type.borrowed_ref;
    const lt = ref.lifetime ? `${ref.lifetime} ` : "";
    const mut = ref.is_mutable ? "mut " : "";
    const inner = renderType(ref.type, crate, resolver, plain);
    return `&${lt}${mut}${inner}`;
  }

  if ("raw_pointer" in type) {
    const ptr = type.raw_pointer;
    const kind = ptr.is_mutable ? "mut" : "const";
    const inner = renderType(ptr.type, crate, resolver, plain);
    return `*${kind} ${inner}`;
  }

  if ("tuple" in type) {
    if (type.tuple.length === 0) return "()";
    const items = type.tuple.map((t) => renderType(t, crate, resolver, plain));
    return `(${items.join(", ")})`;
  }

  if ("slice" in type) {
    const inner = renderType(type.slice, crate, resolver, plain);
    return `[${inner}]`;
  }

  if ("array" in type) {
    const inner = renderType(type.array.type, crate, resolver, plain);
    return `[${inner}; ${type.array.len}]`;
  }

  if ("impl_trait" in type) {
    const bounds = type.impl_trait.map((b) => renderGenericBound(b, crate, resolver));
    return `impl ${bounds.join(" + ")}`;
  }

  if ("dyn_trait" in type) {
    const traits = type.dyn_trait.traits.map((t) => renderTypePath(t.trait, crate, resolver, plain));
    const lt = type.dyn_trait.lifetime ? ` + ${type.dyn_trait.lifetime}` : "";
    return `dyn ${traits.join(" + ")}${lt}`;
  }

  if ("function_pointer" in type) {
    const fp = type.function_pointer;
    const header = renderHeader(fp.header);
    const params = fp.sig.inputs.map(([, t]) => renderType(t, crate, resolver, plain));
    const ret = fp.sig.output ? ` -> ${renderType(fp.sig.output, crate, resolver, plain)}` : "";
    return `${header}fn(${params.join(", ")})${ret}`;
  }

  if ("infer" in type) {
    return "_";
  }

  if ("pat" in type) {
    return renderType(type.pat.type, crate, resolver);
  }

  // Unknown type variant
  return "/* unknown type */";
}

function renderTypePath(tp: TypePath, crate: Crate, resolver: LinkResolver, plain = false): string {
  const url = plain ? undefined : resolver.resolveId(tp.id, crate);
  // Strip crate:: prefix from internal paths
  const name = tp.path.replace(/^crate::(?:.*::)?/, "");
  const args = tp.args ? renderGenericArgs(tp.args, crate, resolver, plain) : "";
  const rendered = `${name}${args}`;

  if (url) {
    return `[${rendered}](${url})`;
  }
  return rendered;
}

function renderQualifiedPath(qp: QualifiedPath, crate: Crate, resolver: LinkResolver, plain = false): string {
  const selfType = renderType(qp.self_type, crate, resolver, plain);
  const args = qp.args ? renderGenericArgs(qp.args, crate, resolver, plain) : "";

  if (qp.trait) {
    const traitName = qp.trait.path || resolveTraitName(qp.trait.id, crate);
    if (traitName) {
      return `<${selfType} as ${traitName}>::${qp.name}${args}`;
    }
  }

  // No trait — just an associated type
  return `${selfType}::${qp.name}${args}`;
}

function resolveTraitName(id: number, crate: Crate): string {
  const item = crate.index[String(id)];
  if (item?.name) return item.name;

  const path = crate.paths[String(id)];
  if (path) return path.path[path.path.length - 1] ?? "";

  return "";
}

// ---------------------------------------------------------------------------
// Generic args
// ---------------------------------------------------------------------------

function renderGenericArgs(args: GenericArgs, crate: Crate, resolver: LinkResolver, plain = false): string {
  if ("angle_bracketed" in args) {
    const ab = args.angle_bracketed;
    const parts: string[] = [];

    for (const arg of ab.args) {
      parts.push(renderGenericArg(arg, crate, resolver, plain));
    }

    for (const constraint of ab.constraints) {
      parts.push(renderTypeBindingConstraint(constraint, crate, resolver));
    }

    if (parts.length === 0) return "";
    if (plain) return `<${parts.join(", ")}>`;
    return `\\<${parts.join(", ")}\\>`;
  }

  if ("parenthesized" in args) {
    const p = args.parenthesized;
    const inputs = p.inputs.map((t) => renderType(t, crate, resolver, plain));
    const ret = p.output ? ` -> ${renderType(p.output, crate, resolver, plain)}` : "";
    return `(${inputs.join(", ")})${ret}`;
  }

  return "";
}

function renderGenericArg(arg: GenericArg, crate: Crate, resolver: LinkResolver, plain = false): string {
  if ("type" in arg) return renderType(arg.type, crate, resolver, plain);
  if ("lifetime" in arg) return arg.lifetime;
  if ("const" in arg) return arg.const.expr;
  if ("infer" in arg) return "_";
  return "";
}

function renderTypeBindingConstraint(
  c: TypeBindingConstraint,
  crate: Crate,
  resolver: LinkResolver,
): string {
  const args = c.args ? renderGenericArgs(c.args, crate, resolver) : "";
  if ("equality" in c.binding) {
    return `${c.name}${args} = ${renderGenericArg(c.binding.equality, crate, resolver)}`;
  }
  const bounds = c.binding.constraint.map((b) => renderGenericBound(b, crate, resolver));
  return `${c.name}${args}: ${bounds.join(" + ")}`;
}

// ---------------------------------------------------------------------------
// Generic params & bounds
// ---------------------------------------------------------------------------

export function renderGenericParams(params: GenericParam[], crate: Crate, resolver: LinkResolver): string {
  if (params.length === 0) return "";

  const parts = params
    .filter((p) => {
      // Skip synthetic params (auto-generated by the compiler)
      if ("type" in p.kind && p.kind.type.is_synthetic) return false;
      return true;
    })
    .map((p) => renderGenericParam(p, crate, resolver));

  if (parts.length === 0) return "";
  return `\\<${parts.join(", ")}\\>`;
}

function renderGenericParam(param: GenericParam, crate: Crate, resolver: LinkResolver): string {
  if ("type" in param.kind) {
    const bounds = param.kind.type.bounds.map((b) => renderGenericBound(b, crate, resolver));
    const boundsStr = bounds.length > 0 ? `: ${bounds.join(" + ")}` : "";
    const defaultStr = param.kind.type.default
      ? ` = ${renderType(param.kind.type.default, crate, resolver)}`
      : "";
    return `${param.name}${boundsStr}${defaultStr}`;
  }

  if ("lifetime" in param.kind) {
    const outlives = param.kind.lifetime.outlives;
    if (outlives.length > 0) {
      return `${param.name}: ${outlives.join(" + ")}`;
    }
    return param.name;
  }

  if ("const" in param.kind) {
    const typeStr = renderType(param.kind.const.type, crate, resolver);
    const defaultStr = param.kind.const.default != null ? ` = ${param.kind.const.default}` : "";
    return `const ${param.name}: ${typeStr}${defaultStr}`;
  }

  return param.name;
}

export function renderGenericBound(bound: GenericBound, crate: Crate, resolver: LinkResolver): string {
  if ("trait_bound" in bound) {
    return renderTraitBound(bound.trait_bound, crate, resolver);
  }
  if ("outlives" in bound) {
    return bound.outlives;
  }
  if ("use" in bound) {
    return `use<${bound.use.join(", ")}>`;
  }
  return "";
}

function renderTraitBound(tb: TraitBound, crate: Crate, resolver: LinkResolver): string {
  const prefix = tb.modifier === "maybe" ? "?" : tb.modifier === "const" ? "~const " : "";
  const hrtb =
    tb.generic_params.length > 0
      ? `for\\<${tb.generic_params.map((p) => renderGenericParam(p, crate, resolver)).join(", ")}\\> `
      : "";
  return `${prefix}${hrtb}${renderTypePath(tb.trait, crate, resolver)}`;
}

// ---------------------------------------------------------------------------
// Where predicates
// ---------------------------------------------------------------------------

export function renderWhereClause(predicates: WherePredicate[], crate: Crate, resolver: LinkResolver): string {
  if (predicates.length === 0) return "";

  const parts = predicates.map((p) => renderWherePredicate(p, crate, resolver));
  return `where ${parts.join(", ")}`;
}

function renderWherePredicate(pred: WherePredicate, crate: Crate, resolver: LinkResolver): string {
  if ("bound_predicate" in pred) {
    const bp = pred.bound_predicate;
    const ty = renderType(bp.type, crate, resolver);
    const bounds = bp.bounds.map((b) => renderGenericBound(b, crate, resolver));
    return `${ty}: ${bounds.join(" + ")}`;
  }

  if ("lifetime_predicate" in pred) {
    const lp = pred.lifetime_predicate;
    return `${lp.lifetime}: ${lp.outlives.join(" + ")}`;
  }

  if ("eq_predicate" in pred) {
    const ep = pred.eq_predicate;
    return `${renderType(ep.lhs, crate, resolver)} = ${renderType(ep.rhs, crate, resolver)}`;
  }

  return "";
}

// ---------------------------------------------------------------------------
// Function signature
// ---------------------------------------------------------------------------

function renderHeader(header: FunctionHeader): string {
  const parts: string[] = [];
  if (header.is_const) parts.push("const ");
  if (header.is_unsafe) parts.push("unsafe ");
  if (header.is_async) parts.push("async ");
  if (header.abi !== "Rust") parts.push(`extern "${header.abi}" `);
  return parts.join("");
}

export function renderFunctionSig(
  fn: FunctionItem,
  name: string,
  crate: Crate,
  resolver: LinkResolver,
): string {
  const header = renderHeader(fn.header);
  const generics = renderGenericParams(fn.generics.params, crate, resolver);
  const params = fn.sig.inputs.map(([paramName, paramType]) => {
    // Self params
    if (paramName === "self") {
      return renderSelfParam(paramType, crate, resolver);
    }
    return `${paramName}: ${renderType(paramType, crate, resolver)}`;
  });
  const ret = fn.sig.output ? ` -> ${renderType(fn.sig.output, crate, resolver)}` : "";
  const whereClause = renderWhereClause(fn.generics.where_predicates, crate, resolver);
  const whereStr = whereClause ? ` ${whereClause}` : "";

  return `${header}fn ${name}${generics}(${params.join(", ")})${ret}${whereStr}`;
}

function renderSelfParam(type: Type, crate: Crate, resolver: LinkResolver): string {
  if ("borrowed_ref" in type) {
    const ref = type.borrowed_ref;
    const lt = ref.lifetime ? `${ref.lifetime} ` : "";
    const mut = ref.is_mutable ? "mut " : "";
    return `&${lt}${mut}self`;
  }
  // Owned self
  return "self";
}
