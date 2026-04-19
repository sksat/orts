import { execSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { resolve } from "node:path";

const docsRoot = resolve(import.meta.dirname, "..");
const repoRoot = resolve(docsRoot, "..");

// Embed viewer (skip if not built yet — dev workflow may not need it)
const viewerDist = resolve(repoRoot, "viewer/dist");
const viewerDest = resolve(docsRoot, "public/viewer");
if (existsSync(viewerDist)) {
  rmSync(viewerDest, { recursive: true, force: true });
  cpSync(viewerDist, viewerDest, { recursive: true });
  console.log("Embedded viewer/dist into docs/public/viewer/");
} else if (process.env.ALLOW_MISSING_VIEWER_DIST) {
  console.log("Skipped viewer embed (viewer/dist not found, allowed by ALLOW_MISSING_VIEWER_DIST)");
} else {
  console.error("Error: viewer/dist not found. Run 'pnpm --filter orts-viewer build' first.");
  console.error("Set ALLOW_MISSING_VIEWER_DIST=1 to skip this check for docs-only development.");
  process.exit(1);
}

// Build and embed uneri examples
const examplesDir = resolve(repoRoot, "uneri/examples");
const examplesDest = resolve(docsRoot, "public/uneri/examples");
rmSync(examplesDest, { recursive: true, force: true });

const examples = readdirSync(examplesDir, { withFileTypes: true })
  .filter((d) => d.isDirectory())
  .map((d) => d.name);

for (const name of examples) {
  const exampleRoot = resolve(examplesDir, name);
  const outDir = resolve(examplesDest, name, "demo");
  const basePath = `/orts/uneri/examples/${name}/demo/`;
  console.log(`Building example: ${name}...`);
  execSync(`npx vite build --base ${basePath} --outDir ${outDir}`, {
    cwd: exampleRoot,
    stdio: "inherit",
    env: { ...process.env },
  });
}
console.log(`Embedded ${examples.length} examples into docs/public/uneri/examples/`);

// Build and embed tobari Earth environment visualizer
const tobariWebRoot = resolve(repoRoot, "tobari/examples/web");
const tobariDest = resolve(docsRoot, "public/tobari/examples/earth-visualizer/demo");
const tobariBase = "/orts/tobari/examples/earth-visualizer/demo/";

// Download space weather data for bundling
const swDest = resolve(tobariWebRoot, "public/space-weather.txt");
console.log("Downloading space weather data...");
try {
  const res = await fetch("https://celestrak.org/SpaceData/SW-Last5Years.txt");
  if (res.ok) {
    mkdirSync(resolve(tobariWebRoot, "public"), { recursive: true });
    writeFileSync(swDest, await res.text());
    console.log("Downloaded space weather data.");
  }
} catch {
  console.log("Warning: could not download space weather data, Real SW mode will be unavailable.");
}

try {
  // Always rebuild WASM to avoid shipping stale artifacts
  console.log("Building tobari WASM...");
  execSync("pnpm --filter tobari-example-web build:wasm:all", {
    cwd: repoRoot,
    stdio: "inherit",
  });
  console.log("Building tobari-example-web...");
  // Build to a temp dir first, then swap — so a failed build doesn't delete an existing demo
  const tobariTmp = resolve(`${tobariDest}.tmp`);
  rmSync(tobariTmp, { recursive: true, force: true });
  execSync(`npx vite build --base ${tobariBase} --outDir ${tobariTmp}`, {
    cwd: tobariWebRoot,
    stdio: "inherit",
    env: { ...process.env },
  });
  rmSync(tobariDest, { recursive: true, force: true });
  cpSync(tobariTmp, tobariDest, { recursive: true });
  rmSync(tobariTmp, { recursive: true, force: true });
  console.log(
    "Embedded tobari-example-web into docs/public/tobari/examples/earth-visualizer/demo/",
  );
} catch {
  if (process.env.ALLOW_MISSING_TOBARI) {
    console.log("Skipped tobari-example-web embed (build failed, allowed by ALLOW_MISSING_TOBARI)");
  } else {
    console.error(
      "Error: tobari-example-web build failed. Ensure Rust and wasm-pack are installed.",
    );
    console.error("Set ALLOW_MISSING_TOBARI=1 to skip this for docs-only development.");
    process.exit(1);
  }
}

// ---------------------------------------------------------------------------
// Auto-discover example READMEs and copy them into docs content as .mdx pages.
// Any `README.md` under `orts/examples/*/` or `plugin-sdk/examples/*/` is
// picked up. Metadata (title/description/locale) comes from an optional YAML
// frontmatter block at the top of the README; otherwise title falls back to
// the first `# heading`, description to the first paragraph after it, and
// locale defaults to `ja`.
// ---------------------------------------------------------------------------

/**
 * @param {string} raw README contents
 * @returns {{ front: Record<string, string>, body: string }}
 */
function parseFrontmatter(raw) {
  if (!raw.startsWith("---\n")) return { front: {}, body: raw };
  const end = raw.indexOf("\n---\n", 4);
  if (end === -1) return { front: {}, body: raw };
  const block = raw.slice(4, end);
  const body = raw.slice(end + 5).replace(/^\n+/, "");
  /** @type {Record<string, string>} */
  const front = {};
  for (const line of block.split("\n")) {
    const m = line.match(/^(\w+):\s*(.*)$/);
    if (!m) continue;
    let value = m[2].trim();
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }
    front[m[1]] = value;
  }
  return { front, body };
}

