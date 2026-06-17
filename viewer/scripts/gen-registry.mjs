// Generate viewer/registry.json from the actual import closure of the public
// library (src/lib/index.ts). The orbit-viewer registry item ships exactly the
// files the public API transitively needs — no more, no less — so the
// shadcn-distributed source stays in sync with what the library imports.
//
// Run from the viewer package root: `node scripts/gen-registry.mjs`
// (wired as the `registry:gen` script). CI re-runs it and diffs registry.json
// to catch drift between the closure and the committed manifest.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, relative, resolve, sep } from "node:path";

const SRC = resolve("src");
const ENTRY = resolve(SRC, "lib/index.ts");

// Runtime npm packages the copied source needs. arika-wasm is intentionally
// omitted: it isn't published to npm yet, so listing it would make `shadcn add`
// try (and fail) to install it. Consumers provide it separately until then —
// see the item `docs` below.
const DEPENDENCIES = [
  "react@^19.0.0",
  "react-dom@^19.0.0",
  // three uses 0.x (minor-as-major) versioning, and the *copied* source compiles
  // against the consumer's three — unlike the type-erased compiled library whose
  // peer floor can be looser. Floor at the tested version, open upper (not `^`,
  // which on 0.x would pin to 0.183.x and block newer three).
  "three@>=0.183.0",
  "@react-three/fiber@^9.0.0",
  "@react-three/drei@^10.0.0",
];

// Capture import specifiers: side-effect `import "x"`, any `... from "x"`
// (import/export), and dynamic `import("x")`. Kept as three separate branches so
// a leading side-effect import isn't swallowed by a later `from` on another line.
const IMPORT_RE =
  /(?:^|[;\n])\s*import\s+["']([^"']+)["']|from\s*["']([^"']+)["']|import\(\s*["']([^"']+)["']\)/gm;

function resolveSpec(fromFile, spec) {
  if (!spec.startsWith(".")) return null; // bare package — external, not copied
  const base = resolve(dirname(fromFile), spec);
  // Resolve the TS/TSX file behind a `.js` (or extensionless) specifier. The raw
  // `base` is intentionally excluded: for `import "./foo"` where `foo/` is a
  // directory it would match the dir and make readFileSync throw.
  const cands = [
    base.replace(/\.js$/, ".ts"),
    base.replace(/\.js$/, ".tsx"),
    `${base}.ts`,
    `${base}.tsx`,
    resolve(base, "index.ts"),
    resolve(base, "index.tsx"),
  ];
  for (const c of cands) if (existsSync(c)) return c;
  throw new Error(`Cannot resolve ${spec} from ${relative(SRC, fromFile)}`);
}

function traceClosure() {
  const seen = new Set();
  const queue = [ENTRY];
  while (queue.length) {
    const file = queue.pop();
    if (seen.has(file)) continue;
    seen.add(file);
    const src = readFileSync(file, "utf8");
    for (const m of src.matchAll(IMPORT_RE)) {
      const spec = m[1] ?? m[2] ?? m[3];
      if (!spec) continue;
      const target = resolveSpec(file, spec);
      if (target) {
        if (target !== SRC && !target.startsWith(SRC + sep)) {
          throw new Error(`closure escapes src/: ${target}`);
        }
        queue.push(target);
      }
    }
  }
  return [...seen].map((f) => relative(SRC, f)).sort();
}

const files = traceClosure().map((rel) => ({
  path: `src/${rel}`,
  type: rel.endsWith(".tsx") ? "registry:component" : "registry:lib",
  // Single root in the consumer so the dense relative-import graph stays valid:
  // every file keeps its src-relative path under <components alias>/orbit-viewer.
  target: `@components/orbit-viewer/${rel}`,
}));

const registry = {
  $schema: "https://ui.shadcn.com/schema/registry.json",
  name: "orts-viewer",
  homepage: "https://github.com/sksat/orts",
  items: [
    {
      name: "orbit-viewer",
      type: "registry:block",
      title: "OrbitViewer",
      description:
        "Embeddable React Three Fiber orbit viewer: the OrbitViewer component, its " +
        "primitives, frame/trail logic, body definitions, and arika-wasm bindings.",
      docs:
        "Also requires the `arika-wasm` package (the Rust→wasm coordinate/ephemeris " +
        "engine). It is not yet on npm; install it from the orts workspace until it " +
        "is published.",
      dependencies: DEPENDENCIES,
      files,
    },
  ],
};

writeFileSync("registry.json", `${JSON.stringify(registry, null, 2)}\n`);
console.log(`registry.json: orbit-viewer item with ${files.length} files`);
