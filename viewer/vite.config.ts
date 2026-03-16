import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  root: ".",
  base: process.env.VITE_BASE_PATH ?? "/",
  plugins: [react()],
  build: {
    outDir: "dist",
  },
  optimizeDeps: {
    exclude: ["@duckdb/duckdb-wasm"],
  },
});
