/**
 * Astro integration that rewrites absolute URLs in starlight-typedoc output
 * to relative URLs.
 *
 * ## Why
 *
 * `starlight-typedoc` (which wraps `typedoc-plugin-markdown`) generates
 * Markdown files containing hard-coded absolute links such as
 * `[ChartBuffer](/orts/en/uneri/api/classes/chartbuffer/)`. When Starlight
 * later serves the same content as a Japanese fallback at
 * `/orts/ja/uneri/api/...`, those absolute links still point back into
 * `/orts/en/...` and silently drop the visitor out of the Japanese locale —
 * exactly the bug we fixed for `starlight-rustdoc` by switching to relative
 * links.
 *
 * Because the third-party `starlight-typedoc` plugin does not expose a
 * locale-agnostic output mode, this integration post-processes its output
 * files on disk. It walks every generated `.md` file and rewrites inline
 * Markdown link targets of the form `/{base}/{locale}/{typedocRoot}/...` into
 * **relative** URLs computed from the current file's own URL path. Relative
 * URLs are automatically rebased by the browser onto whatever locale URL the
 * page is being served at, so Japanese fallback pages stay in `/orts/ja/`.
 *
 * ## When it runs
 *
 * The `astro:config:setup` hook runs during Astro's config phase, in the
 * order integrations appear in `astro.config.mjs`. Registering this
 * integration **after** `starlight()` guarantees that `starlight-typedoc`'s
 * own `config:setup` (invoked via Starlight's plugin system) has already
 * written the Markdown files to disk before this hook walks them.
 *
 * This approach is idempotent: re-running over already-relative links is a
 * no-op because the rewrite pattern only matches absolute URLs.
 *
 * Unit tests in `./rewrite-typedoc-links.test.ts` cover the pure helpers
 * (`computeRelativeUrl`, `rewriteAbsoluteTypedocLinks`) without touching the
 * filesystem.
 */

