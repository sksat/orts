import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  root: "examples/multi-series",
  optimizeDeps: {
    exclude: ["@duckdb/duckdb-wasm"],
  },
});
