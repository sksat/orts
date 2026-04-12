/**
 * Generates Markdown pages from collected API items.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import {
  renderFunctionSig,
  renderGenericBound,
  renderGenericParams,
  renderType,
  renderWhereClause,
} from "./render.js";
import type { ApiItem, ApiItemCategory, LinkResolver } from "./resolve.js";
import {
  collectImplementors,
  collectInherentImpls,
  collectTraitImpls,
  resolveTraitImplUrl,
} from "./resolve.js";
import type {
  ConstantItem,
  Crate,
  EnumItem,
  FunctionItem,
  Generics,
  Id,
  Item,
  StructItem,
  StructKind,
  TraitItem,
  TypeAliasItem,
  VariantItem,
  VariantKind,
} from "./types.js";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export interface MarkdownOptions {
  contentDir: string; // e.g. docs/src/content/docs
  basePath: string; // e.g. "/orts"
  /**
   * Locale prefix for i18n (e.g. "en"). When set, files are written under
   * `${contentDir}/${locale}/${crate}/api/...` and cross-crate URLs embed
   * `${locale}/` between basePath and crate segments.
   */
  locale?: string;
  sourceLinks?: {
    repository: string;
    branch: string;
  };
}

export interface GeneratedPage {
  /** Relative path within contentDir */
  relativePath: string;
  category: ApiItemCategory;
  name: string;
}

export function generateCratePages(
  crateName: string,
  items: ApiItem[],
  crate: Crate,
  resolver: LinkResolver,
  options: MarkdownOptions,
): GeneratedPage[] {
  const pages: GeneratedPage[] = [];
  const localePrefix = localePrefixOf(options);

  // Generate overview page.
  // Tell the resolver which page is being generated so that link lookups
  // return relative URLs rooted at this page. Relative links keep the
  // generated content locale-agnostic — the same file works whether served
  // from `/en/...` or from a `/ja/...` fallback route.
  resolver.setCurrentPage(`${crateName}/api/overview/`);
  const overviewPath = `${localePrefix}${crateName}/api/overview.md`;
  const overviewContent = generateOverviewPage(crateName, items, crate, resolver);
  writePage(join(options.contentDir, overviewPath), overviewContent);
  pages.push({ relativePath: overviewPath, category: "struct", name: "overview" });

  // Generate individual pages
  for (const apiItem of items) {
    const slug = apiItem.displayName.toLowerCase();
    resolver.setCurrentPage(`${crateName}/api/${categoryDir(apiItem.category)}/${slug}/`);
    const relativePath = `${localePrefix}${crateName}/api/${categoryDir(apiItem.category)}/${slug}.md`;
    const content = generateItemPage(apiItem, crate, resolver, options);
    writePage(join(options.contentDir, relativePath), content);
    pages.push({ relativePath, category: apiItem.category, name: apiItem.displayName });
  }

  return pages;
}

function localePrefixOf(options: Pick<MarkdownOptions, "locale">): string {
  return options.locale ? `${options.locale}/` : "";
}

// ---------------------------------------------------------------------------
// Overview page
// ---------------------------------------------------------------------------

