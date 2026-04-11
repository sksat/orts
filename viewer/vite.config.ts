import path from "node:path";
import react from "@vitejs/plugin-react";
import license from "rollup-plugin-license";
import { defineConfig } from "vite";

export default defineConfig({
  root: ".",
  base: process.env.VITE_BASE_PATH ?? "/",
  plugins: [
    react(),
    // Emit a NOTICE file covering every npm dependency that actually ends up
    // in the viewer bundle. The CLI (`cli/build.rs` + `rust-embed`) picks it
    // up from `viewer/dist/` and serves it via `orts --license-notice` so the
    // distributed binary carries a complete third-party license notice for
    // both Rust and JS dependencies.
    license({
      thirdParty: {
        output: {
          file: path.resolve(__dirname, "dist/third-party-licenses.txt"),
          template(dependencies) {
            const header =
              "Third-party JavaScript dependencies bundled into the orts " +
              "viewer. Licenses follow.\n\n";
            const body = dependencies
              .map((dep) => {
                const name = `${dep.name}@${dep.version}`;
                const author = dep.author ? ` — ${dep.author.text()}` : "";
                const spdx = dep.license ?? "UNKNOWN";
                const repo = dep.repository?.url ? `\n  ${dep.repository.url}` : "";
                const text = dep.licenseText ? `\n\n${dep.licenseText}` : "";
                return `${name}${author} (${spdx})${repo}${text}\n` + "-".repeat(74);
              })
              .join("\n\n");
            return header + body + "\n";
          },
        },
      },
    }),
  ],
  build: {
    outDir: "dist",
  },
  optimizeDeps: {
    exclude: ["@duckdb/duckdb-wasm"],
  },
});
