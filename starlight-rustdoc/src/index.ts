import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { CrateSpec } from "./exec.js";
import { generateRustdocJson, normalizeCrateSpec } from "./exec.js";
import { LinkResolver, collectApiItems } from "./resolve.js";
import { generateCratePages } from "./markdown.js";
import { buildCrateSidebar } from "./sidebar.js";
import type { Crate } from "./types.js";

/**
 * Minimal Starlight plugin type — avoids importing from @astrojs/starlight
 * which has virtual module dependencies that break standalone tsc.
 * This is structurally compatible with StarlightPlugin from @astrojs/starlight/types.
 */
interface StarlightPlugin {
  name: string;
  hooks: {
    "config:setup"?: (context: {
      config: Record<string, unknown>;
      updateConfig: (update: Record<string, unknown>) => void;
      addIntegration?: (integration: unknown) => void;
      astroConfig: { root: URL | string; base?: string };
      command: string;
      isRestart: boolean;
      logger: { info: (msg: string) => void; warn: (msg: string) => void; error: (msg: string) => void };
    }) => void | Promise<void>;
  };
}

export interface StarlightRustdocOptions {
  /** Crate names or specs to document */
  crates: (string | CrateSpec)[];
  /** Path to Cargo workspace root, relative to Astro root (default: "..") */
  workspace?: string;
  /** Rust toolchain for rustdoc JSON (default: "nightly") */
  toolchain?: string;
  /** Output directory pattern per crate (default: "{crate}/api") */
  output?: (crateName: string) => string;
  /** Source link configuration */
  sourceLinks?: {
    repository: string;
    branch?: string;
  };
  /** Sidebar options */
  sidebar?: {
    collapsed?: boolean;
  };
}

export default function starlightRustdoc(options: StarlightRustdocOptions): StarlightPlugin {
  return {
    name: "starlight-rustdoc",
    hooks: {
      "config:setup": async ({ astroConfig, config, updateConfig, logger, command }) => {
        if (command === "preview") return;

        // Allow skipping via environment variable
        if (process.env.STARLIGHT_RUSTDOC_SKIP) {
          logger.info("Skipped rustdoc generation (STARLIGHT_RUSTDOC_SKIP is set)");
          return;
        }

        const astroRoot =
          astroConfig.root instanceof URL
            ? fileURLToPath(astroConfig.root)
            : String(astroConfig.root);
        const workspace = resolve(astroRoot, options.workspace ?? "..");
        const contentDir = resolve(astroRoot, "src", "content", "docs");
        const basePath = (astroConfig.base ?? "/").replace(/\/$/, "");

        const crateSpecs = options.crates.map(normalizeCrateSpec);

        // Pass 1: Generate JSON for all crates
        const crateJsons = new Map<string, Crate>();
        for (const spec of crateSpecs) {
          logger.info(`Generating rustdoc JSON for ${spec.name}...`);
          try {
            const json = generateRustdocJson(spec, {
              workspace,
              toolchain: options.toolchain,
            });
            crateJsons.set(spec.name, json);
            logger.info(`  format_version=${json.format_version}, items=${Object.keys(json.index).length}`);
          } catch (e) {
            logger.error(`Failed to generate rustdoc JSON for ${spec.name}: ${e}`);
            throw e;
          }
        }

        // Initialize link resolver with all crates
        const resolver = new LinkResolver(crateJsons, basePath);

        // Collect API items for all crates and register pages (Pass 1 of link resolution)
        const allItems = new Map<string, ReturnType<typeof collectApiItems>>();
        for (const [crateName, crateJson] of crateJsons) {
          const items = collectApiItems(crateJson, crateName);
          allItems.set(crateName, items);

          // Register pages in resolver for cross-crate linking
          for (const item of items) {
            const slug = item.displayName.toLowerCase();
            const categoryDir =
              item.category === "type_alias" ? "type-aliases" : `${item.category}s`;
            const pagePath = `${basePath}/${crateName}/api/${categoryDir}/${slug}/`;
            resolver.registerPage(item.item.id, pagePath, item.displayName, crateName);
          }
        }

        // Pass 2: Generate Markdown pages
        const sidebarItems: ReturnType<typeof buildCrateSidebar>[] = [];

        for (const [crateName, crateJson] of crateJsons) {
          const items = allItems.get(crateName)!;
          logger.info(`Generating ${items.length} pages for ${crateName}...`);

          const pages = generateCratePages(crateName, items, crateJson, resolver, {
            contentDir,
            basePath,
            sourceLinks: options.sourceLinks
              ? {
                  repository: options.sourceLinks.repository,
                  branch: options.sourceLinks.branch ?? "main",
                }
              : undefined,
          });

          sidebarItems.push(
            buildCrateSidebar(crateName, pages, {
              collapsed: options.sidebar?.collapsed,
            }),
          );
        }

        // Update Starlight sidebar
        const existingSidebar = (config.sidebar ?? []) as unknown[];
        updateConfig({
          sidebar: [...existingSidebar, ...sidebarItems],
        });

        logger.info("Rustdoc generation complete.");
      },
    },
  };
}

export type { CrateSpec } from "./exec.js";