function generateOverviewPage(
  crateName: string,
  items: ApiItem[],
  crate: Crate,
  resolver: LinkResolver,
): string {
  const lines: string[] = [];

  lines.push(frontmatter(crateName, { sidebarOrder: -1 }));

  // Crate-level docs
  const root = crate.index[String(crate.root)];
  if (root?.docs) {
    lines.push(processDocComment(root.docs, crate, resolver));
    lines.push("");
  }

  // Group items by category
  const grouped = groupByCategory(items);

  for (const [category, categoryItems] of grouped) {
    lines.push(`## ${categoryLabel(category)}`);
    lines.push("");
    lines.push("| Name | Description |");
    lines.push("|------|-------------|");
    for (const item of categoryItems) {
      // Look up each item through the resolver so that the resulting link is
      // a relative URL from the overview page (set as the current page by
      // generateCratePages). Falling back to a root-relative path if the
      // resolver doesn't know about the item shouldn't happen because items
      // on this list were all registered, but guard defensively.
      const link =
        resolver.resolveId(item.item.id, crate, item.crateName) ??
        `${item.crateName}/api/${categoryDir(item.category)}/${item.displayName.toLowerCase()}/`;
      const desc = firstSentence(item.item.docs);
      lines.push(`| [${item.displayName}](${link}) | ${desc} |`);
    }
    lines.push("");
  }

  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// Item pages
// ---------------------------------------------------------------------------

function generateItemPage(
  apiItem: ApiItem,
  crate: Crate,
  resolver: LinkResolver,
  options: MarkdownOptions,
): string {
  const lines: string[] = [];
  lines.push(frontmatter(apiItem.displayName));
  lines.push(sourceLink(apiItem.item, options));

  switch (apiItem.category) {
    case "trait":
      generateTraitPage(lines, apiItem, crate, resolver);
      break;
    case "struct":
      generateStructPage(lines, apiItem, crate, resolver);
      break;
    case "enum":
      generateEnumPage(lines, apiItem, crate, resolver);
      break;
    case "function":
      generateFunctionPage(lines, apiItem, crate, resolver);
      break;
    case "type_alias":
      generateTypeAliasPage(lines, apiItem, crate, resolver);
      break;
    case "constant":
      generateConstantPage(lines, apiItem, crate, resolver);
      break;
  }

  return lines.join("\n");
}

// ---------------------------------------------------------------------------
// Trait page
// ---------------------------------------------------------------------------

function generateTraitPage(
  lines: string[],
  apiItem: ApiItem,
  crate: Crate,
  resolver: LinkResolver,
): void {
  const traitData = (apiItem.item.inner as { trait: TraitItem }).trait;

  // Trait signature
  const generics = renderGenericParams(traitData.generics.params, crate, resolver);
  const bounds =
    traitData.bounds.length > 0
      ? `: ${traitData.bounds.map((b) => renderGenericBound(b, crate, resolver)).join(" + ")}`
      : "";
  const where = renderWhereClause(traitData.generics.where_predicates, crate, resolver);
  lines.push("");
  lines.push("```rust");
  lines.push(`pub trait ${apiItem.displayName}${generics}${bounds} ${where}`);
  lines.push("```");
  lines.push("");

  // Docs
  if (apiItem.item.docs) {
    lines.push(processDocComment(apiItem.item.docs, crate, resolver));
    lines.push("");
  }

  // Separate required vs provided methods
  const requiredMethods: Item[] = [];
  const providedMethods: Item[] = [];

  for (const methodId of traitData.items) {
    const method = crate.index[String(methodId)];
    if (!method) continue;
    const fn = (method.inner as { function?: FunctionItem }).function;
    if (!fn) continue;

    if (fn.has_body) {
      providedMethods.push(method);
    } else {
      requiredMethods.push(method);
    }
  }

  if (requiredMethods.length > 0) {
    lines.push("## Required Methods");
    lines.push("");
    for (const method of requiredMethods) {
      renderMethodSection(lines, method, crate, resolver);
    }
  }

  if (providedMethods.length > 0) {
    lines.push("## Provided Methods");
    lines.push("");
    for (const method of providedMethods) {
      renderMethodSection(lines, method, crate, resolver);
    }
  }

  // Implementors
  const implementors = collectImplementors(crate, traitData.implementations);
  if (implementors.length > 0) {
    lines.push("## Implementors");
    lines.push("");
    for (const impl of implementors) {
      const url = resolver.resolveId(impl.id, crate);
      if (url) {
        lines.push(`- [${impl.name}](${url})`);
      } else {
        lines.push(`- ${impl.name}`);
      }
    }
    lines.push("");
  }
}

// ---------------------------------------------------------------------------
// Struct page
// ---------------------------------------------------------------------------

function generateStructPage(
  lines: string[],
  apiItem: ApiItem,
  crate: Crate,
  resolver: LinkResolver,
): void {
  const structData = (apiItem.item.inner as { struct: StructItem }).struct;

  const generics = renderGenericParams(structData.generics.params, crate, resolver);
  const where = renderWhereClause(structData.generics.where_predicates, crate, resolver);

  // Determine struct kind and collect fields
  const kindObj = structData.kind as
    | { plain: { fields: Id[]; has_stripped_fields: boolean } }
    | { tuple: (Id | null)[] }
    | "unit";

  const isPlain = typeof kindObj === "object" && "plain" in kindObj;
  const isTuple = typeof kindObj === "object" && "tuple" in kindObj;

  const plainFields = isPlain
    ? kindObj.plain.fields
        .map((fid) => crate.index[String(fid)])
        .filter((f): f is Item => f != null && f.visibility === "public")
    : [];

  const tupleFields = isTuple
    ? (kindObj.tuple as (Id | null)[])
        .map((fid) => (fid != null ? crate.index[String(fid)] : null))
        .filter((f): f is Item => f != null)
    : [];

  // Struct signature with fields
  lines.push("");
  lines.push("```rust");
  if (isTuple && tupleFields.length > 0) {
    // Tuple struct: pub struct Eci(pub Vector3<f64>);
    const fieldTypes = tupleFields.map((f) => {
      const ft = (f.inner as { struct_field?: unknown }).struct_field;
      const vis = f.visibility === "public" ? "pub " : "";
      const typeStr = ft ? renderType(ft as import("./types.js").Type, crate, resolver, true) : "?";
      return `${vis}${typeStr}`;
    });
    const whereStr = where ? ` ${where}` : "";
    lines.push(
      `pub struct ${apiItem.displayName}${generics}(${fieldTypes.join(", ")})${whereStr};`,
    );
  } else if (isPlain && plainFields.length > 0) {
    // Named fields: pub struct Foo { pub x: f64, ... }
    const whereStr = where ? `\n${where}` : "";
    lines.push(`pub struct ${apiItem.displayName}${generics}${whereStr} {`);
    for (const field of plainFields) {
      const fieldType = (field.inner as { struct_field?: unknown }).struct_field;
      const typeStr = fieldType
        ? renderType(fieldType as import("./types.js").Type, crate, resolver, true)
        : "?";
      lines.push(`    pub ${field.name}: ${typeStr},`);
    }
    if (isPlain && kindObj.plain.has_stripped_fields) {
      lines.push("    // ...");
    }
    lines.push("}");
  } else {
    lines.push(`pub struct ${apiItem.displayName}${generics} ${where}`);
  }
  lines.push("```");
  lines.push("");

  // Docs
  if (apiItem.item.docs) {
    lines.push(processDocComment(apiItem.item.docs, crate, resolver));
    lines.push("");
  }

  // Fields detail section (named fields only)
  if (plainFields.length > 0) {
    lines.push("## Fields");
    lines.push("");
    for (const field of plainFields) {
      const fieldType = (field.inner as { struct_field?: unknown }).struct_field;
      const typeStr = fieldType
        ? renderType(fieldType as import("./types.js").Type, crate, resolver)
        : "";
      lines.push(`### ${field.name}`);
      lines.push("");
      lines.push(`> **${field.name}**: ${typeStr}`);
      lines.push("");
      if (field.docs) {
        lines.push(processDocComment(field.docs, crate, resolver));
        lines.push("");
      }
      lines.push("***");
      lines.push("");
    }
  }

  // Methods (from inherent impls)
  const impls = collectInherentImpls(crate, structData.impls);
  const allMethods = impls.flatMap((i) => i.methods).filter((m) => m.visibility === "public");

  if (allMethods.length > 0) {
    lines.push("## Methods");
    lines.push("");
    for (const method of allMethods) {
      renderMethodSection(lines, method, crate, resolver);
    }
  }

  // Trait implementations
  const { userTraits, autoTraits } = collectTraitImpls(crate, structData.impls);
  if (userTraits.length > 0) {
    lines.push("## Trait Implementations");
    lines.push("");
    for (const ti of userTraits) {
      renderTraitImplLink(lines, ti, crate, resolver);
    }
    lines.push("");
  }
}

// ---------------------------------------------------------------------------
// Enum page
// ---------------------------------------------------------------------------

function generateEnumPage(
  lines: string[],
  apiItem: ApiItem,
  crate: Crate,
  resolver: LinkResolver,
): void {
  const enumData = (apiItem.item.inner as { enum: EnumItem }).enum;

  const generics = renderGenericParams(enumData.generics.params, crate, resolver);
  const where = renderWhereClause(enumData.generics.where_predicates, crate, resolver);

  // Collect variants
  const variants = enumData.variants
    .map((vid) => crate.index[String(vid)])
    .filter((v): v is Item => v != null);

  // Enum signature with variants in code block
  lines.push("");
  lines.push("```rust");
  const whereStr = where ? ` ${where}` : "";
  if (variants.length > 0) {
    lines.push(`pub enum ${apiItem.displayName}${generics}${whereStr} {`);
    for (const variant of variants) {
      const variantData = (variant.inner as { variant?: VariantItem }).variant;
      const variantSig = renderVariantSignature(variant.name!, variantData, crate, resolver);
      lines.push(`    ${variantSig},`);
    }
    lines.push("}");
  } else {
    lines.push(`pub enum ${apiItem.displayName}${generics}${whereStr}`);
  }
  lines.push("```");
  lines.push("");

  // Docs
  if (apiItem.item.docs) {
    lines.push(processDocComment(apiItem.item.docs, crate, resolver));
    lines.push("");
  }

  // Variants detail section
  if (variants.length > 0) {
    lines.push("## Variants");
    lines.push("");
    for (const variant of variants) {
      lines.push(`### ${variant.name}`);
      lines.push("");
      const variantData = (variant.inner as { variant?: VariantItem }).variant;
      if (variantData) {
        const kind = variantData.kind;
        if (typeof kind === "object" && "tuple" in kind) {
          // Tuple variant: Completed(Y)
          const fields = (kind.tuple as (Id | null)[])
            .map((fid) => (fid != null ? crate.index[String(fid)] : null))
            .filter((f): f is Item => f != null);
          if (fields.length > 0) {
            const types = fields.map((f) => {
              const ft = (f.inner as { struct_field?: unknown }).struct_field;
              return ft ? renderType(ft as import("./types.js").Type, crate, resolver) : "_";
            });
            lines.push(`> ${variant.name}(${types.join(", ")})`);
            lines.push("");
          }
        } else if (typeof kind === "object" && "struct" in kind) {
          // Struct variant: Terminated { state: Y, t: f64, reason: B }
          const structFields = kind.struct.fields
            .map((fid: Id) => crate.index[String(fid)])
            .filter((f): f is Item => f != null);
          if (structFields.length > 0) {
            for (const field of structFields) {
              const ft = (field.inner as { struct_field?: unknown }).struct_field;
              const typeStr = ft
                ? renderType(ft as import("./types.js").Type, crate, resolver)
                : "?";
              lines.push(`> **${field.name}**: ${typeStr}`);
            }
            lines.push("");
          }
        }
      }
      if (variant.docs) {
        lines.push(processDocComment(variant.docs, crate, resolver));
        lines.push("");
      }
      lines.push("***");
      lines.push("");
    }
  }

  // Methods
  const impls = collectInherentImpls(crate, enumData.impls);
  const allMethods = impls.flatMap((i) => i.methods).filter((m) => m.visibility === "public");

  if (allMethods.length > 0) {
    lines.push("## Methods");
    lines.push("");
    for (const method of allMethods) {
      renderMethodSection(lines, method, crate, resolver);
    }
  }
}

// ---------------------------------------------------------------------------
// Function page
// ---------------------------------------------------------------------------

function generateFunctionPage(
  lines: string[],
  apiItem: ApiItem,
  crate: Crate,
  resolver: LinkResolver,
): void {
  const fnData = (apiItem.item.inner as { function: FunctionItem }).function;

  const sig = renderFunctionSig(fnData, apiItem.displayName, crate, resolver);
  lines.push("");
  lines.push(`> **${sig}**`);
  lines.push("");

  if (apiItem.item.docs) {
    lines.push(processDocComment(apiItem.item.docs, crate, resolver));
    lines.push("");
  }
}

// ---------------------------------------------------------------------------
// Type alias page
// ---------------------------------------------------------------------------

function generateTypeAliasPage(
  lines: string[],
  apiItem: ApiItem,
  crate: Crate,
  resolver: LinkResolver,
): void {
  const taData = (apiItem.item.inner as { type_alias: TypeAliasItem }).type_alias;

  const generics = renderGenericParams(taData.generics.params, crate, resolver);
  const underlying = renderType(taData.type, crate, resolver);
  lines.push("");
  lines.push(`> **type ${apiItem.displayName}${generics}** = ${underlying}`);
  lines.push("");

  if (apiItem.item.docs) {
    lines.push(processDocComment(apiItem.item.docs, crate, resolver));
    lines.push("");
  }
}

// ---------------------------------------------------------------------------
// Constant page
// ---------------------------------------------------------------------------

function generateConstantPage(
  lines: string[],
  apiItem: ApiItem,
  crate: Crate,
  resolver: LinkResolver,
): void {
  const constData = (apiItem.item.inner as { constant?: ConstantItem; static?: unknown }).constant;

  if (constData) {
    const typeStr = renderType(constData.type, crate, resolver);
    const value = constData.const.value ?? constData.const.expr;
    lines.push("");
    lines.push(`> **const ${apiItem.displayName}**: ${typeStr} = ${value}`);
    lines.push("");
  }

  if (apiItem.item.docs) {
    lines.push(processDocComment(apiItem.item.docs, crate, resolver));
    lines.push("");
  }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

function renderMethodSection(
  lines: string[],
  method: Item,
  crate: Crate,
  resolver: LinkResolver,
): void {
  const fn = (method.inner as { function?: FunctionItem }).function;
  if (!fn) return;

  lines.push(`### ${method.name}()`);
  lines.push("");
  const sig = renderFunctionSig(fn, method.name!, crate, resolver);
  lines.push(`> **${sig}**`);
  lines.push("");

  if (method.docs) {
    lines.push(processDocComment(method.docs, crate, resolver));
    lines.push("");
  }

  lines.push("***");
  lines.push("");
}

function renderTraitImplLink(
  lines: string[],
  ti: import("./resolve.js").TraitImplInfo,
  crate: Crate,
  resolver: LinkResolver,
): void {
  const url = resolveTraitImplUrl(ti, crate, resolver);
  if (url) {
    lines.push(`- [${ti.traitName}](${url})`);
  } else {
    lines.push(`- ${ti.traitName}`);
  }
}

function renderVariantSignature(
  name: string,
  variantData: VariantItem | undefined,
  crate: Crate,
  resolver: LinkResolver,
): string {
  if (!variantData) return name;
  const kind = variantData.kind;

  if (typeof kind === "string") {
    // Unit variant: "plain"
    return name;
  }

  if ("tuple" in kind) {
    const fields = (kind.tuple as (Id | null)[])
      .map((fid) => (fid != null ? crate.index[String(fid)] : null))
      .filter((f): f is Item => f != null);
    const types = fields.map((f) => {
      const ft = (f.inner as { struct_field?: unknown }).struct_field;
      return ft ? renderType(ft as import("./types.js").Type, crate, resolver, true) : "_";
    });
    return `${name}(${types.join(", ")})`;
  }

  if ("struct" in kind) {
    const fields = kind.struct.fields
      .map((fid: Id) => crate.index[String(fid)])
      .filter((f): f is Item => f != null);
    const fieldStrs = fields.map((f) => {
      const ft = (f.inner as { struct_field?: unknown }).struct_field;
      const typeStr = ft ? renderType(ft as import("./types.js").Type, crate, resolver, true) : "?";
      return `${f.name}: ${typeStr}`;
    });
    return `${name} { ${fieldStrs.join(", ")} }`;
  }

  return name;
}

// ---------------------------------------------------------------------------
// Frontmatter & formatting
// ---------------------------------------------------------------------------

function frontmatter(title: string, options?: { sidebarOrder?: number }): string {
  const lines = ["---", "editUrl: false", "next: false", "prev: false", `title: "${title}"`];
  if (options?.sidebarOrder !== undefined) {
    lines.push("sidebar:");
    lines.push(`  order: ${options.sidebarOrder}`);
  }
  lines.push("---", "");
  return lines.join("\n");
}

function sourceLink(item: Item, options: MarkdownOptions): string {
  if (!item.span || !options.sourceLinks) return "";
  const { filename, begin } = item.span;
  const line = begin[0];
  const url = `${options.sourceLinks.repository}/blob/${options.sourceLinks.branch}/${filename}#L${line}`;
  const shortFile = filename.split("/").slice(-1)[0];
  return `Defined in: [${shortFile}:${line}](${url})\n`;
}

function processDocComment(docs: string, crate: Crate, resolver: LinkResolver): string {
  // Convert rustdoc intralinks like [`Integrator::step`] to Markdown links
  return docs.replace(/\[`([^`]+)`\]/g, (match, path: string) => {
    const url = resolver.resolvePath(path);
    if (url) return `[\`${path}\`](${url})`;
    return `\`${path}\``;
  });
}

export function firstSentence(docs: string | null): string {
  if (!docs) return "";
  const trimmed = docs.trim();
  // Take the first sentence: period followed by whitespace or end, or newline.
  // Avoid splitting on periods in numbers like "F10.7" or "0.25".
  const match = trimmed.match(/^(.+?\.)\s/);
  if (match) return escapeTableCell(match[1]!.trim());
  // Fall back to first line
  const newlineIdx = trimmed.indexOf("\n");
  const line = newlineIdx >= 0 ? trimmed.slice(0, newlineIdx) : trimmed.slice(0, 120);
  return escapeTableCell(line.trim());
}

function escapeTableCell(text: string): string {
  return text.replace(/\|/g, "\\|");
}

function categoryDir(category: ApiItemCategory): string {
  switch (category) {
    case "type_alias":
      return "type-aliases";
    default:
      return `${category}s`;
  }
}

function categoryLabel(category: ApiItemCategory): string {
  switch (category) {
    case "trait":
      return "Traits";
    case "struct":
      return "Structs";
    case "enum":
      return "Enums";
    case "function":
      return "Functions";
    case "type_alias":
      return "Type Aliases";
    case "constant":
      return "Constants";
  }
}

function groupByCategory(items: ApiItem[]): [ApiItemCategory, ApiItem[]][] {
  const order: ApiItemCategory[] = [
    "trait",
    "struct",
    "enum",
    "function",
    "type_alias",
    "constant",
  ];
  const map = new Map<ApiItemCategory, ApiItem[]>();
  for (const item of items) {
    const list = map.get(item.category) ?? [];
    list.push(item);
    map.set(item.category, list);
  }
  return order.filter((c) => map.has(c)).map((c) => [c, map.get(c)!]);
}

function writePage(fullPath: string, content: string): void {
  mkdirSync(dirname(fullPath), { recursive: true });
  writeFileSync(fullPath, content, "utf-8");
}