/**
 * @param {string} body README body (frontmatter already stripped)
 * @returns {{ title: string | null, description: string | null }}
 */
function extractTitleAndDescription(body) {
  const lines = body.split("\n");
  let title = null;
  let i = 0;
  for (; i < lines.length; i++) {
    const m = lines[i].match(/^#\s+(.+?)\s*$/);
    if (m) {
      title = m[1];
      i++;
      break;
    }
  }
  // Join consecutive non-empty lines into a single description paragraph.
  while (i < lines.length && !lines[i].trim()) i++;
  const paraLines = [];
  for (; i < lines.length; i++) {
    const line = lines[i];
    if (!line.trim()) break;
    if (/^#{1,6}\s/.test(line)) break;
    paraLines.push(line.trim());
  }
  const description = paraLines.length ? paraLines.join(" ").replace(/\s+/g, " ") : null;
  return { title, description };
}

/** @type {Array<{slug: string, readme: string, locale: string, title: string, description: string}>} */
const examplePages = [];
const exampleRoots = [
  { dir: "orts/examples", slugPrefix: "examples" },
  { dir: "plugin-sdk/examples", slugPrefix: "examples/plugins" },
];
for (const { dir, slugPrefix } of exampleRoots) {
  const absDir = resolve(repoRoot, dir);
  if (!existsSync(absDir)) continue;
  const entries = readdirSync(absDir, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => d.name)
    .sort();
  for (const name of entries) {
    const readmeRel = `${dir}/${name}/README.md`;
    const readmeAbs = resolve(repoRoot, readmeRel);
    if (!existsSync(readmeAbs)) continue;
    const raw = readFileSync(readmeAbs, "utf-8");
    const { front, body } = parseFrontmatter(raw);
    const { title: h1Title, description: firstPara } = extractTitleAndDescription(body);
    const title = front.title ?? h1Title;
    if (!title) {
      console.log(`Warning: ${readmeRel} has no title (frontmatter or H1), skipping`);
      continue;
    }
    const description = front.description ?? firstPara ?? "";
    const locale = front.locale ?? "ja";
    const slugName = name.replace(/_/g, "-");
    examplePages.push({
      slug: `${slugPrefix}/${slugName}`,
      readme: readmeRel,
      locale,
      title,
      description,
      body,
    });
  }
}

for (const page of examplePages) {
  // Strip the first `# ...` heading — Starlight renders the title from frontmatter.
  let body = page.body.replace(/^#\s+.*\n+/, "");

  // Convert bare GitHub video URLs to <video> tags.
  // GitHub README renders bare video URLs (user-attachments, release assets) as video,
  // but mdx treats it as plain text. Convert to HTML video element.
  body = body.replace(
    /^(https:\/\/github\.com\/(?:user-attachments\/assets\/[a-f0-9-]+|[^\s]+\.mp4))$/gm,
    (_, url) => `<video controls width="100%"><source src="${url}" type="video/mp4" /></video>`,
  );

  // Rewrite relative image paths to GitHub raw URLs.
  // e.g. ![alt](image.png) → ![alt](https://raw.githubusercontent.com/sksat/orts/main/orts/examples/apollo11/image.png)
  const readmeDir = page.readme.replace(/\/[^/]+$/, "");
  body = body.replace(
    /!\[([^\]]*)\]\((?!https?:\/\/)([^)]+)\)/g,
    (_, alt, src) =>
      `![${alt}](https://raw.githubusercontent.com/sksat/orts/main/${readmeDir}/${src})`,
  );

  // Escape bare `<` in prose that MDX would parse as JSX tags.
  // Matches `<` NOT followed by a known HTML/JSX-like pattern (tag name, !, /, etc.)
  // but preceded by a space or start-of-line — e.g. "< 1°" or "<1°".
  // Skip lines inside code fences.
  let inCodeFence = false;
  body = body
    .split("\n")
    .map((line) => {
      if (line.startsWith("```")) inCodeFence = !inCodeFence;
      if (inCodeFence) return line;
      // Replace bare < that looks like a comparison, not an HTML tag
      return line.replace(/(<)(\d)/g, "&lt;$2");
    })
    .join("\n");

  const yamlEscape = (s) => s.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  const frontmatter = [
    "---",
    `title: "${yamlEscape(page.title)}"`,
    `description: "${yamlEscape(page.description)}"`,
    "---",
    "",
  ].join("\n");

  // Always write to en/ (default locale) so the page appears in all sidebars.
  // Starlight's i18n fallback shows a "not translated" warning when a non-default
  // locale falls back to the default locale content.
  const locales = page.locale === "en" ? ["en"] : ["en", page.locale];
  for (const locale of locales) {
    const contentBase = resolve(docsRoot, `src/content/docs/${locale}`);
    const dest = resolve(contentBase, `${page.slug}.mdx`);
    mkdirSync(resolve(dest, ".."), { recursive: true });
    writeFileSync(dest, frontmatter + body);
  }
  console.log(`Copied ${page.readme} → ${page.slug}.mdx (${locales.join(" + ")})`);
}
