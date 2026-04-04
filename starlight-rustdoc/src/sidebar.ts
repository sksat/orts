import type { ApiItemCategory } from "./resolve.js";
import type { GeneratedPage } from "./markdown.js";

// Starlight sidebar types (simplified from @astrojs/starlight)
export interface SidebarLink {
  label: string;
  slug?: string;
  link?: string;
  attrs?: Record<string, string>;
}

export interface SidebarGroup {
  label: string;
  collapsed?: boolean;
  items: (SidebarLink | SidebarGroup | SidebarAutogenerate)[];
}

export interface SidebarAutogenerate {
  label: string;
  collapsed?: boolean;
  autogenerate: { directory: string };
}

export type SidebarItem = SidebarLink | SidebarGroup | SidebarAutogenerate;

const CATEGORY_ORDER: ApiItemCategory[] = [
  "trait",
  "struct",
  "enum",
  "function",
  "type_alias",
  "constant",
];

const CATEGORY_LABELS: Record<ApiItemCategory, string> = {
  trait: "Traits",
  struct: "Structs",
  enum: "Enums",
  function: "Functions",
  type_alias: "Type Aliases",
  constant: "Constants",
};

const CATEGORY_DIRS: Record<ApiItemCategory, string> = {
  trait: "traits",
  struct: "structs",
  enum: "enums",
  function: "functions",
  type_alias: "type-aliases",
  constant: "constants",
};

export function buildCrateSidebar(
  crateName: string,
  pages: GeneratedPage[],
  options: { collapsed?: boolean },
): SidebarGroup {
  // Determine which categories have pages
  const categoriesWithPages = new Set(
    pages.filter((p) => p.name !== "overview").map((p) => p.category),
  );

  const apiItems: SidebarItem[] = [
    { label: "Overview", slug: `${crateName}/api/overview` },
  ];

  for (const category of CATEGORY_ORDER) {
    if (!categoriesWithPages.has(category)) continue;
    apiItems.push({
      label: CATEGORY_LABELS[category],
      autogenerate: { directory: `${crateName}/api/${CATEGORY_DIRS[category]}` },
    });
  }

  return {
    label: crateName,
    collapsed: options.collapsed ?? true,
    items: [
      {
        label: "API Reference",
        collapsed: true,
        items: apiItems,
      },
    ],
  };
}
