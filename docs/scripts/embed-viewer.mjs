import { cpSync, existsSync, rmSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { execSync } from "node:child_process";

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
