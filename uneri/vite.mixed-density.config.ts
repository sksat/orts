import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [react()],
  root: "examples/mixed-density",
  optimizeDeps: {
    exclude: ["@duckdb/duckdb-wasm"],
  },
});
