import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  root: "examples/mixed-density",
  optimizeDeps: {
    exclude: ["@duckdb/duckdb-wasm"],
  },
});