import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { join, posix, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

export interface RewriteTypedocLinksOptions {
  /**
   * Absolute path to Astro content directory, e.g.
   * `/path/to/docs/src/content/docs`. Usually derived from the Astro config
   * in the hook (`new URL("./src/content/docs/", astroConfig.root)`).
   */
  contentDir: string;
  /** Locale subdirectory containing the typedoc output, e.g. "en". */
  locale: string;
  /**
   * Path to the typedoc output root relative to `${contentDir}/${locale}`.
   * For `starlightTypeDoc({ output: "en/uneri/api" })` this is "uneri/api".
   */
  typedocRoot: string;
  /** Astro base path without trailing slash, e.g. "/orts". */
  base: string;
}

export interface AstroIntegration {
  name: string;
  hooks: {
    "astro:config:setup"?: (context: {
      config: { root: URL | string };
      logger: { info: (message: string) => void; warn: (message: string) => void };
    }) => void | Promise<void>;
  };
}

export default function rewriteTypedocLinks(
  options: Omit<RewriteTypedocLinksOptions, "contentDir"> & { contentDir?: string },
): AstroIntegration {
  return {
    name: "rewrite-typedoc-links",
    hooks: {
      "astro:config:setup": ({ config, logger }) => {
        const astroRoot =
          config.root instanceof URL ? fileURLToPath(config.root) : String(config.root);
        const contentDir = options.contentDir ?? join(astroRoot, "src", "content", "docs");
        const localeDir = join(contentDir, options.locale);
        const typedocAbsDir = join(localeDir, options.typedocRoot);
        if (!existsSync(typedocAbsDir)) {
          logger.warn(`[rewrite-typedoc-links] Skipping: ${typedocAbsDir} does not exist yet.`);
          return;
        }
        const fullOptions: RewriteTypedocLinksOptions = {
          ...options,
          contentDir,
        };
        const rewritten = walkAndRewrite(typedocAbsDir, fullOptions);
        logger.info(
          `[rewrite-typedoc-links] Rewrote ${rewritten} file(s) under ${relative(astroRoot, typedocAbsDir)}`,
        );
      },
    },
  };
}

function walkAndRewrite(dirAbs: string, options: RewriteTypedocLinksOptions): number {
  let count = 0;
  for (const entry of readdirSync(dirAbs)) {
    const entryAbs = join(dirAbs, entry);
    if (statSync(entryAbs).isDirectory()) {
      count += walkAndRewrite(entryAbs, options);
      continue;
    }
    if (!entry.endsWith(".md")) continue;

    const before = readFileSync(entryAbs, "utf-8");
    const after = rewriteAbsoluteTypedocLinks(before, {
      filePath: entryAbs,
      ...options,
    });
    if (before !== after) {
      writeFileSync(entryAbs, after, "utf-8");
      count++;
    }
  }
  return count;
}

/**
 * Derive a file's logical URL path (relative to `${contentDir}/${locale}/`,
 * stripping the `.md` suffix, normalising the basename) from its absolute
 * filesystem path. Exported for unit tests.
 *
 * Examples (with `contentDir = ".../docs"`, `locale = "en"`):
 *
 * - `.../docs/en/uneri/api/README.md` → `"uneri/api/readme/"`
 * - `.../docs/en/uneri/api/classes/ChartBuffer.md` → `"uneri/api/classes/chartbuffer/"`
 *
 * typedoc-plugin-markdown writes files with PascalCase / camelCase basenames
 * but links to them using the **lowercased** slug produced by Starlight's
 * content loader, so we must lowercase the stem to match.
 */
export function filePathToLogicalUrl(
  filePath: string,
  options: Pick<RewriteTypedocLinksOptions, "contentDir" | "locale">,
): string {
  const localeAbs = join(options.contentDir, options.locale);
  const rel = relative(localeAbs, filePath).split(sep).join(posix.sep);
  const withoutExt = rel.replace(/\.md$/, "");
  // Lowercase only the final basename so that directory names (which are
  // already lowercase in typedoc output) are preserved even if they happen
  // to contain unusual characters.
  const parts = withoutExt.split("/");
  const last = parts.pop() ?? "";
  const slug = last.toLowerCase();
  const logical = [...parts, slug].join("/");
  return `${logical}/`;
}

/**
 * Rewrite inline Markdown link targets that point to
 * `/${base}/${locale}/${typedocRoot}/...` into relative URLs anchored at the
 * current file's logical URL.
 *
 * The transformation is stable under repeated invocation because it only
 * matches absolute URLs; relative URLs produced by a previous run are
 * ignored.
 */
export function rewriteAbsoluteTypedocLinks(
  content: string,
  options: RewriteTypedocLinksOptions & { filePath: string },
): string {
  const fromLogical = filePathToLogicalUrl(options.filePath, options);
  const baseNoTrail = options.base.replace(/\/$/, "");
  const absolutePrefix = `${baseNoTrail}/${options.locale}/`;
  // Only rewrite links that actually fall inside the typedoc tree we own.
  // Anything else (e.g. cross-links into user-written pages) is intentionally
  // left alone.
  const typedocRootRel = options.typedocRoot.replace(/\/$/, "");
  // Match `](/orts/en/uneri/api/...)` occurrences inside inline Markdown
  // links. Captures the path after `/${locale}/` so we can re-root it.
  const pattern = new RegExp(
    `\\]\\(${escapeRegExp(absolutePrefix)}${escapeRegExp(typedocRootRel)}/([^)\\s]*)\\)`,
    "g",
  );
  return content.replace(pattern, (_match, subpath: string) => {
    const targetLogical = `${typedocRootRel}/${subpath}`;
    const rel = computeRelativeUrl(fromLogical, targetLogical);
    return `](${rel})`;
  });
}

/**
 * Relative URL computation between two "logical" page paths that have no
 * base or locale segment. Mirrors the helper of the same name in
 * `starlight-rustdoc/src/resolve.ts`.
 *
 * Both inputs are directory-style paths like `"uneri/api/classes/foo/"`. The
 * output is a relative URL that navigates from `fromPath` to `toPath`.
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

function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
