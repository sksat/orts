import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

/**
 * Library build for the embeddable `./lib` entry (`@OrbitViewer` + primitives).
 *
 * Separate from the app build (vite.config.ts → dist/): this emits to dist-lib/
 * so the two never clash. The repo and the app keep consuming the raw TS via the
 * top-level `exports["./lib"]` (source ergonomics); this build is what a future
 * publish ships, wired through `publishConfig.exports` in package.json.
 *
 * react / react-dom / three / @react-three/* are externalised so a consumer
 * supplies its single copy (they're peerDependencies) — bundling them would
 * cause duplicate-React / "multiple instances of three" failures.
 */

/** Packages the consumer provides (peerDependencies); never bundled. */
const PEERS = [
  "react",
  "react-dom",
  "three",
  "@react-three/fiber",
  "@react-three/drei",
];

const isPeer = (id: string) => PEERS.some((p) => id === p || id.startsWith(`${p}/`));

export default defineConfig({
  plugins: [react()],
  build: {
    outDir: "dist-lib",
    emptyOutDir: true,
    sourcemap: true,
    // public/ holds the app's static textures (28MB of earth_*.jpg). Those are
    // app/server assets, not part of the library — a consumer supplies textures
    // via `textureBaseUrl`. Don't copy them into the package.
    copyPublicDir: false,
    // Library mode without a forced single bundle: preserveModules (below) keeps
    // the source tree so it stays tree-shakeable and the tsc-emitted .d.ts tree
    // lines up file-for-file.
    lib: {
      entry: "src/lib/index.ts",
      formats: ["es"],
    },
    rollupOptions: {
      external: isPeer,
      output: {
        preserveModules: true,
        preserveModulesRoot: "src",
        entryFileNames: "[name].js",
      },
    },
  },
});
