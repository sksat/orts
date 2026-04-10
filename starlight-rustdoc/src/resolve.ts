import type {
  Crate,
  Id,
  Item,
  ItemInner,
  ImplItem,
  ModuleItem,
  UseItem,
  StructItem,
  EnumItem,
  TraitItem,
} from "./types.js";

// ---------------------------------------------------------------------------
// Collected item — a public API item that will get its own page
// ---------------------------------------------------------------------------

export type ApiItemCategory = "trait" | "struct" | "enum" | "function" | "type_alias" | "constant";

export interface ApiItem {
  /** The resolved Item from the index (follows re-exports) */
  item: Item;
  /** Display name (may differ from item.name due to re-export rename) */
  displayName: string;
  /** Which category this item belongs to */
  category: ApiItemCategory;
  /** The crate this item is exposed from */
  crateName: string;
}

// ---------------------------------------------------------------------------
// LinkResolver — maps item IDs to page paths and external URLs
// ---------------------------------------------------------------------------

export class LinkResolver {
  /** "crateName:id" → stored page path — keyed by crate to avoid ID collisions */
  private pageMap = new Map<string, string>();
  /** display name (lowercase) → page path — for intra-doc link resolution */
  private nameToPage = new Map<string, string>();
  /** All loaded crate JSONs for cross-crate resolution */
  private crates: Map<string, Crate>;

  private basePath: string;

  /** Crate object → crate name reverse lookup */
  private crateNames = new Map<Crate, string>();

  /**
   * Logical path of the page currently being generated, e.g.
   * `"kaname/api/structs/epoch/"`. Used to compute relative URLs from stored
   * logical paths so that generated links navigate correctly regardless of
   * which locale the page is served under (including Starlight i18n fallback
   * routes).
   */
  private currentPagePath = "";

  constructor(crates: Map<string, Crate>, basePath: string) {
    this.crates = crates;
    this.basePath = basePath;
    for (const [name, crate] of crates) {
      this.crateNames.set(crate, name);
    }
  }

  /** Get the crate name for a Crate object */
  getCrateName(crate: Crate): string {
    return this.crateNames.get(crate) ?? "unknown";
  }

  /**
   * Set the logical path of the page currently being generated. Callers
   * should invoke this before generating markdown for each page so that
   * `resolveId`/`resolvePath` can return relative URLs rooted at the current
   * page's location.
   */
  setCurrentPage(logicalPath: string): void {
    this.currentPagePath = logicalPath;
  }

  /**
   * Register a page path for an item ID, scoped to its crate.
   *
   * Prefer passing a **logical path** (e.g. `"kaname/api/structs/epoch/"`)
   * without a leading slash, basePath, or locale segment. Such paths are
   * resolved to relative URLs at lookup time using the current page. Paths
   * that already look absolute (starting with `/` or containing `://`) are
   * stored verbatim and returned as-is (backward-compatible with callers that
   * pre-compute absolute URLs).
   */
  registerPage(id: Id, pagePath: string, displayName: string, crateName: string): void {
    this.pageMap.set(`${crateName}:${id}`, pagePath);
    this.nameToPage.set(displayName.toLowerCase(), pagePath);
  }

  /** Resolve an item ID to a URL. fromCrateName scopes the ID lookup. */
  resolveId(id: Id, fromCrate: Crate, fromCrateName?: string): string | undefined {
    const crateName = fromCrateName ?? this.getCrateName(fromCrate);
    // 1. Check our generated pages for this crate's ID space
    const page = this.pageMap.get(`${crateName}:${id}`);
    if (page) return this.toUrl(page);

    // 2. Try to resolve via the fromCrate's paths table
    const summary = fromCrate.paths[String(id)];
    if (!summary) return undefined;

    // crate_id 0 = from this crate itself (but didn't get a page)
    if (summary.crate_id === 0) return undefined;

    // Look up the external crate using THIS crate's external_crates table
    const ext = fromCrate.external_crates[String(summary.crate_id)];
    if (!ext) return undefined;

    // If the external crate is one of our documented crates, look up by name
    if (this.crates.has(ext.name)) {
      const itemName = summary.path[summary.path.length - 1];
      if (itemName) {
        const localPage = this.nameToPage.get(itemName.toLowerCase());
        if (localPage) return this.toUrl(localPage);
      }
    }

    // Build external URL
    return this.buildExternalUrl(ext.name, ext.html_root_url, summary.path, summary.kind);
  }

  /** Resolve a path string (from rustdoc intralinks) to a URL */
  resolvePath(path: string): string | undefined {
    // Extract the last segment (e.g. "Integrator::step" → "step", "Nrlmsise00" → "nrlmsise00")
    const segments = path.split("::");
    const lastName = segments[segments.length - 1]!.toLowerCase();

    // Exact match on display name
    const exact = this.nameToPage.get(lastName);
    if (exact) return this.toUrl(exact);

    return undefined;
  }

