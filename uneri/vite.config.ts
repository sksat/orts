import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  // The chart worker is emitted as a file of its own. With the default base
  // ("/") the library refers to it as `/assets/…`, a path on whatever site
  // hosts the consumer, where it does not exist. Relative to the library's own
  // `import.meta.url` it is found next to `index.js`, and a consumer's bundler
  // copies it into its build.
  base: "./",
  build: {
    lib: {
      entry: "src/index.ts",
      formats: ["es"],
      fileName: "index",
    },
    rolldownOptions: {
      external: ["react", "react-dom", "react/jsx-runtime"],
    },
  },
  optimizeDeps: {
    exclude: ["@duckdb/duckdb-wasm"],
  },
});
