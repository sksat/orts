import { execSync } from "node:child_process";
import { cpSync, existsSync, readdirSync, rmSync } from "node:fs";
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
  execSync("pnpm --filter tobari-web build:wasm:all", {
    cwd: repoRoot,
    stdio: "inherit",
  });
  console.log("Building tobari-web...");
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
  console.log("Embedded tobari-web into docs/public/tobari/examples/earth-visualizer/demo/");
} catch {
  if (process.env.ALLOW_MISSING_TOBARI) {
    console.log("Skipped tobari-web embed (build failed, allowed by ALLOW_MISSING_TOBARI)");
  } else {
    console.error("Error: tobari-web build failed. Ensure Rust and wasm-pack are installed.");
    console.error("Set ALLOW_MISSING_TOBARI=1 to skip this for docs-only development.");
    process.exit(1);
  }
}