  /**
   * Convert a stored page path to a URL suitable for embedding in generated
   * Markdown. External URLs (containing `://`) and pre-formatted absolute
   * paths (starting with `/`) are returned verbatim; logical paths are
   * resolved to a relative URL from `currentPagePath`.
   */
  private toUrl(stored: string): string {
    if (stored.includes("://") || stored.startsWith("/")) {
      return stored;
    }
    return computeRelativeUrl(this.currentPagePath, stored);
  }

  private buildExternalUrl(
    crateName: string,
    htmlRootUrl: string | null,
    path: string[],
    kind: string,
  ): string {
    const itemName = path[path.length - 1];

    // std/core/alloc → doc.rust-lang.org
    if (["std", "core", "alloc"].includes(crateName)) {
      // path includes crate name as first element, skip it for std
      const modulePath = path.slice(1, -1).join("/");
      const modPart = modulePath ? `${modulePath}/` : "";
      return `https://doc.rust-lang.org/std/${modPart}${kind}.${itemName}.html`;
    }

    // For external crates, use docs.rs with "latest"
    // path[0] is the crate name, skip it to avoid duplication
    const innerPath = path.slice(1, -1).join("/");
    const modPart = innerPath ? `${innerPath}/` : "";
    return `https://docs.rs/${crateName}/latest/${crateName}/${modPart}${kind}.${itemName}.html`;
  }
}

// ---------------------------------------------------------------------------
// collectApiItems — collect all public items from a crate
// ---------------------------------------------------------------------------

const INNER_KIND_TO_CATEGORY: Record<string, ApiItemCategory | undefined> = {
  trait: "trait",
  struct: "struct",
  enum: "enum",
  function: "function",
  type_alias: "type_alias",
  constant: "constant",
  static: "constant",
};

export function collectApiItems(crate: Crate, crateName: string): ApiItem[] {
  const root = crate.index[String(crate.root)];
  if (!root) return [];

  const moduleData = (root.inner as { module?: ModuleItem }).module;
  if (!moduleData) return [];

  const items: ApiItem[] = [];
  const visited = new Set<number>();

  collectFromModule(crate, moduleData, crateName, items, visited);

  return items;
}

function collectFromModule(
  crate: Crate,
  moduleData: ModuleItem,
  crateName: string,
  items: ApiItem[],
  visited: Set<number>,
): void {
  for (const childId of moduleData.items) {
    if (visited.has(childId)) continue;
    visited.add(childId);

    const child = crate.index[String(childId)];
    if (!child) continue;

    const innerKind = Object.keys(child.inner)[0]!;

    if (innerKind === "module") {
      // Recurse into public submodules
      const subModule = (child.inner as { module: ModuleItem }).module;
      collectFromModule(crate, subModule, crateName, items, visited);
    } else if (innerKind === "use") {
      // Re-export: follow to the actual definition
      const useData = (child.inner as { use: UseItem }).use;
      if (useData.id == null) continue;
      if (visited.has(useData.id)) continue;
      visited.add(useData.id);

      const target = crate.index[String(useData.id)];
      if (!target) continue;

      const targetKind = Object.keys(target.inner)[0]!;
      const category = INNER_KIND_TO_CATEGORY[targetKind];
      if (!category) continue;

      items.push({
        item: target,
        displayName: useData.name,
        category,
        crateName,
      });
    } else {
      const category = INNER_KIND_TO_CATEGORY[innerKind];
      if (!category) continue;

      items.push({
        item: child,
        displayName: child.name ?? "unnamed",
        category,
        crateName,
      });
    }
  }
}

// ---------------------------------------------------------------------------
// Relative URL computation
// ---------------------------------------------------------------------------

/**
 * Compute a relative URL from a source logical page path to a target logical
 * page path. Both inputs are "logical" paths without leading slash, basePath,
 * or locale prefix (e.g. `"kaname/api/structs/epoch/"`). Trailing slashes on
 * inputs are preserved in the output.
 *
 * Examples:
 * - from `kaname/api/overview/` to `kaname/api/structs/epoch/`
 *   → `../structs/epoch/`
 * - from `kaname/api/structs/eci/` to `tobari/api/structs/nrlmsise00/`
 *   → `../../../tobari/api/structs/nrlmsise00/`
 *
 * When the source path is empty (no current page set), falls back to a
 * root-relative path `/${toPath}` — useful for unit tests that bypass page
 * generation.
 */
export function computeRelativeUrl(fromPath: string, toPath: string): string {
  if (!fromPath) {
    return `/${toPath}`;
  }
  const fromParts = fromPath.replace(/\/$/, "").split("/").filter(Boolean);
  const toParts = toPath.replace(/\/$/, "").split("/").filter(Boolean);

  let common = 0;
  while (
    common < fromParts.length &&
    common < toParts.length &&
    fromParts[common] === toParts[common]
  ) {
    common++;
  }

  // Current page URL ends with `/` (directory-like), so the base for
  // resolving relative links is the current page itself. We need one `..`
  // for each remaining segment of fromParts after the common prefix.
  const up = fromParts.length - common;
  const down = toParts.slice(common);
  const prefix = "../".repeat(up);
  const suffix = down.join("/");
  const trailing = toPath.endsWith("/") ? "/" : "";
  const rel = `${prefix}${suffix}${trailing}`;
  return rel || "./";
}

