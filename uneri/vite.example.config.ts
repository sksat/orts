import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  root: "examples/sine-wave",
  optimizeDeps: {
    exclude: ["@duckdb/duckdb-wasm"],
  },
});
