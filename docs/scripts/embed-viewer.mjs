import { execSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { basename, resolve } from "node:path";

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
{
  console.log("Downloading space weather data...");
  try {
    const res = await fetch("https://celestrak.org/SpaceData/SW-Last5Years.txt");
    if (res.ok) {
      const { writeFileSync, mkdirSync } = await import("node:fs");
      mkdirSync(resolve(tobariWebRoot, "public"), { recursive: true });
      writeFileSync(swDest, await res.text());
      console.log("Downloaded space weather data.");
    }
  } catch {
    console.log(
      "Warning: could not download space weather data, Real SW mode will be unavailable.",
    );
  }
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
  const tobariTmp = resolve(tobariDest + ".tmp");
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
// Copy example READMEs into docs content as .mdx pages.
// Each entry maps a docs slug to { source README, title, description }.
// The script reads the README, strips the first `# heading` line (replaced
// by Starlight's title), prepends frontmatter, and writes to content dir.
// ---------------------------------------------------------------------------

/** @type {Array<{slug: string, readme: string, title: string, description: string}>} */
const examplePages = [
  {
    slug: "orts/examples/apollo11",
    readme: "orts/examples/apollo11/README.md",
    title: "Apollo 11 Trajectory",
    description: "Apollo 11 全行程の軌道シミュレーションと 3D 可視化",
  },
  {
    slug: "orts/examples/artemis1",
    readme: "orts/examples/artemis1/README.md",
    title: "Artemis 1 Coast Feasibility",
    description: "Artemis 1 coast phase を JPL Horizons と照合する feasibility spike",
  },
  {
    slug: "orts/examples/orbital-lifetime",
    readme: "orts/examples/orbital_lifetime/README.md",
    title: "Orbital Lifetime Analysis",
    description: "6U CubeSat の軌道減衰を再現し、大気モデルと太陽活動の影響を比較",
  },
];

const contentBase = resolve(docsRoot, "src/content/docs/en");

for (const page of examplePages) {
  const src = resolve(repoRoot, page.readme);
  if (!existsSync(src)) {
    console.log(`Warning: ${page.readme} not found, skipping`);
    continue;
  }

  let body = readFileSync(src, "utf-8");

  // Strip the first `# ...` heading — Starlight renders the title from frontmatter.
  body = body.replace(/^#\s+.*\n+/, "");

  // Convert bare GitHub video URLs to <video> tags.
  // GitHub README renders bare `https://github.com/user-attachments/...` as video,
  // but mdx treats it as plain text. Convert to HTML video element.
  body = body.replace(
    /^(https:\/\/github\.com\/user-attachments\/assets\/[a-f0-9-]+)$/gm,
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

  const frontmatter = [
    "---",
    `title: "${page.title}"`,
    `description: "${page.description}"`,
    "---",
    "",
  ].join("\n");

  const dest = resolve(contentBase, `${page.slug}.mdx`);
  mkdirSync(resolve(dest, ".."), { recursive: true });
  writeFileSync(dest, frontmatter + body);
  console.log(`Copied ${page.readme} → ${page.slug}.mdx`);
}