// ---------------------------------------------------------------------------
// collectImpls — collect inherent impls for a type
// ---------------------------------------------------------------------------

export interface CollectedImpl {
  implItem: ImplItem;
  methods: Item[];
}

export function collectInherentImpls(crate: Crate, implIds: Id[]): CollectedImpl[] {
  const result: CollectedImpl[] = [];

  for (const implId of implIds) {
    const implEntry = crate.index[String(implId)];
    if (!implEntry) continue;

    const implData = (implEntry.inner as { impl?: ImplItem }).impl;
    if (!implData) continue;

    // Skip trait impls (only collect inherent impls)
    if (implData.trait != null) continue;

    const methods: Item[] = [];
    for (const methodId of implData.items) {
      const method = crate.index[String(methodId)];
      if (method) methods.push(method);
    }

    result.push({ implItem: implData, methods });
  }

  return result;
}

// Auto-traits and compiler-internal traits that clutter the listing
const AUTO_TRAITS = new Set([
  "Send",
  "Sync",
  "Unpin",
  "Freeze",
  "UnsafeUnpin",
  "UnwindSafe",
  "RefUnwindSafe",
  "StructuralPartialEq",
]);

export interface TraitImplInfo {
  traitName: string;
  traitId: Id;
  fullPath: string[];
  crateId: number;
}

/** Collect trait impls, separated into user-facing and auto-traits */
export function collectTraitImpls(
  crate: Crate,
  implIds: Id[],
): { userTraits: TraitImplInfo[]; autoTraits: TraitImplInfo[] } {
  const userTraits: TraitImplInfo[] = [];
  const autoTraits: TraitImplInfo[] = [];

  for (const implId of implIds) {
    const implEntry = crate.index[String(implId)];
    if (!implEntry) continue;

    const implData = (implEntry.inner as { impl?: ImplItem }).impl;
    if (!implData || !implData.trait) continue;

    // Skip blanket impls
    if (implData.blanket_impl) continue;

    const traitName = implData.trait.path;
    const traitId = implData.trait.id;

    // Look up full path from paths table for external URL construction
    const summary = crate.paths[String(traitId)];
    const fullPath = summary?.path ?? [traitName];
    const crateId = summary?.crate_id ?? 0;

    const info: TraitImplInfo = { traitName, traitId, fullPath, crateId };

    if (AUTO_TRAITS.has(traitName)) {
      autoTraits.push(info);
    } else {
      userTraits.push(info);
    }
  }

  return { userTraits, autoTraits };
}

/** Resolve a trait impl to a URL using the crate's own external_crates table */
export function resolveTraitImplUrl(
  info: TraitImplInfo,
  crate: Crate,
  resolver: LinkResolver,
): string | undefined {
  // Try resolver first (works for local traits)
  const resolved = resolver.resolveId(info.traitId, crate);
  if (resolved) return resolved;

  // For std/core auto-traits, build the URL from the path
  const ext = crate.external_crates[String(info.crateId)];
  if (ext) {
    const crateName = ext.name;
    const itemName = info.fullPath[info.fullPath.length - 1] ?? info.traitName;
    if (["std", "core", "alloc"].includes(crateName)) {
      const innerPath = info.fullPath.slice(1, -1).join("/");
      const modPart = innerPath ? `${innerPath}/` : "";
      return `https://doc.rust-lang.org/std/${modPart}trait.${itemName}.html`;
    }
    const innerPath = info.fullPath.slice(1, -1).join("/");
    const modPart = innerPath ? `${innerPath}/` : "";
    return `https://docs.rs/${crateName}/latest/${crateName}/${modPart}trait.${itemName}.html`;
  }

  return undefined;
}

/** Find which types implement a given trait */
export function collectImplementors(crate: Crate, traitImplIds: Id[]): { name: string; id: Id }[] {
  const result: { name: string; id: Id }[] = [];

  for (const implId of traitImplIds) {
    const implEntry = crate.index[String(implId)];
    if (!implEntry) continue;

    const implData = (implEntry.inner as { impl?: ImplItem }).impl;
    if (!implData) continue;
    if (implData.blanket_impl) continue;

    const forType = implData.for;
    const resolved = (forType as { resolved_path?: { path: string; id: Id } }).resolved_path;
    if (resolved) {
      // Strip crate:: prefix from implementor names
      const name = resolved.path.replace(/^crate::(?:.*::)?/, "");
      result.push({ name, id: resolved.id });
    }
  }

  return result;
}
